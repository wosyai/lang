use super::*;

#[test]
fn standalone_detached_convergence_propagates_checked_returns_without_integral_copies() {
    let text = "%%start\n*u8(*u8) forward = fn(input) { input };\n*u8(*u8) relay = fn(input) { forward(input) };\n*u8(*u8) third = fn(input) { relay(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let call_sites = resolve_program_call_sites(&program).unwrap();
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
        let call_sites = resolve_program_call_sites(&program).unwrap();
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

#[test]
fn independent_contract_replay_retains_first_cfg_and_ordered_errors_without_replay() {
    let text = "%%start\nstruct Payload { u8 value; }\nPayload(Payload) left = fn(value) { next = value; next; next };\nPayload(Payload) right = fn(value) { next = value; next; next };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites(&program).unwrap();
        let (mut updates, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let mut replays = Vec::new();
        let contracts = ReferenceOutputContracts::default();
        for initial in [true, false] {
            let mut cursor = 0;
            let mut inputs = HashMap::new();
            let before = updates.iter().map(|(_, result)| result.cfg.points.as_ptr()).collect::<Vec<_>>();
            let before_errors = replays.iter().map(|replay: &ReferenceContractReplay| replay.errors.as_ptr()).collect::<Vec<_>>();
            for (_, result) in &mut updates {
                transfer_function_output_contracts(
                    result, &contracts, &contracts, &mut inputs, &mut replays, &mut cursor,
                );
                assert!(inputs.is_empty());
            }
            assert_eq!(cursor, replays.len());
            assert_eq!(replays.len(), 2);
            assert!(replays.iter().all(|replay| replay.module_calls.is_empty() && replay.errors.len() == 1));
            if !initial {
                for ((_, result), pointer) in updates.iter().zip(before) {
                    assert_eq!(result.cfg.points.as_ptr(), pointer);
                }
                for (replay, pointer) in replays.iter().zip(before_errors) {
                    assert_eq!(replay.errors.as_ptr(), pointer);
                }
            }
        }
        let starts = replays.into_iter().flat_map(|replay| replay.errors).map(|error| {
            let ReferenceAnalysisError::ConflictingPlace { source_span, .. } = error else {
                panic!("expected unavailable place")
            };
            source_span.range.start
        }).collect::<Vec<_>>();
        assert!(starts[0] < starts[1], "{starts:?}");
    }
}

#[test]
fn replay_observes_only_consumed_first_output_contract_origins() {
    let text = "%%start\n*u8(*u8) left = fn(input) { input };\n*u8(*u8) right = fn(input) { input };\n*u8(*u8) left_relay = fn(input) { left(input) };\n*u8(*u8) right_relay = fn(input) { right(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites(&program).unwrap();
        let (mut updates, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let span = |index| match &program.items[index] {
            ScalarItem::Function(function) => function.name_span,
            _ => panic!("expected callable"),
        };
        let origins = vec![ReferenceOrigin::Null { span: ByteSpan::new(0, 1) }];
        let row = |declaration_span, output, origins| ReferenceOutputContract {
            source: &program.source,
            declaration_span,
            body: ReferenceCallableBody::Declaration,
            output,
            origins,
            independent_static: true,
        };
        let previous = ReferenceOutputContracts::new(vec![
            row(span(0), 0, Vec::new()),
            row(span(1), 0, origins.clone()),
        ]);
        let current = ReferenceOutputContracts::new(vec![
            row(span(0), 0, origins.clone()),
            row(span(0), 0, Vec::new()),
            row(span(1), 0, origins.clone()),
            row(span(1), 1, Vec::new()),
        ]);
        let (left, unique) = current.find(
            &program.source, span(0), &ConcreteCallSelection::Declaration, 0,
        ).unwrap();
        assert!(!unique);
        assert_eq!(left.origins, origins);
        let (right_other_output, unique) = current.find(
            &program.source, span(1), &ConcreteCallSelection::Declaration, 1,
        ).unwrap();
        assert!(unique);
        assert!(right_other_output.origins.is_empty());

        let mut replays = Vec::new();
        let mut pointers = Vec::new();
        for (from, to) in [
            (&previous, &previous),
            (&previous, &current),
            (&current, &current),
        ] {
            let mut cursor = 0;
            let mut inputs = HashMap::new();
            for (_, result) in &mut updates {
                transfer_function_output_contracts(
                    result, from, to, &mut inputs, &mut replays, &mut cursor,
                );
            }
            assert_eq!(cursor, replays.len());
            pointers.push(updates.iter().map(|(_, result)| result.cfg.points.as_ptr()).collect::<Vec<_>>());
        }
        assert_ne!(pointers[0][2], pointers[1][2]);
        for index in [0, 1, 3] {
            assert_eq!(pointers[0][index], pointers[1][index]);
        }
        assert_eq!(pointers[1], pointers[2]);
        assert!(replays.iter().all(|replay| replay.errors.is_empty()));
    }
}
