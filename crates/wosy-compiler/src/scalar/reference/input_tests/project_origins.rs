use super::*;

fn raw_module(path: &str, text: &str, target: ScalarTargetLayout) -> ScalarModule {
    let identity =
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into());
    let parsed = parse_source(identity, text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let ScalarSourceDeclarations {
        source,
        items,
        structs,
        enums,
        diagnostics,
    } = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let mut module = ScalarModule::new(source, items, Vec::new());
    module.structs = structs;
    module.enums = enums;
    module.target_layout = target;
    module
}

fn project(modules: Vec<ScalarModule>) -> ScalarProject {
    let order = modules.iter().map(|module| module.source.clone()).collect();
    ScalarProject::new(modules, order)
}

#[test]
fn actual_project_origin_stage_copies_one_aggregate_inventory_and_preserves_convergence_failure() {
    let text = "%%start\nstruct Pair { *?u8 address; u32 count; }\nstruct Payload { u8 value; }\nenum Marker { ready; }\nu32(Pair) read = fn(input) { input.count };\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let raw = project(vec![
            raw_module("a.w", text, target),
            raw_module("b.w", text, target),
        ]);
        let public_input = raw.clone();
        let typed = typecheck_scalar_project(raw);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut derived = typed.project;
        let before = derived.clone();
        assert_ne!(
            derived.modules[0].structs[0].id,
            derived.modules[1].structs[0].id
        );
        for module in &derived.modules {
            let pair = &module.structs[0];
            let expected_offset = if target == ScalarTargetLayout::WASM32 {
                4
            } else {
                8
            };
            assert_eq!(
                module.layouts.field(&pair.fields[1].id).unwrap().offset,
                Some(expected_offset)
            );
        }
        let calls = resolve_project_call_sites(&derived.modules);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let (mut updates, errors, _) = derive_project_reference_origins(&derived.modules, calls);
        assert!(errors.is_empty(), "{errors:?}");
        let counts = scope.counts();
        assert_eq!(counts.program_copies, 0);
        assert_eq!(counts.module_copies, 0);
        assert_eq!(counts.cfg_copies, 0);
        assert_eq!(counts.struct_copies, 4);
        assert_eq!(counts.module_constructions, 0);
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 0,
                reference_origins: 2,
                output_contracts: 0,
                copy_specialization: 0
            }
        );
        drop(stages);
        drop(scope);
        assert_eq!(derived, before);
        for (module, original) in derived.modules.iter().zip(&before.modules) {
            assert_eq!(module.structs, original.structs);
            assert_eq!(module.enums, original.enums);
            assert_eq!(module.layouts, original.layouts);
        }
        for (_, results) in &updates {
            for (_, result) in results {
                assert!(!result.cfg.points.is_empty());
                assert!(!result.cfg.initial_points.is_empty());
            }
        }
        let mut views = updates
            .iter_mut()
            .map(|(index, results)| ModuleReferenceView {
                source: &derived.modules[*index].source,
                items: &derived.modules[*index].items,
                results,
            })
            .collect::<Vec<_>>();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = resolve_reference_output_contracts(&mut views);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().output_contracts, 1);
        drop(stages);
        drop(scope);
        drop(views);
        assert_eq!(derived, before);
        for (index, results) in updates {
            install_module_reference_results(&mut derived.modules[index].items, results);
        }
        let unavailable = text.rfind("next };").unwrap() as u32;
        let moved = text.find("next; next").unwrap() as u32;
        let expected = derived
            .modules
            .iter()
            .map(|module| ReferenceAnalysisError::ConflictingPlace {
                source_span: SourceSpan::new(
                    module.source.clone(),
                    ByteSpan::new(unavailable, unavailable + 4),
                ),
                origins: vec![SourceSpan::new(
                    module.source.clone(),
                    ByteSpan::new(moved, moved + 4),
                )],
            })
            .collect::<Vec<_>>();
        assert_eq!(errors, expected);
        let public = validate_scalar_project(public_input);
        assert_eq!(
            public.diagnostics,
            errors
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>()
        );
        assert_eq!(public.project, derived);
    }
}

#[test]
fn module_lookup_failure_preserves_prior_slots_and_later_modules_still_attach() {
    let text = "%%start\nstruct Pair { u32 count; }\nu32(u32) identity = fn(input) { input };\nu32(u32) caller = fn(input) { identity(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let typed = typecheck_scalar_project(project(vec![
            raw_module("bad.w", text, target),
            raw_module("good.w", text, target),
        ]));
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut modules = typed.project.modules;
        let views = modules
            .iter()
            .map(CallResolutionModule::module)
            .collect::<Vec<_>>();
        let sites =
            resolve_module_call_sites(&modules[0].source, &modules[0].items, &views).unwrap();
        drop(views);
        let original = modules[0].clone();
        assert!(record_module_reference_origins(
            &original.source,
            &mut modules[0].items,
            &sites,
            &original.structs,
            &original.enums,
            original.resolved_assignment_outputs.as_map(),
        )
        .is_empty());
        let ScalarItem::Function(identity) = &mut modules[0].items[0] else {
            panic!("identity")
        };
        identity.name = "renamed".into();
        let untouched = modules[0].clone();
        let views = modules
            .iter()
            .map(CallResolutionModule::module)
            .collect::<Vec<_>>();
        let expected =
            resolve_module_call_sites(&modules[0].source, &modules[0].items, &views).unwrap_err();
        drop(views);
        let calls = resolve_project_call_sites(&modules);
        let before_analysis = modules.clone();
        let complete = analyze_project_references(&modules, calls);
        assert_eq!(modules, before_analysis);
        assert_eq!(complete.errors, vec![expected.clone()]);
        assert_eq!(
            complete
                .updates
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![1]
        );
        let calls = resolve_project_call_sites(&modules);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = record_project_reference_origins(&mut modules, calls);
        assert_eq!(errors, vec![expected]);
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 0,
                reference_origins: 1,
                output_contracts: 0,
                copy_specialization: 0
            }
        );
        assert_eq!(scope.counts().struct_copies, 2);
        assert_eq!(scope.counts().module_copies, 0);
        assert_eq!(scope.counts().cfg_copies, 0);
        drop(stages);
        drop(scope);
        assert_eq!(modules[0], untouched);
        let mut moved = before_analysis;
        assert_eq!(complete.install(&mut moved), errors);
        assert_eq!(moved, modules);
        for item in &modules[1].items {
            let ScalarItem::Function(function) = item else {
                panic!("function")
            };
            assert!(!function.reference_cfg.initial_points.is_empty());
        }
        let mut failed_only = vec![untouched.clone()];
        let calls = resolve_project_call_sites(&failed_only);
        let scope = layout::observations::Scope::new();
        assert_eq!(
            record_project_reference_origins(&mut failed_only, calls).len(),
            1
        );
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        drop(scope);
        assert_eq!(failed_only, vec![untouched]);
    }
}

#[test]
fn recursive_checkpoints_isolate_child_sibling_and_module_errors() {
    let text = "%%start\npick = overload { u8(u8) => fn(input) { next = input; next }; u32(u32) => fn(input) { u32 next = input; next }; };\nu8(u8) broken = fn(input) { missing = input; missing };\nu8(u8) valid = fn(input) { input };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let typed = typecheck_scalar_project(project(vec![
            raw_module("first.w", text, target),
            raw_module("second.w", text, target),
        ]));
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut modules = typed.project.modules;
        for module in &mut modules {
            module.resolved_assignment_outputs = ScalarAssignmentTyping::default();
        }
        let mut expected = Vec::new();
        for module in &modules {
            let ScalarItem::Function(parent) = &module.items[0] else {
                panic!("pick")
            };
            let ScalarItem::Function(broken) = &module.items[1] else {
                panic!("broken")
            };
            for function in [&parent.overload_arms[0], broken] {
                let ScalarBlockItem::Assignment(assignment) = &function.body.items[0] else {
                    panic!("assignment")
                };
                expected.push(ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(
                        module.source.clone(),
                        assignment.targets[0].target_span,
                    ),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                });
            }
        }
        let calls = resolve_project_call_sites(&modules);
        assert_eq!(
            record_project_reference_origins(&mut modules, calls),
            expected
        );
        for module in &modules {
            let ScalarItem::Function(parent) = &module.items[0] else {
                panic!("pick")
            };
            assert!(parent.reference_cfg.initial_points.is_empty());
            assert!(parent.overload_arms[0]
                .reference_cfg
                .initial_points
                .is_empty());
            assert!(!parent.overload_arms[0].reference_cfg.points.is_empty());
            assert!(!parent.overload_arms[1]
                .reference_cfg
                .initial_points
                .is_empty());
            let ScalarItem::Function(valid) = &module.items[2] else {
                panic!("valid")
            };
            assert!(!valid.reference_cfg.initial_points.is_empty());
        }
    }
}

#[test]
fn ordered_project_call_slots_keep_origin_then_lookup_errors_and_partial_attachments() {
    let origin_text = "%%start\ni32(i32) assigned = fn(input) { next = input; next };\n%%end";
    let call_text = "%%start\ni32(i32) identity = fn(input) { input };\ni32(i32) caller = fn(input) { identity(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let raw = project(vec![
            raw_module("a.w", origin_text, target),
            raw_module("b.w", call_text, target),
            raw_module("c.w", call_text, target),
        ]);
        let public_input = raw.clone();
        let typed = typecheck_scalar_project(raw);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut modules = typed.project.modules;
        let b_calls = resolve_project_call_sites(&modules)
            .modules
            .into_iter()
            .nth(1)
            .unwrap()
            .unwrap();
        let b = &mut modules[1];
        assert!(record_module_reference_origins(
            &b.source,
            &mut b.items,
            &b_calls,
            &b.structs,
            &b.enums,
            b.resolved_assignment_outputs.as_map()
        )
        .is_empty());
        let ScalarItem::Function(assigned) = &modules[0].items[0] else {
            panic!("assigned")
        };
        let ScalarBlockItem::Assignment(assignment) = &assigned.body.items[0] else {
            panic!("assignment")
        };
        assert_eq!(modules[0].resolved_assignment_outputs.len(), 1);
        let origin_error = ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(
                modules[0].source.clone(),
                assignment.targets[0].target_span,
            ),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        };
        modules[0].resolved_assignment_outputs = ScalarAssignmentTyping::default();
        let ScalarItem::Function(identity) = &mut modules[1].items[0] else {
            panic!("identity")
        };
        identity.name = "renamed".into();
        let start = call_text.find("identity(input)").unwrap() as u32;
        let lookup_error = ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(
                modules[1].source.clone(),
                ByteSpan::new(start, start + "identity(input)".len() as u32),
            ),
            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
        };
        let before = modules.clone();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let calls = resolve_project_call_sites(&modules);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 3,
                reference_origins: 0,
                output_contracts: 0,
                copy_specialization: 0
            }
        );
        drop(stages);
        drop(scope);
        assert_eq!(modules, before);
        assert_eq!(calls.modules.len(), 3);
        assert!(calls.modules[0].is_ok());
        assert_eq!(calls.modules[1], Err(lookup_error.clone()));
        assert!(calls.modules[2].is_ok());
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = record_project_reference_origins(&mut modules, calls);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 0,
                reference_origins: 2,
                output_contracts: 0,
                copy_specialization: 0
            }
        );
        drop(stages);
        drop(scope);
        let expected = vec![origin_error, lookup_error];
        assert_eq!(errors, expected);
        assert_eq!(
            errors
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>()
        );
        let ScalarItem::Function(partial) = &modules[0].items[0] else {
            panic!("partial")
        };
        assert!(!partial.reference_cfg.points.is_empty());
        assert!(partial.reference_cfg.initial_points.is_empty());
        assert_eq!(modules[1], before[1]);
        for item in &modules[2].items {
            let ScalarItem::Function(function) = item else {
                panic!("valid")
            };
            assert!(!function.reference_cfg.initial_points.is_empty());
        }
        let stages = stage_observations::Scope::new();
        let public = validate_scalar_project(public_input);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 3,
                reference_origins: 3,
                output_contracts: 1,
                copy_specialization: 3
            }
        );
    }
}

#[test]
fn empty_project_calls_have_zero_slots_and_query_enters_only_convergence() {
    let mut modules = Vec::new();
    let scope = layout::observations::Scope::new();
    let stages = stage_observations::Scope::new();
    let calls = resolve_project_call_sites(&modules);
    assert!(calls.modules.is_empty());
    assert_eq!(stages.counts(), stage_observations::Counts::default());
    assert!(record_project_reference_origins(&mut modules, calls).is_empty());
    assert_eq!(
        stages.counts(),
        stage_observations::Counts {
            call_sites: 0,
            reference_origins: 0,
            output_contracts: 1,
            copy_specialization: 0
        }
    );
    assert_eq!(scope.counts(), layout::observations::Counts::default());
    assert!(modules.is_empty());
}

#[test]
fn actual_empty_module_keeps_its_successful_ordinal_slot() {
    let modules = vec![ScalarModule::new(source(), Vec::new(), Vec::new())];
    let calls = resolve_project_call_sites(&modules);
    assert_eq!(calls.modules, vec![Ok(Vec::new())]);
}
