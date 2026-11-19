use super::*;

const TEXT: &str = "%%start
struct Payload { i32 value; }
u8[2] first, u8[2] second = [1, 2], [3, 4];
u8(u8[2], u8[2]) read = fn(input, decoy) { decoy[0]; input[0] };
u8(u8[2]) module_read = fn(decoy) { decoy[0]; second[0] };
Payload(Payload) take = fn(value) { *!Payload writer = &!value; Payload moved = *writer; moved };
%%end";

#[test]
fn prepared_projected_reads_use_frozen_roots_places_and_pointers_on_both_routes_and_layouts() {
    let source = source();
    let parsed = parse_source(source.clone(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let module_names = HashSet::from([
        "first".to_owned(),
        "read".to_owned(),
        "module_read".to_owned(),
        "take".to_owned(),
    ]);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let stages = stage_observations::Scope::new();
        let standalone = static_calls::PreparedProgram::new(declarations(), target_layout)
            .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let project = prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 2,
                ..stage_observations::Counts::default()
            }
        );
        drop(stages);
        let single_rows = [1, 2, 3].map(|item| standalone.function_calls(item).clone());
        let single = standalone.finish();
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let build = analyze_scalar_build(&project);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let raw = resolve_program_call_sites(&single.program).unwrap();
        let ScalarItem::Binding(global) = &single.program.items[0] else {
            panic!("module binding")
        };
        let secondary = SourceSpan::new(source.clone(), global.receivers[1].name_span);

        for (item, single_row) in (1..=3).zip(&single_rows) {
            let ScalarItem::Function(single_function) = &single.program.items[item] else {
                panic!("standalone callable")
            };
            let ScalarItem::Function(project_function) = &project.project().modules[0].items[item]
            else {
                panic!("project callable")
            };
            let baseline = &single_function.reference_cfg;
            assert_eq!(build.reference_cfg(0, item, None), Some(baseline));
            let derive = |function: &ScalarFunction, row: &FunctionCallSites| {
                derive_function_reference_cfg(
                    &source,
                    function,
                    &module_names,
                    row,
                    &single.program.structs,
                    &single.program.enums,
                    true,
                )
            };
            for (function, row) in [
                (single_function, single_row),
                (project_function, project.function_calls(0, item)),
            ] {
                let (mut expected, errors) = derive(function, row);
                assert!(errors.is_empty(), "{errors:?}");
                let mut function = function.clone();
                let mut corruptions;
                if item == 3 {
                    let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[1] else {
                        panic!("moved binding")
                    };
                    let ScalarExpression::Dereference { place, span } = &mut binding.value else {
                        panic!("dereference")
                    };
                    let ScalarPlace::Dereference { pointer, .. } = place else {
                        panic!("dereference root")
                    };
                    let ScalarExpression::Name {
                        name,
                        span: pointer_span,
                    } = pointer.as_mut()
                    else {
                        panic!("pointer name")
                    };
                    *name = "value".to_owned();
                    let span = *span;
                    let pointer_span = *pointer_span;
                    let place_ordinal = row
                        .occurrences
                        .iter()
                        .enumerate()
                        .find_map(|(index, metadata)| {
                            let ordinal = OccurrenceOrdinal(index);
                            (metadata.span == span
                                && row.structure.edges.contains_key(&(
                                    OccurrenceParent::Place(ordinal),
                                    OccurrenceEdge::DereferencePointer,
                                )))
                            .then_some(ordinal)
                        })
                        .expect("dereference place ordinal");
                    let OccurrenceNode::Expression(pointer_ordinal) = row.structure.edges[&(
                        OccurrenceParent::Place(place_ordinal),
                        OccurrenceEdge::DereferencePointer,
                    )] else {
                        panic!("pointer ordinal")
                    };
                    for points in [&mut expected.points, &mut expected.initial_points] {
                        let mut mutations = 0;
                        for point in points {
                            match &mut point.kind {
                                CfgPointKind::ReadPlace {
                                    place,
                                    pointer_point: Some(_),
                                    candidates,
                                    ..
                                } if point.source_span.range == span => {
                                    *place = ScalarPlace::Dereference {
                                        pointer: pointer.clone(),
                                        span,
                                    };
                                    for candidate in candidates {
                                        candidate.place = place.clone();
                                    }
                                    mutations += 1;
                                }
                                CfgPointKind::MoveThrough {
                                    place, candidates, ..
                                } if point.source_span.range == span => {
                                    *place = ScalarPlace::Dereference {
                                        pointer: pointer.clone(),
                                        span,
                                    };
                                    for candidate in candidates {
                                        candidate.place.place = place.clone();
                                    }
                                    mutations += 1;
                                }
                                _ => {}
                            }
                        }
                        assert_eq!(mutations, 2);
                    }
                    let mut missing_name = row.clone();
                    let PlainNameFacts::Prepared(names) = &mut missing_name.plain_names else {
                        unreachable!()
                    };
                    assert!(matches!(
                        names.remove(&pointer_ordinal),
                        Some(PlainNameIdentity::Lexical(_))
                    ));
                    let mut missing_type = row.clone();
                    assert!(missing_type
                        .structure
                        .checked_types
                        .expressions
                        .remove(&pointer_ordinal)
                        .is_some());
                    corruptions = vec![
                        (
                            missing_name,
                            pointer_span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        ),
                        (
                            missing_type,
                            pointer_span,
                            ReferenceAnalysisErrorReason::MissingCheckedExpressionResult {
                                ordinal: pointer_ordinal.0,
                            },
                        ),
                    ];
                } else {
                    let final_read = row.structure.blocks[row.structure.root.0]
                        .final_outputs()
                        .next()
                        .unwrap()
                        .expression;
                    let OccurrenceNode::Place(place_ordinal) = row.structure.edges[&(
                        OccurrenceParent::Expression(final_read),
                        OccurrenceEdge::ExpressionPlace,
                    )] else {
                        panic!("indexed place ordinal")
                    };
                    let OccurrenceNode::Place(root_ordinal) = row.structure.edges[&(
                        OccurrenceParent::Place(place_ordinal),
                        OccurrenceEdge::PlaceBase,
                    )] else {
                        panic!("named root ordinal")
                    };
                    let Some(ScalarBlockItem::Expression(
                        expression @ ScalarExpression::IndexedRead { .. },
                    )) = function.body.items.last_mut()
                    else {
                        panic!("indexed read")
                    };
                    let ScalarExpression::IndexedRead { place, span } = expression else {
                        unreachable!()
                    };
                    let ScalarPlace::Index { base, .. } = place else {
                        panic!("index place")
                    };
                    let ScalarPlace::Name {
                        name,
                        span: root_span,
                    } = base.as_mut()
                    else {
                        panic!("named root")
                    };
                    let root_span = *root_span;
                    let span = *span;
                    let NamedPlaceRootFacts::Prepared(roots) = &row.named_place_roots else {
                        unreachable!()
                    };
                    if item == 2 {
                        assert_eq!(
                            roots[&root_ordinal],
                            NamedPlaceRootIdentity::ModuleValue(secondary.clone())
                        );
                        let errors = derive(single_function, raw.function(item)).1;
                        assert!(
                            matches!(errors.as_slice(), [ReferenceAnalysisError::MissingBinding { source_span, name }] if source_span.range == root_span && name == "second")
                        );
                    } else {
                        assert!(
                            matches!(&roots[&root_ordinal], NamedPlaceRootIdentity::Lexical(binding) if binding.declaration_span == function.parameter_spans[0])
                        );
                    }
                    *name = "decoy".to_owned();
                    let mutated_place = place.clone();
                    for points in [&mut expected.points, &mut expected.initial_points] {
                        let point = points
                            .iter_mut()
                            .find(|point| {
                                point.source_span.range == span
                                    && matches!(
                                        point.kind,
                                        CfgPointKind::ReadPlace { .. }
                                            | CfgPointKind::Evaluate(
                                                ScalarExpression::IndexedRead { .. }
                                            )
                                    )
                            })
                            .expect("projected read event");
                        match &mut point.kind {
                            CfgPointKind::ReadPlace {
                                place, candidates, ..
                            } if item == 1 => {
                                assert_eq!(
                                    candidates[0].binding.declaration_span,
                                    function.parameter_spans[0]
                                );
                                *place = mutated_place.clone();
                                candidates[0].place = mutated_place.clone();
                            }
                            CfgPointKind::Evaluate(value) if item == 2 => {
                                *value = expression.clone()
                            }
                            _ => panic!("original read classification"),
                        }
                    }
                    let mut missing_root = row.clone();
                    let NamedPlaceRootFacts::Prepared(roots) = &mut missing_root.named_place_roots
                    else {
                        unreachable!()
                    };
                    assert!(roots.remove(&root_ordinal).is_some());
                    let mut missing_type = row.clone();
                    assert!(missing_type
                        .structure
                        .checked_types
                        .places
                        .remove(&place_ordinal)
                        .is_some());
                    corruptions = vec![(
                        missing_root,
                        root_span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )];
                    if item == 1 {
                        corruptions.push((
                            missing_type,
                            row.occurrences[place_ordinal.0].span,
                            ReferenceAnalysisErrorReason::MissingCheckedPlaceType {
                                ordinal: place_ordinal.0,
                            },
                        ));
                    }
                }
                assert_eq!(derive(&function, row), (expected, Vec::new()));
                for (corrupt, span, reason) in corruptions {
                    assert_eq!(
                        derive(&function, &corrupt).1,
                        [ReferenceAnalysisError::MissingContext {
                            source_span: SourceSpan::new(source.clone(), span),
                            reason
                        }]
                    );
                }
            }
        }
    }
}

#[test]
fn static_indexed_root_obligation_rejects_missing_identity_at_the_root_token() {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source(), TEXT.into(), &[]);
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            layout,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut calls =
            resolve_program_call_sites_with_occurrences(&typed.program, occurrences).unwrap();
        let row = calls.function_mut(2);
        let start = TEXT.find("second[0]").unwrap() as u32;
        let span = ByteSpan::new(start, start + "second".len() as u32);
        let index = row
            .occurrences
            .iter()
            .position(|metadata| metadata.requires_named_place_root && metadata.span == span)
            .expect("indexed root obligation");
        let NamedPlaceRootFacts::Prepared(roots) = &mut row.named_place_roots else {
            unreachable!()
        };
        assert!(roots.remove(&OccurrenceOrdinal(index)).is_some());
        let stages = stage_observations::Scope::new();
        assert_eq!(
            occurrence_obligations::validate_module(&calls),
            Err(ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(typed.program.source.clone(), span),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            })
        );
        assert_eq!(stages.counts(), stage_observations::Counts::default());
    }
}
