use super::*;

#[test]
fn standalone_detached_convergence_propagates_checked_returns_without_integral_copies() {
    let text = "%%start\n*u8(*u8) forward = fn(input) { input };\n*u8(*u8) relay = fn(input) { forward(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let call_sites = resolve_module_call_sites(
            &program.source,
            &program.items,
            std::slice::from_ref(&module),
        )
        .unwrap();
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = record_program_reference_origins(&mut program, &call_sites);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 0,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 1
            }
        );
        drop(stages);
        drop(scope);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(program, public.program);
        for item in &program.items {
            let ScalarItem::Function(function) = item else {
                panic!("checked function")
            };
            let returns = function
                .reference_cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { output: 0, .. }))
                .collect::<Vec<_>>();
            assert_eq!(returns.len(), 1);
            let origins = returns[0]
                .possible_origins
                .as_ref()
                .expect("propagated checked return");
            assert_eq!(origins.len(), 1);
            let origin = &function.reference_cfg.facts[origins.first().unwrap().0];
            let ReferenceOrigin::BorrowedFrom { parameter, loan } = origin else {
                panic!("parameter origin: {origin:?}")
            };
            assert_eq!(*parameter, 0);
            assert_eq!(loan.origin_place.source, source());
            assert_eq!(loan.origin_place.function_span, function.span);
            assert_eq!(
                loan.origin_place.declaration_span,
                function.parameter_spans[0]
            );
            assert_eq!(
                loan.origin_place.binding.declaration_span,
                function.parameter_spans[0]
            );
        }
    }
}

#[test]
fn standalone_detached_failure_installs_exact_partial_cfg_and_ownership_error() {
    let text = "%%start\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let call_sites = resolve_module_call_sites(
            &program.source,
            &program.items,
            std::slice::from_ref(&module),
        )
        .unwrap();
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let errors = record_program_reference_origins(&mut program, &call_sites);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(
            stages.counts(),
            stage_observations::Counts {
                call_sites: 0,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 0
            }
        );
        drop(stages);
        drop(scope);
        let unavailable = text.rfind("next };").unwrap() as u32;
        let moved = text.find("next; next").unwrap() as u32;
        assert_eq!(
            errors,
            vec![ReferenceAnalysisError::ConflictingPlace {
                source_span: SourceSpan::new(source(), ByteSpan::new(unavailable, unavailable + 4)),
                origins: vec![SourceSpan::new(source(), ByteSpan::new(moved, moved + 4))],
            }]
        );
        assert_eq!(
            public.diagnostics,
            errors
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>()
        );
        assert_eq!(program, public.program);
        let ScalarItem::Function(function) = &program.items[0] else {
            panic!("consume")
        };
        assert!(!function.reference_cfg.points.is_empty());
        assert!(!function.reference_cfg.initial_points.is_empty());
    }
}
