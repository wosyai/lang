use super::*;

const TEXT: &str = "%%start
u8 global = 0;
u8(u8) run = fn(input) { next, input, global = input, 2, 3; next = input; next };
%%end";

fn destinations(
    row: &FunctionCallSites,
    item: usize,
) -> impl Iterator<Item = OccurrenceOrdinal> + '_ {
    row.assignments[&(row.structure.root, item)]
        .destinations()
        .map(|slot| slot.destination)
}

fn assignment(function: &mut ScalarFunction, index: usize) -> &mut ScalarAssignment {
    let ScalarBlockItem::Assignment(assignment) = &mut function.body.items[index] else {
        panic!("canonical assignment");
    };
    assignment
}

fn rename(target: &mut ScalarAssignmentTarget, spelling: &str) {
    let ScalarPlace::Name { name, .. } = &mut target.place else {
        panic!("direct destination");
    };
    *name = spelling.to_owned();
}

#[test]
fn prepared_direct_destinations_select_installs_on_both_routes_and_layouts() {
    let source = source();
    let parsed = parse_source(source.clone(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let module_names = HashSet::from(["global".to_owned(), "run".to_owned()]);
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
        let standalone_calls = standalone.function_calls(1).clone();
        let standalone = standalone.finish();
        assert!(
            standalone.diagnostics.is_empty(),
            "{:?}",
            standalone.diagnostics
        );
        let build = analyze_scalar_build(&project);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());

        let ScalarItem::Binding(global) = &standalone.program.items[0] else {
            panic!("global")
        };
        let ScalarItem::Function(single) = &standalone.program.items[1] else {
            panic!("standalone run")
        };
        let ScalarItem::Function(project_function) = &project.project().modules[0].items[1] else {
            panic!("project run")
        };
        let baseline = &single.reference_cfg;
        assert_eq!(build.reference_cfg(0, 1, None), Some(baseline));
        let raw = resolve_program_call_sites(&standalone.program).unwrap();
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
            derive(single, raw.function(1)),
            (baseline.clone(), Vec::new())
        );
        for (function, row) in [
            (single, &standalone_calls),
            (project_function, project.function_calls(0, 1)),
        ] {
            let mut function = function.clone();
            let order = destinations(row, 0)
                .chain(destinations(row, 1))
                .collect::<Vec<_>>();
            let [introduced, existing, module, later] = order.as_slice() else {
                panic!("direct assignment slots")
            };
            let DirectDestinationFacts::Prepared(rows) = &row.direct_destinations else {
                panic!("prepared identities")
            };
            let binding = |declaration_span| ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            };
            let next_id = binding(row.occurrences[introduced.0].span);
            assert_eq!(
                rows[introduced],
                DirectDestinationIdentity::Introduce {
                    binding: next_id.clone(),
                    name: "next".to_owned(),
                }
            );
            assert_eq!(
                rows[existing],
                DirectDestinationIdentity::Existing(binding(function.parameter_spans[0]))
            );
            assert_eq!(
                rows[module],
                DirectDestinationIdentity::Module(SourceSpan::new(
                    source.clone(),
                    global.name_span
                ))
            );
            assert_eq!(rows[later], DirectDestinationIdentity::Existing(next_id));

            let spellings = ["global", "next", "input", "input"];
            for (target, spelling) in assignment(&mut function, 0)
                .targets
                .iter_mut()
                .zip(spellings)
            {
                rename(target, spelling);
            }
            rename(&mut assignment(&mut function, 1).targets[0], spellings[3]);
            let mut expected = baseline.clone();
            for points in [&mut expected.points, &mut expected.initial_points] {
                for (index, (ordinal, spelling)) in order.iter().zip(spellings).enumerate() {
                    let span = row.occurrences[ordinal.0].span;
                    let point = points
                        .iter_mut()
                        .find(|point| {
                            point.source_span.range == span
                                && matches!(
                                    point.kind,
                                    CfgPointKind::Assign { .. }
                                        | CfgPointKind::Evaluate(ScalarExpression::Name { .. })
                                )
                        })
                        .expect("retained install event");
                    match &mut point.kind {
                        CfgPointKind::Assign { target, .. } if index != 2 => {
                            let ScalarPlace::Name { name, .. } = &mut target.place else {
                                panic!("named place")
                            };
                            *name = spelling.to_owned();
                        }
                        CfgPointKind::Evaluate(ScalarExpression::Name { name, .. })
                            if index == 2 =>
                        {
                            *name = spelling.to_owned();
                        }
                        _ => panic!("expected assignment classification"),
                    }
                }
            }
            assert_eq!(derive(&function, row), (expected, Vec::new()));

            for ordinal in order {
                let mut corrupt = row.clone();
                let DirectDestinationFacts::Prepared(rows) = &mut corrupt.direct_destinations
                else {
                    unreachable!()
                };
                assert!(rows.remove(&ordinal).is_some());
                assert_eq!(
                    derive(&function, &corrupt).1,
                    [ReferenceAnalysisError::MissingContext {
                        source_span: SourceSpan::new(
                            source.clone(),
                            row.occurrences[ordinal.0].span
                        ),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    }]
                );
            }
        }
    }
}

#[test]
fn direct_destination_overlay_reuses_first_introduction_in_install_order() {
    // Source validation rejects duplicate destinations. Inject one into the typed
    // canonical group to exercise the structural walker's install-order correspondence.
    let text =
        "%%start\nu8(u8) run = fn(input) { first, second = input, input; first; second };\n%%end";
    let repeat = |items: &mut [ScalarItem]| {
        let ScalarItem::Function(function) = &mut items[0] else {
            panic!("run")
        };
        rename(&mut assignment(function, 0).targets[1], "first");
        let Some(ScalarBlockItem::Expression(ScalarExpression::Name { name, .. })) =
            function.body.items.last_mut()
        else {
            panic!("final name read");
        };
        *name = "first".to_owned();
    };
    let assert_overlay = |calls: &mut ModuleCallSites, source: &SourceIdentity| {
        let row = calls.function_mut(0);
        let order = destinations(row, 0).collect::<Vec<_>>();
        let DirectDestinationFacts::Prepared(rows) = &mut row.direct_destinations else {
            unreachable!()
        };
        let DirectDestinationIdentity::Introduce { binding, name } = &rows[&order[0]] else {
            panic!("first introduction")
        };
        assert_eq!(name, "first");
        assert_eq!(binding.declaration_span, row.occurrences[order[0].0].span);
        let binding = binding.clone();
        assert_eq!(
            rows.remove(&order[1]),
            Some(DirectDestinationIdentity::Existing(binding))
        );
        let span = row.occurrences[order[1].0].span;
        assert_eq!(
            occurrence_obligations::validate_module(calls),
            Err(ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(source.clone(), span),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            })
        );
    };
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (mut typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            layout,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        repeat(&mut typed.program.items);
        let mut calls = resolve_program_call_sites_with_occurrences(&typed.program, occurrences)
            .expect("structural topology");
        assert_overlay(&mut calls, &typed.program.source);

        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let mut module =
            ScalarModule::new(declarations.source.clone(), declarations.items, Vec::new());
        module.target_layout = layout;
        let (mut typed, occurrences) = typecheck_scalar_project_with_occurrences(
            ScalarProject::new(vec![module], vec![declarations.source.clone()]),
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        repeat(&mut typed.project.modules[0].items);
        let mut calls =
            resolve_project_call_sites_with_occurrences(&typed.project.modules, occurrences);
        assert_overlay(
            calls.modules[0].as_mut().expect("structural topology"),
            &declarations.source,
        );
    }
}
