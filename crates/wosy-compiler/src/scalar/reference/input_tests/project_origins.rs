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
        let (mut updates, errors, _) = derive_project_reference_origins(&derived.modules, &calls);
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
        let sites = resolve_module_call_sites(
            &modules[0].source,
            &modules[0].items,
            &modules[0].namespace_bindings,
            &views,
            modules[0].resolved_assignment_outputs.as_map(),
        )
        .unwrap();
        drop(views);
        let original = modules[0].clone();
        assert!(record_module_reference_origins(
            &original.source,
            &mut modules[0].items,
            &sites,
            &original.structs,
            &original.enums,
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
        let expected = resolve_module_call_sites(
            &modules[0].source,
            &modules[0].items,
            &modules[0].namespace_bindings,
            &views,
            modules[0].resolved_assignment_outputs.as_map(),
        )
        .unwrap_err();
        drop(views);
        let calls = resolve_project_call_sites(&modules);
        let before_analysis = modules.clone();
        let before_calls = calls.modules.clone();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let complete = analyze_project_references(&modules, &calls);
        let counts = scope.counts();
        let stage_counts = stages.counts();
        assert_eq!(stage_counts.call_sites, 0);
        drop(stages);
        drop(scope);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let repeated = analyze_project_references(&modules, &calls);
        assert_eq!(scope.counts(), counts);
        assert_eq!(stages.counts(), stage_counts);
        drop(stages);
        drop(scope);
        assert_project_reference_results(&repeated, &complete);
        assert_eq!(calls.modules, before_calls);
        let mut repeated_install = before_analysis.clone();
        repeated.install(&mut repeated_install);
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
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = record_project_reference_origins(&mut modules, &calls);
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
        assert_eq!(repeated_install, modules);
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
            record_project_reference_origins(&mut failed_only, &calls).len(),
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
            record_project_reference_origins(&mut modules, &calls),
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
            &b.enums
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
        let errors = record_project_reference_origins(&mut modules, &calls);
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
    assert!(record_project_reference_origins(&mut modules, &calls).is_empty());
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
    assert_eq!(
        calls.modules,
        vec![Ok(ModuleCallSites {
            functions: BTreeMap::new(),
            initialization: ModuleInitializationRegion {
                source: modules[0].source.clone(),
                steps: Vec::new(),
                detail: ModuleInitializationDetail::Occurrences(Box::new(
                    ModuleInitializationTopology {
                        structure: OccurrenceStructure::new(ByteSpan::new(0, 0)),
                        occurrences: Vec::new(),
                        calls: Vec::new(),
                        assignments: HashMap::new(),
                        conditionals: HashMap::new(),
                        loops: HashMap::new(),
                        short_circuits: HashMap::new(),
                    }
                )),
            },
        })]
    );
}

#[test]
fn standalone_and_project_publish_source_ordered_module_initialization_dispatch() {
    use ModuleInitializationStepKind as Step;

    let text = "%%start\nchild = namespace package \"child.w\";\nenv = extern wasm \"helper\" { unit() flush; };\ni32 value = 1;\ni32(i32) identity = fn(input) { input };\nvalue;\n%%end";
    let target = ScalarTargetLayout::WASM32;
    let mut root = raw_module("root.w", text, target);
    let child = raw_module("child.w", text, target);
    let parsed = parse_source(root.source.clone(), text.into(), &[]);
    let typed = typecheck_scalar_program(
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
        target,
    );
    assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
    let standalone = resolve_program_call_sites(&typed.program).unwrap();

    let ScalarItem::Namespace(namespace) = &root.items[0] else {
        panic!("namespace item")
    };
    let binding_span = namespace.binding_span;
    assert_eq!(
        &text[binding_span.start as usize..binding_span.end as usize],
        "child"
    );
    root.namespace_bindings.push(ScalarNamespaceBinding {
        binding: namespace.binding.clone(),
        span: namespace.span,
        target: child.source.clone(),
    });
    let typed_project = typecheck_scalar_project(project(vec![root, child]));
    assert!(
        typed_project.diagnostics.is_empty(),
        "{:?}",
        typed_project.diagnostics
    );
    let modules = &typed_project.project.modules;
    let project_calls = resolve_project_call_sites(modules);
    for (calls, source, target) in [
        (&standalone, &typed.program.source, NamespaceTarget::Unbound),
        (
            project_calls.modules[0].as_ref().unwrap(),
            &modules[0].source,
            NamespaceTarget::Bound(modules[1].source.clone()),
        ),
        (
            project_calls.modules[1].as_ref().unwrap(),
            &modules[1].source,
            NamespaceTarget::Unbound,
        ),
    ] {
        let region = &calls.initialization;
        assert_eq!(&region.source, source);
        assert_eq!(
            region
                .steps
                .iter()
                .map(|step| step.item)
                .collect::<Vec<_>>(),
            [0, 2, 4]
        );
        assert_eq!(
            region.steps[0].kind,
            Step::NamespaceDispatch {
                binding: "child".into(),
                binding_span,
                target,
            }
        );
        assert_eq!(region.steps[1].kind, Step::BindingInitialization);
        assert_eq!(region.steps[2].kind, Step::ExecutableItem);
    }
}

#[test]
fn prepared_module_initialization_publishes_nested_canonical_occurrences() {
    use OccurrenceEdge as Edge;
    use OccurrenceNode as Node;
    use OccurrenceParent as Parent;

    let text = "%%start\nfirst = namespace package \"child.w\";\ni32(i32) identity = fn(input) { input };\nsecond = namespace package \"child.w\";\ni32 value = if (true && second.echo(false)) { if (false) { 1 } else { 2 } } else { 3 };\nif (true) { while (false) { value = identity(4); } };\n%%end";
    let source = |path: &str| {
        SourceIdentity::new("project".into(), "package".into(), path.into(), "r1".into())
    };
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let root_source = source("root.w");
        let child_text = "%%start\nbool(bool) echo = fn(flag) { flag };\n%%end";
        let child_source = source("child.w");
        let root = parse_source(root_source.clone(), text.into(), &[]);
        let child = parse_source(child_source.clone(), child_text.into(), &[]);
        assert!(root.diagnostics.is_empty(), "{:?}", root.diagnostics);
        assert!(child.diagnostics.is_empty(), "{:?}", child.diagnostics);
        let root = derive_scalar_declarations_from_cst(root.result.canonical_cst());
        let child = derive_scalar_declarations_from_cst(child.result.canonical_cst());
        let bindings = root
            .namespaces()
            .map(|namespace| ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                span: namespace.span,
                target: child_source.clone(),
            })
            .collect();
        let stages = stage_observations::Scope::new();
        let prepared = crate::prepare_scalar_project(
            vec![(root, bindings), (child, Vec::new())],
            vec![root_source.clone(), child_source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 2,
                ..Default::default()
            }
        );
        let region = prepared
            .initialization_region(0)
            .expect("root initialization");
        assert_eq!(region.source, root_source);
        assert_eq!(
            prepared.initialization_region(1).unwrap().source,
            child_source
        );
        assert_eq!(
            region
                .steps
                .iter()
                .map(|step| step.item)
                .collect::<Vec<_>>(),
            [0, 2, 3, 4]
        );
        let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
            panic!("prepared initialization topology");
        };
        let structure = &topology.structure;
        let root = Parent::Block(structure.root);
        let root_items = &structure.blocks[structure.root.0].items;
        assert_eq!(
            root_items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            [3, 4]
        );
        assert!(structure.edges.keys().all(|(parent, edge)| *parent != root
            || matches!(
                edge,
                Edge::BindingRoot { item: 3, output: 0 } | Edge::ExpressionItem { item: 4 }
            )));
        let spelling = |ordinal: OccurrenceOrdinal| {
            let span = topology.occurrences[ordinal.0].span;
            &text[span.start as usize..span.end as usize]
        };
        let BlockItemKind::Binding(binding) = &root_items[0].kind else {
            panic!("root binding");
        };
        assert_eq!(
            structure.edges[&(root, Edge::BindingRoot { item: 3, output: 0 })],
            Node::Expression(binding.primary)
        );
        assert_eq!(binding.receivers()[0].name, "value");
        let ConditionalRegion::Value {
            condition,
            then_block,
            else_block,
            ..
        } = topology.conditionals[&binding.primary]
        else {
            panic!("initializer conditional");
        };
        let short = topology.short_circuits[&condition];
        assert!(short.evaluate_right_when);
        assert!(!short.bypass);
        assert_eq!(short.join, condition);
        assert_eq!(spelling(short.left), "true");
        assert_eq!(spelling(short.right), "second.echo(false)");
        let Node::Expression(argument) =
            structure.edges[&(Parent::Expression(short.right), Edge::CallArgument(0))]
        else {
            panic!("qualified call argument");
        };
        assert_eq!(spelling(argument), "false");
        let final_output =
            |block: BlockOrdinal| *structure.blocks[block.0].final_outputs().next().unwrap();
        let nested = final_output(then_block);
        assert_eq!((nested.item, nested.output), (0, 0));
        assert_eq!(nested.span, topology.occurrences[nested.expression.0].span);
        let ConditionalRegion::Value {
            then_block,
            else_block: nested_else,
            ..
        } = topology.conditionals[&nested.expression]
        else {
            panic!("nested conditional");
        };
        assert_eq!(spelling(final_output(then_block).expression), "1");
        assert_eq!(spelling(final_output(nested_else).expression), "2");
        assert_eq!(spelling(final_output(else_block).expression), "3");

        let Node::Expression(executable) =
            structure.edges[&(root, Edge::ExpressionItem { item: 4 })]
        else {
            panic!("root executable");
        };
        let ConditionalRegion::Unit { then_block, .. } = topology.conditionals[&executable] else {
            panic!("executable conditional");
        };
        let loop_region = topology.loops[&(then_block, 0)];
        assert_eq!(
            loop_region.body_to_test(),
            (loop_region.body, loop_region.test)
        );
        assert_eq!(spelling(loop_region.test), "false");
        let group = &topology.assignments[&(loop_region.body, 0)];
        let rhs = group.rhs_evaluations().next().unwrap();
        let destination = group.destinations().next().unwrap();
        assert_eq!(spelling(rhs.expression), "identity(4)");
        assert_eq!(spelling(destination.destination), "value");
        assert_eq!(
            topology.occurrences[destination.destination.0].span,
            destination.target_span
        );
        assert!(rhs.expression.0 < destination.destination.0);
        assert_eq!(
            destination.replacement,
            AssignmentReplacement::StructuralOnly
        );
        assert_eq!(group.previous_observations().next(), Some(destination));
        assert_eq!(group.installs().next(), Some(destination));
        assert_eq!(
            group.continuation(),
            BlockPosition {
                block: loop_region.body,
                item: 1
            }
        );
        assert_eq!(topology.calls, [short.right, rhs.expression]);
    }
}
