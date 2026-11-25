use super::*;

const PROJECTED_ASSIGNMENTS: &str = "%%start
struct Record { u8[2] bytes; u8 value; }
Record global = { .bytes = [0, 0]; .value = 0; };
unit(Record, u64) lexical = fn(input, index) { input.bytes[index] = 1; input.value; };
unit(Record) module = fn(input) { global.value = 2; input.value; };
%%end";

fn rename_destination_root(place: &mut ScalarPlace, spelling: &str) {
    match place {
        ScalarPlace::Name { name, .. } => *name = spelling.to_owned(),
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => {
            rename_destination_root(base, spelling);
        }
        ScalarPlace::Dereference { .. } => panic!("named destination root"),
    }
}

#[test]
fn prepared_projected_assignment_roots_are_authoritative_on_both_routes_and_layouts() {
    let source = source();
    let parsed = parse_source(source.clone(), PROJECTED_ASSIGNMENTS.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let module_names = HashSet::from([
        "global".to_owned(),
        "lexical".to_owned(),
        "module".to_owned(),
    ]);
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let stages = stage_observations::Scope::new();
        let standalone = static_calls::PreparedProgram::new(declarations(), target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let project = prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs {
                target_layout: target,
            },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 2,
                ..stage_observations::Counts::default()
            }
        );
        drop(stages);
        let standalone_calls = [
            standalone.function_calls(1).clone(),
            standalone.function_calls(2).clone(),
        ];
        let standalone = standalone.finish();
        assert!(
            standalone.diagnostics.is_empty(),
            "{:?}",
            standalone.diagnostics
        );
        let build = analyze_scalar_build(&project);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());

        for (item, standalone_row) in (1..=2).zip(&standalone_calls) {
            let ScalarItem::Function(standalone_function) = &standalone.program.items[item] else {
                panic!("standalone callable");
            };
            let ScalarItem::Function(project_function) = &project.project().modules[0].items[item]
            else {
                panic!("project callable");
            };
            let baseline = &standalone_function.reference_cfg;
            assert_eq!(build.reference_cfg(0, item, None), Some(baseline));
            let raw = resolve_program_call_sites(&standalone.program).unwrap();
            for (function, row) in [
                (standalone_function, standalone_row),
                (project_function, project.function_calls(0, item)),
            ] {
                let derive = |function: &ScalarFunction, row: &FunctionCallSites| {
                    derive_function_reference_cfg(
                        &source,
                        function,
                        &module_names,
                        row,
                        &standalone.program.structs,
                        &standalone.program.enums,
                        true,
                    )
                };
                assert_eq!(
                    derive(function, raw.function(item)),
                    (baseline.clone(), Vec::new())
                );
                let ordinal = row.assignments[&(row.structure.root, 0)]
                    .destinations()
                    .next()
                    .unwrap()
                    .destination;
                let mut function = function.clone();
                let ScalarBlockItem::Assignment(assignment) = &mut function.body.items[0] else {
                    panic!("canonical assignment");
                };
                let destination = &mut assignment.targets[0];
                let root_span = scalar_place_target_span(reference_root(&destination.place));
                let mut root = ordinal;
                while let Some(OccurrenceNode::Place(base)) = row
                    .structure
                    .edges
                    .get(&(OccurrenceParent::Place(root), OccurrenceEdge::PlaceBase))
                {
                    root = *base;
                }
                assert_ne!(root, ordinal);
                assert_eq!(row.occurrences[root.0].span, root_span);
                let NamedPlaceRootFacts::Prepared(roots) = &row.named_place_roots else {
                    panic!("prepared roots");
                };
                let ScalarItem::Binding(global) = &standalone.program.items[0] else {
                    panic!("module declaration");
                };
                let binding = ReferenceBindingId {
                    source: source.clone(),
                    function_span: function.span,
                    declaration_span: function.parameter_spans[0],
                    block_span: function.body.span,
                    kind: ReferenceBindingKind::Declared,
                };
                let expected_root = if item == 1 {
                    NamedPlaceRootIdentity::Lexical(binding.clone())
                } else {
                    NamedPlaceRootIdentity::ModuleValue(SourceSpan::new(
                        source.clone(),
                        global.name_span,
                    ))
                };
                assert_eq!(roots[&root], expected_root);
                let spelling = if item == 1 { "global" } else { "input" };
                rename_destination_root(&mut destination.place, spelling);
                let destination = destination.clone();

                let mut expected = baseline.clone();
                for points in [&mut expected.points, &mut expected.initial_points] {
                    let mut mutations = 0;
                    for point in points {
                        match &mut point.kind {
                            CfgPointKind::Assign { target, .. } if item == 1 => {
                                assert_eq!(target.binding, binding);
                                rename_destination_root(&mut target.place, spelling);
                                mutations += 1;
                            }
                            CfgPointKind::Evaluate(ScalarExpression::Name { name, span })
                                if item == 2 && *span == root_span =>
                            {
                                assert_eq!(point.source_span.range, destination.target_span);
                                *name = spelling.to_owned();
                                mutations += 1;
                            }
                            _ => {}
                        }
                    }
                    assert_eq!(mutations, 1);
                }
                assert_eq!(derive(&function, row), (expected, Vec::new()));

                if item == 1 {
                    let mut builder = ReferenceFlowBuilder::new(
                        &source,
                        &function,
                        &module_names,
                        row,
                        &standalone.program.structs,
                        &standalone.program.enums,
                    );
                    builder.checked.insert(binding.clone());
                    let selector = &row.boundary.parameters[1];
                    let selector_binding = ReferenceBindingId {
                        declaration_span: selector.span,
                        ..binding.clone()
                    };
                    builder
                        .binding_types
                        .insert(selector_binding.clone(), selector.ty.clone());
                    let resolved = builder
                        .assignment_destination(&destination, ReferenceScopeId(0), ordinal)
                        .unwrap();
                    assert!(
                        matches!(resolved, ReferenceAssignmentDestination::Resolved(place) if place.binding == binding)
                    );
                    let [read, index] = builder.cfg.points.as_slice() else {
                        panic!("one preliminary root read followed by one selector read");
                    };
                    assert_eq!(read.source_span.range, root_span);
                    assert_eq!(
                        read.kind,
                        CfgPointKind::Read {
                            binding: binding.clone()
                        }
                    );
                    assert_eq!(
                        index.kind,
                        CfgPointKind::Read {
                            binding: selector_binding
                        }
                    );
                }

                let mut corrupt = row.clone();
                let NamedPlaceRootFacts::Prepared(roots) = &mut corrupt.named_place_roots else {
                    panic!("prepared roots");
                };
                assert_eq!(roots.remove(&root), Some(expected_root));
                let (cfg, errors) = derive(&function, &corrupt);
                assert_eq!(
                    errors,
                    [ReferenceAnalysisError::MissingContext {
                        source_span: SourceSpan::new(source.clone(), root_span),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    }]
                );
                assert!(cfg.points.iter().all(|point| !matches!(
                    point.kind,
                    CfgPointKind::Assign { .. }
                        | CfgPointKind::CaptureAssignmentPrevious { .. }
                        | CfgPointKind::Evaluate(ScalarExpression::Name { .. })
                )));
            }
        }
    }
}

#[test]
fn prepared_projected_namespace_assignment_uses_selected_base_on_both_layouts() {
    let source = source();
    let dependency = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "values.w".into(),
        "r1".into(),
    );
    let main = parse_source(
        source.clone(),
        "%%start\nimported = namespace package \"values.w\";\nunit(u16) run = fn(decoy) { imported.value = 4; decoy; };\n%%end".into(),
        &[],
    );
    let values = parse_source(
        dependency.clone(),
        "%%start\nu16 value = 1;\n%%end".into(),
        &[],
    );
    assert!(main.diagnostics.is_empty(), "{:?}", main.diagnostics);
    assert!(values.diagnostics.is_empty(), "{:?}", values.diagnostics);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(main.result.canonical_cst());
        let namespace = declarations.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            target: dependency.clone(),
            span: namespace.span,
        };
        let project = prepare_scalar_project(
            vec![
                (declarations, vec![binding]),
                (
                    derive_scalar_declarations_from_cst(values.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![dependency.clone(), source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let build = analyze_scalar_build(&project);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let module = &project.project().modules[0];
        let ScalarItem::Function(function) = &module.items[1] else {
            panic!("run declaration");
        };
        let mut function = function.clone();
        let ScalarBlockItem::Assignment(assignment) = &mut function.body.items[0] else {
            panic!("canonical assignment");
        };
        let target = &mut assignment.targets[0];
        let root_span = scalar_place_target_span(reference_root(&target.place));
        rename_destination_root(&mut target.place, "decoy");
        let row = project.function_calls(0, 1);
        let ordinal = row.assignments[&(row.structure.root, 0)]
            .destinations()
            .next()
            .unwrap()
            .destination;
        let OccurrenceNode::Place(root) =
            row.structure.edges[&(OccurrenceParent::Place(ordinal), OccurrenceEdge::PlaceBase)]
        else {
            panic!("namespace root ordinal");
        };
        assert_eq!(row.structure.resolved_namespace_places[&root], dependency);
        let module_names = HashSet::from(["imported".to_owned(), "run".to_owned()]);
        let derive = |row: &FunctionCallSites| {
            derive_function_reference_cfg(
                &source,
                &function,
                &module_names,
                row,
                &module.structs,
                &module.enums,
                true,
            )
        };
        let mut expected = build.reference_cfg(0, 1, None).unwrap().clone();
        for points in [&mut expected.points, &mut expected.initial_points] {
            let event = points
                .iter_mut()
                .find_map(|point| match &mut point.kind {
                    CfgPointKind::Evaluate(ScalarExpression::Name { name, span })
                        if *span == root_span =>
                    {
                        Some(name)
                    }
                    _ => None,
                })
                .expect("namespace evaluation-only event");
            assert_eq!(event, "imported");
            *event = "decoy".to_owned();
        }
        assert_eq!(derive(row), (expected, Vec::new()));

        let mut corrupt = row.clone();
        assert_eq!(
            corrupt.structure.resolved_namespace_places.remove(&root),
            Some(dependency.clone())
        );
        assert_eq!(
            derive(&corrupt).1,
            [ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(source.clone(), root_span),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            }]
        );
    }
}

#[test]
fn checked_assignment_retains_original_destination_before_retargeting_on_both_pipelines_and_layouts(
) {
    let text = "%%start\nunit() check = fn { u8 first = 1; u8 second = 2; *!u8 selected = &!first; selected, *selected = &!second, 9; u8 observed = *selected; u8 first_expected = 9; u8 observed_expected = 2; if (first != first_expected || observed != observed_expected) { core.system_panic(); }; };\ncheck();\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarSourceDeclarations {
            source,
            items,
            structs,
            enums,
            diagnostics,
        } = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        assert!(diagnostics.is_empty());
        let order = vec![source.clone()];
        let mut module = ScalarModule::new(source, items, Vec::new());
        module.structs = structs;
        module.enums = enums;
        module.target_layout = target;
        let project = validate_scalar_project(ScalarProject::new(vec![module], order));
        assert_eq!(project.diagnostics, single.diagnostics);
        assert_eq!(project.project.modules[0].items, single.program.items);
        for items in [&single.program.items, &project.project.modules[0].items] {
            let ScalarItem::Function(function) = &items[0] else {
                panic!("check")
            };
            let cfg = &function.reference_cfg;
            let direct = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::Assign { .. }))
                .expect("retarget install");
            let CfgPointKind::Assign {
                rhs_point: pointer_rhs,
                target: selected,
                ..
            } = &direct.kind
            else {
                unreachable!()
            };
            let through = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
                .expect("retained dereference install");
            let CfgPointKind::AssignThrough {
                rhs_point: byte_rhs,
                pointer_point,
                candidates,
                ..
            } = &through.kind
            else {
                unreachable!()
            };
            let capture = cfg
                .points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::CaptureAssignmentPrevious { .. }))
                .expect("assignment group capture");
            let CfgPointKind::CaptureAssignmentPrevious { writes } = &capture.kind else {
                unreachable!()
            };
            assert_eq!(writes, &[direct.id, through.id]);
            let ScalarBlockItem::Assignment(assignment) = &function.body.items[3] else {
                panic!("retargeting assignment")
            };
            assert_eq!(
                capture.source_span,
                SourceSpan::new(single.program.source.clone(), assignment.span)
            );
            assert_eq!(capture.scope, direct.scope);
            assert_eq!(capture.possible_origins, Some(BTreeSet::new()));
            assert!(pointer_rhs.0 < byte_rhs.0);
            assert!(byte_rhs.0 < pointer_point.0);
            assert!(pointer_point.0 < capture.id.0);
            assert!(capture.id.0 < direct.id.0);
            assert!(direct.id.0 < through.id.0);
            let rhs_start = text.find("&!second, 9").unwrap() as u32;
            assert_eq!(
                cfg.points[pointer_rhs.0].source_span.range,
                ByteSpan::new(rhs_start, rhs_start + 8)
            );
            assert_eq!(
                cfg.points[byte_rhs.0].source_span.range,
                ByteSpan::new(rhs_start + 10, rhs_start + 11)
            );
            let selector = &cfg.points[pointer_point.0];
            let CfgPointKind::Read { binding } = &selector.kind else {
                panic!("direct checked-pointer evaluation")
            };
            assert_eq!(binding, &selected.binding);
            let selector_start = text.find("*selected =").unwrap() as u32 + 1;
            assert_eq!(
                selector.source_span,
                SourceSpan::new(
                    single.program.source.clone(),
                    ByteSpan::new(selector_start, selector_start + 8)
                )
            );
            assert_eq!(
                cfg.points
                    .iter()
                    .filter(|point| point.source_span == selector.source_span
                        && matches!(point.kind, CfgPointKind::Read { .. }))
                    .count(),
                1
            );
            assert_eq!(selector.scope, direct.scope);
            assert_eq!(selector.scope, through.scope);
            let first = text.find("first =").unwrap() as u32;
            let second = text.find("second =").unwrap() as u32;
            assert_eq!(candidates.len(), 1);
            assert_eq!(
                candidates[0].place.binding.declaration_span,
                ByteSpan::new(first, first + 5)
            );
            assert_eq!(candidates[0].place.source, single.program.source);
            assert_eq!(candidates[0].place.function_span, function.span);
            let original = selector
                .possible_origins
                .as_ref()
                .expect("retained first origin");
            assert_eq!(original.len(), 1);
            let ReferenceOrigin::Fresh { loan, .. } = &cfg.facts[original.first().unwrap().0]
            else {
                panic!("first storage origin")
            };
            assert_eq!(
                loan.origin_place.binding.declaration_span,
                ByteSpan::new(first, first + 5)
            );
            let later_start = text.find("*selected;").unwrap() as u32 + 1;
            let later = cfg
                .points
                .iter()
                .find(|point| {
                    point.source_span.range == ByteSpan::new(later_start, later_start + 8)
                        && matches!(point.kind, CfgPointKind::Read { .. })
                })
                .expect("later selected read");
            assert!(through.id.0 < later.id.0);
            let origins = later
                .possible_origins
                .as_ref()
                .expect("retargeted second origin");
            assert_eq!(origins.len(), 1);
            let ReferenceOrigin::Fresh { loan, .. } = &cfg.facts[origins.first().unwrap().0] else {
                panic!("second storage origin")
            };
            assert_eq!(
                loan.origin_place.binding.declaration_span,
                ByteSpan::new(second, second + 6)
            );
        }
    }
}

#[test]
fn grouped_projected_writes_consume_saved_previous_origins_including_empty_sets() {
    let source = source();
    let span = ByteSpan::new(0, 100);
    let binding = |position| ReferenceBindingId {
        source: source.clone(),
        function_span: span,
        declaration_span: ByteSpan::new(position, position + 1),
        block_span: span,
        kind: ReferenceBindingKind::Declared,
    };
    let cell = binding(1);
    let watch = binding(2);
    let field = ScalarFieldReference::Resolved(ScalarStructFieldId {
        structure: ScalarStructId {
            source: source.clone(),
            index: 0,
        },
        index: 0,
    });
    let projected = ReferencePlaceId {
        source: source.clone(),
        function_span: span,
        declaration_span: cell.declaration_span,
        block_span: span,
        binding: cell.clone(),
        projections: vec![ReferencePlaceProjection::Field(field.clone())],
        place: ScalarPlace::Field {
            base: Box::new(ScalarPlace::Name {
                name: "cell".into(),
                span: cell.declaration_span,
            }),
            field,
            span: ByteSpan::new(10, 20),
        },
    };
    let loans = (0..3)
        .map(|position| ReferenceLoanId {
            mode: ScalarReferenceMutability::Shared,
            origin_place: projected.clone(),
            creation_span: ByteSpan::new(30 + position, 31 + position),
            parent: None,
        })
        .collect::<Vec<_>>();
    let facts = loans
        .iter()
        .enumerate()
        .map(|(parameter, loan)| ReferenceOrigin::BorrowedFrom {
            parameter,
            loan: loan.clone(),
        })
        .collect::<Vec<_>>();
    for seeded in [false, true] {
        for through in [false, true] {
            let mut points = Vec::new();
            let mut add = |kind, possible_origins| {
                let index = points.len();
                points.push(ReferenceFlowPoint {
                    id: CfgPointId(index),
                    kind,
                    source_span: SourceSpan::new(
                        source.clone(),
                        ByteSpan::new(50 + index as u32, 51 + index as u32),
                    ),
                    scope: ReferenceScopeId(0),
                    possible_origins,
                    successors: if index < 8 {
                        vec![CfgPointId(index + 1)]
                    } else {
                        Vec::new()
                    },
                });
            };
            add(CfgPointKind::Entry, Some(BTreeSet::new()));
            add(
                CfgPointKind::Borrow {
                    place: projected.clone(),
                    loan: loans[0].clone(),
                },
                Some(BTreeSet::from([ReferenceOriginId(0)])),
            );
            add(
                CfgPointKind::Bind {
                    binding: watch.clone(),
                },
                Some(if seeded {
                    BTreeSet::from([ReferenceOriginId(0)])
                } else {
                    BTreeSet::new()
                }),
            );
            add(
                CfgPointKind::Borrow {
                    place: projected.clone(),
                    loan: loans[1].clone(),
                },
                Some(BTreeSet::from([ReferenceOriginId(1)])),
            );
            add(
                CfgPointKind::Borrow {
                    place: projected.clone(),
                    loan: loans[2].clone(),
                },
                Some(BTreeSet::from([ReferenceOriginId(2)])),
            );
            add(
                CfgPointKind::Evaluate(ScalarExpression::Integer {
                    value: 0.into(),
                    span: ByteSpan::new(55, 56),
                }),
                Some(BTreeSet::new()),
            );
            add(
                CfgPointKind::CaptureAssignmentPrevious {
                    writes: vec![CfgPointId(7), CfgPointId(8)],
                },
                Some(BTreeSet::new()),
            );
            add(
                CfgPointKind::Assign {
                    target: ReferencePlaceId {
                        source: source.clone(),
                        function_span: span,
                        declaration_span: watch.declaration_span,
                        block_span: span,
                        binding: watch.clone(),
                        projections: Vec::new(),
                        place: ScalarPlace::Name {
                            name: "watch".into(),
                            span: watch.declaration_span,
                        },
                    },
                    rhs_point: CfgPointId(3),
                    previous_origins: BTreeSet::new(),
                    previous_loans: Vec::new(),
                },
                None,
            );
            add(
                if through {
                    CfgPointKind::AssignThrough {
                        place: ScalarPlace::Dereference {
                            pointer: Box::new(ScalarExpression::Name {
                                name: "selector".into(),
                                span: ByteSpan::new(58, 59),
                            }),
                            span: ByteSpan::new(57, 59),
                        },
                        pointer_point: CfgPointId(4),
                        rhs_point: CfgPointId(5),
                        candidates: Vec::new(),
                        previous_origins: BTreeSet::new(),
                        previous_loans: Vec::new(),
                    }
                } else {
                    CfgPointKind::Assign {
                        target: projected.clone(),
                        rhs_point: CfgPointId(5),
                        previous_origins: BTreeSet::new(),
                        previous_loans: Vec::new(),
                    }
                },
                Some(BTreeSet::new()),
            );
            let mut grouped = ScalarReferenceCfg {
                primary_call_points: HashMap::new(),
                runtime_array_read_sites: HashMap::new(),
                runtime_array_child_sites: Vec::new(),
                runtime_array_exclusive_sites: HashMap::new(),
                runtime_array_field_sites: HashMap::new(),
                observed_runtime_allocations: None,
                guarded_scalar_allocation: None,
                facts: facts.clone(),
                points,
                initial_points: Vec::new(),
                allocations: BTreeSet::new(),
                specialized_policies: Vec::new(),
            };
            let mut per_write = grouped.clone();
            per_write.points[6].kind = CfgPointKind::Join;
            grouped.initial_points = grouped.points.clone();
            let errors = transfer_reference_facts(ReferenceTransferCfg::new(&mut grouped, None), &BTreeMap::new(), &HashMap::new(), None, None);
            assert!(
                errors.is_empty(),
                "seeded={seeded} through={through}: {errors:?}"
            );
            let expected = if seeded {
                BTreeSet::from([ReferenceOriginId(0)])
            } else {
                BTreeSet::new()
            };
            let (previous_origins, previous_loans) = match &grouped.points[8].kind {
                CfgPointKind::Assign {
                    previous_origins,
                    previous_loans,
                    ..
                }
                | CfgPointKind::AssignThrough {
                    previous_origins,
                    previous_loans,
                    ..
                } => (previous_origins, previous_loans),
                _ => unreachable!(),
            };
            assert_eq!(previous_origins, &expected);
            assert_eq!(
                previous_loans,
                &if seeded {
                    vec![loans[0].clone()]
                } else {
                    Vec::new()
                }
            );
            assert!(!previous_origins.contains(&ReferenceOriginId(1)));
            per_write.initial_points = per_write.points.clone();
            let errors = transfer_reference_facts(ReferenceTransferCfg::new(&mut per_write, None), &BTreeMap::new(), &HashMap::new(), None, None);
            assert!(errors.is_empty(), "{errors:?}");
            let (previous_origins, previous_loans) = match &per_write.points[8].kind {
                CfgPointKind::Assign {
                    previous_origins,
                    previous_loans,
                    ..
                }
                | CfgPointKind::AssignThrough {
                    previous_origins,
                    previous_loans,
                    ..
                } => (previous_origins, previous_loans),
                _ => unreachable!(),
            };
            assert_eq!(previous_origins, &BTreeSet::from([ReferenceOriginId(1)]));
            assert_eq!(previous_loans, &vec![loans[1].clone()]);
        }
    }
}
