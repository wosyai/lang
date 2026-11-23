use super::*;

#[test]
fn equal_call_spans_in_distinct_arms_keep_callable_owned_targets_without_resolution_reentry() {
    let text = "%%start\nu8 padding = 0;\nu8(u8) narrow = fn(input) { input };\nu32(u32) wide = fn(input) { input };\npick = overload { u8(u8) => fn(input) { narrow(input) }; u32(u32) => fn(input) { wide(input) }; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let validated = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(
            validated.diagnostics.is_empty(),
            "{:?}",
            validated.diagnostics
        );
        let mut program = validated.program;
        let expected = [1, 2].map(|index| {
            let ScalarItem::Function(function) = &program.items[index] else {
                panic!("target function")
            };
            ResolvedCallTarget::ModuleCallable {
                source: program.source.clone(),
                declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Declaration,
            }
        });
        let ScalarItem::Function(parent) = &mut program.items[3] else {
            panic!("overload")
        };
        let shared = expression_span(&parent.overload_arms[0].body.expressions[0]);
        let original_metadata = parent
            .overload_arms
            .iter()
            .map(|arm| (arm.span, arm.name_span, arm.parameter_spans.clone()))
            .collect::<Vec<_>>();
        let second = &mut parent.overload_arms[1].body;
        for expression in &mut second.expressions {
            let ScalarExpression::Call { span, .. } = expression else {
                panic!("arm call")
            };
            *span = shared;
        }
        for item in &mut second.items {
            let ScalarBlockItem::Expression(ScalarExpression::Call { span, .. }) = item else {
                panic!("arm call item")
            };
            *span = shared;
        }
        for output in &mut second.final_output_values {
            let ScalarExpression::Call { span, .. } = &mut output.value else {
                panic!("arm call output")
            };
            *span = shared;
        }
        assert_eq!(
            parent
                .overload_arms
                .iter()
                .map(|arm| (arm.span, arm.name_span, arm.parameter_spans.clone()))
                .collect::<Vec<_>>(),
            original_metadata
        );
        let before = program.clone();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let calls = resolve_program_call_sites(&program).unwrap();
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 1);
        drop(stages);
        drop(scope);
        assert_eq!(
            calls.functions.keys().copied().collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let parent_calls = &calls.functions[&3];
        assert!(parent_calls.sites.is_empty());
        assert_eq!(parent_calls.arms.len(), 2);
        for (arm, expected) in parent_calls.arms.iter().zip(&expected) {
            assert_eq!(
                arm.sites,
                vec![ResolvedCallSite {
                    source_span: SourceSpan::new(source(), shared),
                    target: expected.clone()
                }]
            );
        }
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let result = analyze_program_references(&program, &calls);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 0);
        drop(stages);
        drop(scope);
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let project_calls = ProjectCallSiteResults {
            duplicate_sources: HashSet::new(),
            modules: vec![Ok(calls)],
        };
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let project = analyze_project_references(&[module], &project_calls);
        assert!(project.errors.is_empty(), "{:?}", project.errors);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 0);
        drop(stages);
        drop(scope);
        for results in [&result.updates, &project.updates[0].1] {
            let (_, parent) = results.iter().find(|(index, _)| *index == 3).unwrap();
            for (arm, expected) in parent.arms.iter().zip(&expected) {
                let targets = arm
                    .cfg
                    .points
                    .iter()
                    .filter_map(|point| match &point.kind {
                        CfgPointKind::Call { target, .. } => {
                            assert_eq!(point.source_span.range, shared);
                            Some(target)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(targets, vec![expected]);
            }
        }
        assert_eq!(program, before);
    }
}

#[test]
fn standalone_call_lookup_borrows_actual_inputs_and_preserves_complete_indirect_targets() {
    let text = "%%start\nstruct Pool { unit(*?u8) free; }\nunit(*?u8) release = fn(storage) { storage; };\nunit((unit(*?u8))) invoke = fn(free) { Pool heap = { .free = release; }; unsafe { *?u8 storage = core.alloc(1, 1); free(storage); heap.free(storage); core.free(storage); } };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let before = program.clone();
        let view = CallResolutionModule::program(&program);
        assert!(std::ptr::eq(view.source, &program.source));
        assert!(!view.items.is_empty());
        assert!(std::ptr::eq(view.items, program.items.as_slice()));
        assert!(!view.structs.is_empty());
        assert!(std::ptr::eq(view.structs, program.structs.as_slice()));
        assert!(view.namespace_bindings.is_empty());
        let ScalarItem::Function(function) = &program.items[1] else {
            panic!("invoke")
        };
        let binding = ReferenceBindingId {
            source: source(),
            function_span: function.span,
            declaration_span: function.parameter_spans[0],
            block_span: function.body.span,
            kind: ReferenceBindingKind::Declared,
        };
        let heap = text.find("heap =").unwrap() as u32;
        let heap_span = ByteSpan::new(heap, heap + 4);
        let field_call = text.find("heap.free(storage)").unwrap() as u32;
        let expected = [
            (
                "core.alloc(1, 1)",
                ResolvedCallTarget::Core(CoreOperationId::Alloc),
            ),
            ("free(storage)", ResolvedCallTarget::LocalCallable(binding)),
            (
                "heap.free(storage)",
                ResolvedCallTarget::CallableField {
                    receiver_place: ReferencePlaceId {
                        source: source(),
                        function_span: function.span,
                        declaration_span: heap_span,
                        block_span: function.body.span,
                        binding: ReferenceBindingId {
                            source: source(),
                            function_span: function.span,
                            declaration_span: heap_span,
                            block_span: function.body.span,
                            kind: ReferenceBindingKind::Declared,
                        },
                        projections: Vec::new(),
                        place: ScalarPlace::Name {
                            name: "heap".into(),
                            span: ByteSpan::new(field_call, field_call + 4),
                        },
                    },
                    field: program.structs[0].fields[0].id.clone(),
                },
            ),
            (
                "core.free(storage)",
                ResolvedCallTarget::Core(CoreOperationId::Free),
            ),
        ]
        .into_iter()
        .map(|(spelling, target)| {
            let start = text.find(spelling).unwrap() as u32;
            ResolvedCallSite {
                source_span: SourceSpan::new(
                    source(),
                    ByteSpan::new(start, start + spelling.len() as u32),
                ),
                target,
            }
        })
        .collect::<Vec<_>>();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let actual = resolve_program_call_sites(&program).unwrap();
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 1,
                reference_origins: 0,
                output_contracts: 0,
                copy_specialization: 0
            }
        );
        drop(stages);
        drop(scope);
        assert_eq!(actual.functions[&1].sites, expected);
        assert_eq!(program, before);
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        let ScalarItem::Function(completed) = &public.program.items[1] else {
            panic!("invoke")
        };
        let public_sites = completed
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Call {
                    output: 0, target, ..
                } => Some(ResolvedCallSite {
                    source_span: point.source_span.clone(),
                    target: target.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(actual.functions[&1].sites, public_sites);
        let namespace = ScalarNamespaceBinding {
            binding: "peer".into(),
            target: source(),
            span: ByteSpan::new(0, 0),
        };
        let module = ScalarModule::from_program(program, vec![namespace]);
        let module_view = CallResolutionModule::module(&module);
        assert!(std::ptr::eq(module_view.source, &module.source));
        assert!(std::ptr::eq(module_view.items, module.items.as_slice()));
        assert!(std::ptr::eq(module_view.structs, module.structs.as_slice()));
        assert!(!module_view.namespace_bindings.is_empty());
        assert!(std::ptr::eq(
            module_view.namespace_bindings,
            module.namespace_bindings.as_slice()
        ));
        let mut later = module.clone();
        later.structs[0].fields[0].id.index = 99;
        let views = [
            CallResolutionModule::module(&module),
            CallResolutionModule::module(&later),
        ];
        assert_eq!(
            resolve_module_call_sites(
                &module.source,
                &module.items,
                &module.namespace_bindings,
                &CallDeclarationIndex::new(&views),
                module.resolved_assignment_outputs.as_map(),
                false,
                PreparationOccurrenceTyping::default(),
            )
            .unwrap()
            .functions[&1]
                .sites,
            expected
        );
    }
}

#[test]
fn same_callable_equal_call_spans_with_different_targets_keep_exact_diagnostic() {
    let text = "%%start\nbool(bool) first = fn(flag) { flag };\nbool(bool) second = fn(flag) { flag };\nunit() probe = fn() { first(true); second(true); };\n%%end";
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let typed = typecheck_scalar_program(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            target,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let ScalarItem::Function(probe) = &mut program.items[2] else {
            panic!("probe")
        };
        let [ScalarBlockItem::Expression(ScalarExpression::Call { span: first, .. }), ScalarBlockItem::Expression(ScalarExpression::Call { span: second, .. })] =
            probe.body.items.as_mut_slice()
        else {
            panic!("two independent calls")
        };
        let span = *first;
        assert_ne!(span, *second);
        *second = span;

        assert_eq!(
            resolve_program_call_sites(&program).unwrap_err(),
            ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(program.source.clone(), span),
                reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
            }
        );
    }
}

#[test]
fn standalone_lookup_failure_keeps_exact_spanned_invariant_diagnostic() {
    let text = "%%start\ni32(i32) identity = fn(input) { input };\ni32(i32) caller = fn(input) { identity(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let typed = typecheck_scalar_program(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            target,
        );
        assert!(typed.diagnostics.is_empty());
        let mut program = typed.program;
        let ScalarItem::Function(identity) = &mut program.items[0] else {
            panic!("identity")
        };
        identity.name = "renamed".into();
        let before = program.clone();
        let start = text.find("identity(input)").unwrap() as u32;
        let expected = ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(
                source(),
                ByteSpan::new(start, start + "identity(input)".len() as u32),
            ),
            reason: ReferenceAnalysisErrorReason::MissingResolvedCallTarget,
        };
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let error = resolve_program_call_sites(&program).unwrap_err();
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 1);
        drop(stages);
        drop(scope);
        assert_eq!(error, expected);
        assert_eq!(error.diagnostic(), expected.diagnostic());
        assert_eq!(program, before);
    }
}
