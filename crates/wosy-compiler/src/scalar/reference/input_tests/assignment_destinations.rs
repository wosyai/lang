use super::*;

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
                facts: facts.clone(),
                points,
                initial_points: Vec::new(),
                allocations: BTreeSet::new(),
                specialized_policies: Vec::new(),
            };
            let mut per_write = grouped.clone();
            per_write.points[6].kind = CfgPointKind::Join;
            let errors = transfer_reference_facts(&mut grouped, &BTreeMap::new(), &[]);
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
            let errors = transfer_reference_facts(&mut per_write, &BTreeMap::new(), &[]);
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
