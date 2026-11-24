use super::*;

#[test]
fn standalone_detached_convergence_propagates_checked_returns_without_integral_copies() {
    let text = "%%start\n*u8(*u8) forward = fn(input) { input };\n*u8(*u8) relay = fn(input) { forward(input) };\n*u8(*u8) third = fn(input) { relay(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let call_sites = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let (provisional, errors) = derive_module_reference_origins(
            &program.source, &program.items, &call_sites, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        for (_, result) in &provisional[1..] {
            let call = result.cfg.points.iter().find(|point| matches!(
                &point.kind,
                CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { .. }, output: 0, .. },
            )).expect("unresolved checked call");
            let CfgPointKind::Call { argument_points, .. } = &call.kind else {
                unreachable!()
            };
            assert!(result.cfg.points[argument_points[0].0].possible_origins.as_ref()
                .is_some_and(|origins| !origins.is_empty()));
            assert!(call.possible_origins.is_none());
            assert!(result.cfg.points.iter().find(|point| matches!(
                point.kind, CfgPointKind::ReturnOutput { checked: true, .. },
            )).unwrap().possible_origins.is_none());
        }
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
fn mixed_direct_module_call_marks_only_the_checked_output_in_both_grouped_routes() {
    let text = r#"%%start
(*u8, u8)(*u8) checked_first = fn(input) { input, 3 };
(u8, *u8)(*u8) checked_second = fn(input) { 5, input };
(*u8, u8)(*u8) declare_first = fn(input) {
    *u8 reference, u8 scalar = checked_first(input);
    reference, scalar
};
(u8, *u8)(*u8) declare_second = fn(input) {
    u8 scalar, *u8 reference = checked_second(input);
    scalar, reference
};
(*u8, u8)(*u8) assign_first = fn(input) {
    *u8 reference = null;
    u8 scalar = 0;
    reference, scalar = checked_first(input);
    reference, scalar
};
(u8, *u8)(*u8) assign_second = fn(input) {
    u8 scalar = 0;
    *u8 reference = null;
    scalar, reference = checked_second(input);
    scalar, reference
};
%%end"#;
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let call_sites = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let (provisional, errors) = derive_module_reference_origins(
            &program.source, &program.items, &call_sites, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(provisional.len(), 6);
        for ((_, result), checked_slot) in provisional[2..].iter().zip([0, 1, 0, 1]) {
            let outputs = result.cfg.points.iter().filter_map(|point| match &point.kind {
                CfgPointKind::Call {
                    target: ResolvedCallTarget::ModuleCallable { .. }, output, ..
                } => Some((*output, point)),
                _ => None,
            }).collect::<Vec<_>>();
            assert!(matches!(&outputs[..], [(0, _), (1, _)]));
            let checked = outputs[checked_slot].1;
            assert_eq!(result.proof_points.resolved_checked_call_outputs, [checked.id]);
            assert!(checked.possible_origins.is_none());
        }
        let public = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
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
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
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
            origins: ReferenceOutputOrigins::Explicit(origins),
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
        assert_eq!(left.origins, ReferenceOutputOrigins::Explicit(origins.clone()));
        let (right_other_output, unique) = current.find(
            &program.source, span(1), &ConcreteCallSelection::Declaration, 1,
        ).unwrap();
        assert!(unique);
        assert_eq!(right_other_output.origins, ReferenceOutputOrigins::Explicit(Vec::new()));

        let mut replays = Vec::new();
        let marker = ByteSpan::new(text.len() as u32 + 1, text.len() as u32 + 2);
        let mark_entries = |updates: &mut [(usize, FunctionReferenceResult)]| {
            for (_, result) in updates {
                result.cfg.points[0].source_span.range = marker;
            }
        };
        let assert_replayed = |updates: &[(usize, FunctionReferenceResult)], expected: [bool; 4]| {
            assert_eq!(updates.len(), expected.len());
            for ((_, result), replayed) in updates.iter().zip(expected) {
                let entry = result.cfg.initial_points[0].source_span.range;
                assert_eq!(result.cfg.points[0].source_span.range, if replayed { entry } else { marker });
            }
        };
        let assert_missing_left = |replays: &[ReferenceContractReplay], updates: &[(usize, FunctionReferenceResult)]| {
            assert!(matches!(
                replays[2].errors.as_slice(),
                [ReferenceAnalysisError::MissingContext {
                    reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin, ..
                }]
            ), "{:?}", replays[2].errors);
            let left_call = replays[2].module_calls[0];
            assert!(updates[2].1.cfg.points[left_call.0].possible_origins.is_none());
            assert!(replays[3].errors.is_empty(), "{:?}", replays[3].errors);
        };
        for (step, (from, to)) in [
            (&previous, &previous),
            (&previous, &current),
            (&current, &current),
        ].into_iter().enumerate() {
            mark_entries(&mut updates);
            let mut cursor = 0;
            let mut inputs = HashMap::new();
            let before = updates.iter().map(|(_, result)| result.cfg.points.as_ptr()).collect::<Vec<_>>();
            for (_, result) in &mut updates {
                transfer_function_output_contracts(
                    result, from, to, &mut inputs, &mut replays, &mut cursor,
                );
            }
            assert_eq!(cursor, replays.len());
            if step == 0 {
                assert_missing_left(&replays, &updates);
            } else {
                assert!(replays.iter().all(|replay| replay.errors.is_empty()));
            }
            assert_replayed(&updates, match step {
                0 => [true; 4],
                1 => [false, false, true, false],
                _ => [false; 4],
            });
            assert!(updates.iter().zip(before).all(|((_, result), pointer)| result.cfg.points.as_ptr() == pointer));
        }
        let right_call = replays[3].module_calls[0];
        updates[3].1.aggregate.checked_successor_candidate = Some(right_call);
        let metadata_only = ReferenceOutputContracts::new(vec![
            row(span(0), 0, origins.clone()),
            row(span(0), 0, Vec::new()),
            ReferenceOutputContract {
                independent_static: false,
                ..row(span(1), 0, origins.clone())
            },
            row(span(1), 1, Vec::new()),
        ]);
        let removed = ReferenceOutputContracts::new(vec![
            row(span(1), 0, origins.clone()),
            row(span(1), 1, Vec::new()),
        ]);
        for (from, to, successor) in [
            (&current, &current, true),
            (&current, &metadata_only, false),
            (&metadata_only, &removed, true),
        ] {
            mark_entries(&mut updates);
            let mut cursor = 0;
            let mut inputs = HashMap::new();
            let left_before = updates[2].1.cfg.points.as_ptr();
            let right_before = updates[3].1.cfg.points.as_ptr();
            for (_, result) in &mut updates {
                transfer_function_output_contracts(
                    result, from, to, &mut inputs, &mut replays, &mut cursor,
                );
            }
            assert_eq!(cursor, replays.len());
            assert_eq!(updates[3].1.cfg.points.as_ptr(), right_before);
            assert_eq!(updates[3].1.aggregate.checked_successor_contract, successor);
            if std::ptr::eq(to, &removed) {
                assert_missing_left(&replays, &updates);
            } else {
                assert!(replays.iter().all(|replay| replay.errors.is_empty()));
            }
            assert_eq!(updates[2].1.cfg.points.as_ptr(), left_before);
            assert_replayed(&updates, [false, false, std::ptr::eq(to, &removed), false]);
        }
    }
}

#[test]
fn discarded_checked_indirect_call_does_not_take_its_checked_argument_origin() {
    let text = r#"%%start
struct Reader { *u8(*u8) pick; }
*u8(*u8) forward = fn(input) { input };
unit((*u8(*u8)), *u8) inspect = fn(callback, input) {
    Reader holder = { .pick = forward; };
    callback(input);
    holder.pick(input);
};
%%end"#;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(declarations, layout);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let (results, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let result = &results[1].1;
        let points = &result.cfg.points;
        let indirect = points.iter().filter_map(|point| match &point.kind {
            CfgPointKind::Call {
                target: ResolvedCallTarget::LocalCallable(_),
                output: 0, argument_points, ..
            } => Some((false, point, argument_points)),
            CfgPointKind::Call {
                target: ResolvedCallTarget::CallableField { .. },
                output: 0, argument_points, ..
            } => Some((true, point, argument_points)),
            _ => None,
        }).collect::<Vec<_>>();
        assert!(matches!(indirect.as_slice(), [(false, _, _), (true, _, _)]));
        let exact_outputs = indirect.iter().map(|(_, point, _)| point.id).collect::<Vec<_>>();
        assert_eq!(result.proof_points.resolved_checked_call_outputs, exact_outputs);
        for (_, result, arguments) in indirect {
            let [argument] = arguments.as_slice() else { panic!("one checked argument") };
            assert!(points[argument.0].possible_origins.as_ref().is_some_and(|origins| !origins.is_empty()));
            assert!(result.possible_origins.is_none());
        }
        let public = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
    }
}
