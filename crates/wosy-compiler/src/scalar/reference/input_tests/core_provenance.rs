use super::*;

const TEXT: &str = "%%start
unit() release_raw = fn { unsafe { *?u8 storage = core.alloc(1, 1); core.free(storage); } };
unit(u64, bool) release_checked = fn(count, flag) { u8[count] first; u8[count] second; *u8 alias = &first[0]; if (flag) { alias = &second[0]; }; unsafe { core.free(alias); } };
*!u8(*?u8) cast_name = fn(raw) { unsafe { core.pointer_cast<*!u8>(raw) } };
*!u8(u8[2]) cast_address = fn(bytes) { unsafe { core.pointer_cast<*!u8>(&?bytes[0]) } };
*!u8() cast_null = fn { unsafe { core.pointer_cast<*!u8>(null) } };
%%end";

#[test]
fn prepared_core_provenance_uses_argument_facts_on_both_routes_and_layouts() {
    let source = source();
    let parsed = parse_source(source.clone(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let module_names = [
        "release_raw",
        "release_checked",
        "cast_name",
        "cast_address",
        "cast_null",
    ]
    .map(str::to_owned)
    .into_iter()
    .collect();
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = static_calls::PreparedProgram::new(declarations(), target_layout)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let project = prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let rows = [0, 1, 2, 3, 4].map(|item| single.function_calls(item).clone());
        let single = single.finish();
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let build = analyze_scalar_build(&project);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let raw = resolve_program_call_sites(&single.program).unwrap();
        for (item, single_row) in rows.iter().enumerate() {
            let ScalarItem::Function(single_function) = &single.program.items[item] else {
                panic!("standalone function")
            };
            let ScalarItem::Function(project_function) = &project.project().modules[0].items[item]
            else {
                panic!("project function")
            };
            assert_eq!(
                build.reference_cfg(0, item, None),
                Some(&single_function.reference_cfg)
            );
            for (function, row) in [
                (single_function, single_row),
                (project_function, project.function_calls(0, item)),
            ] {
                let derive = |f: &ScalarFunction, row: &FunctionCallSites| {
                    derive_function_reference_cfg(&source, f, &module_names, row, &[], &[], true)
                };
                let (baseline, errors) = derive(function, row);
                assert!(errors.is_empty(), "{errors:?}");
                assert_eq!(
                    derive(function, raw.function(item)),
                    (baseline.clone(), Vec::new())
                );
                let mut function = function.clone();
                let Some(ScalarBlockItem::Expression(ScalarExpression::Block(block))) =
                    function.body.items.last_mut()
                else {
                    panic!("unsafe block")
                };
                let Some(ScalarBlockItem::Expression(call @ ScalarExpression::Call { .. })) =
                    block.items.last_mut()
                else {
                    panic!("core call")
                };
                let ScalarExpression::Call {
                    arguments, span, ..
                } = call
                else {
                    unreachable!()
                };
                let call_span = *span;
                let call_ordinal = row
                    .call_occurrences
                    .iter()
                    .find(|call| row.occurrences[call.ordinal.0].span == call_span)
                    .expect("call occurrence")
                    .ordinal;
                let OccurrenceNode::Expression(argument_ordinal) = row.structure.edges[&(
                    OccurrenceParent::Expression(call_ordinal),
                    OccurrenceEdge::CallArgument(0),
                )] else {
                    panic!("first argument")
                };
                let mut corrupt = row.clone();
                let (binding, missing_span) = match &mut arguments[0] {
                    ScalarExpression::Name { name, span } => {
                        let PlainNameFacts::Prepared(names) = &mut corrupt.plain_names else {
                            unreachable!()
                        };
                        let binding = match names.remove(&argument_ordinal).unwrap() {
                            PlainNameIdentity::Lexical(binding) => Some(binding),
                            PlainNameIdentity::Null if item == 4 => None,
                            _ => panic!("argument identity"),
                        };
                        *name = if item == 2 { "null" } else { "decoy" }.to_owned();
                        (binding, *span)
                    }
                    ScalarExpression::RawAddress {
                        place: ScalarPlace::Index { base, .. },
                        ..
                    } => {
                        let ScalarPlace::Name { name, span } = base.as_mut() else {
                            panic!("address root")
                        };
                        *name = "decoy".into();
                        let edges = &row.structure.edges;
                        let OccurrenceNode::Place(place) = edges[&(
                            OccurrenceParent::Expression(argument_ordinal),
                            OccurrenceEdge::ExpressionPlace,
                        )] else {
                            panic!("address place")
                        };
                        let OccurrenceNode::Place(root) =
                            edges[&(OccurrenceParent::Place(place), OccurrenceEdge::PlaceBase)]
                        else {
                            panic!("root place")
                        };
                        let NamedPlaceRootFacts::Prepared(roots) = &mut corrupt.named_place_roots
                        else {
                            unreachable!()
                        };
                        let Some(NamedPlaceRootIdentity::Lexical(binding)) = roots.remove(&root)
                        else {
                            panic!("lexical root")
                        };
                        (Some(binding), *span)
                    }
                    _ => panic!("expected named or address argument"),
                };
                let expected_call = call.clone();
                let (cfg, errors) = derive(&function, row);
                assert!(errors.is_empty(), "{errors:?}");
                assert_eq!(cfg.points.len(), baseline.points.len());
                let event = cfg
                    .points
                    .iter()
                    .find(|point| {
                        point.source_span.range == call_span
                            && matches!(point.kind, CfgPointKind::Call { .. })
                    })
                    .unwrap();
                let CfgPointKind::Call {
                    argument_points,
                    expression,
                    ..
                } = &event.kind
                else {
                    unreachable!()
                };
                assert_eq!(expression, &expected_call);
                let CfgPointKind::Call {
                    argument_points: original,
                    ..
                } = &baseline.points[event.id.0].kind
                else {
                    panic!("original call event")
                };
                assert_eq!(argument_points, original);
                let argument_point = argument_points[0];
                assert!(argument_point.0 < event.id.0);
                if item < 2 {
                    let release = &cfg.points[event.id.0 - 1];
                    assert_eq!(release, &baseline.points[release.id.0]);
                    match &release.kind {
                        CfgPointKind::Release {
                            target: ReleaseTarget::Raw { binding: owner },
                        } if item == 0 => {
                            assert_eq!(Some(owner), binding.as_ref());
                        }
                        CfgPointKind::Release {
                            target: ReleaseTarget::Checked { address_point, .. },
                        } if item == 1 => {
                            assert_eq!(*address_point, argument_point);
                            assert_eq!(
                                cfg.points[argument_point.0].kind,
                                CfgPointKind::Read {
                                    binding: binding.unwrap()
                                }
                            );
                        }
                        _ => panic!("release event"),
                    }
                } else {
                    let origins = event.possible_origins.as_ref().unwrap();
                    assert_eq!(
                        origins,
                        baseline.points[event.id.0]
                            .possible_origins
                            .as_ref()
                            .unwrap()
                    );
                    assert_eq!(origins.len(), 1);
                    let id = origins.first().unwrap().0;
                    let mut expected = baseline.facts[id].clone();
                    if let ReferenceOrigin::Fresh { loan, .. } = &mut expected {
                        assert_eq!(Some(&loan.origin_place.binding), binding.as_ref());
                        let ScalarExpression::Call { arguments, .. } = &expected_call else {
                            unreachable!()
                        };
                        loan.origin_place.place = match &arguments[0] {
                            ScalarExpression::Name { name, span } => ScalarPlace::Name {
                                name: name.clone(),
                                span: *span,
                            },
                            ScalarExpression::RawAddress { place, .. } => place.clone(),
                            _ => unreachable!(),
                        };
                    } else {
                        assert_eq!(expected, ReferenceOrigin::Null { span: call_span });
                    }
                    assert_eq!(cfg.facts[id], expected);
                }
                assert_eq!(
                    derive(&function, &corrupt).1,
                    [ReferenceAnalysisError::MissingContext {
                        source_span: SourceSpan::new(source.clone(), missing_span),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    }]
                );
            }
        }
    }
}
