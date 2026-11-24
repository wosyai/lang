use super::*;

#[test]
fn binding_output_sources_borrow_canonical_roots_and_project_precise_primary_ordinals() {
    let text = "%%start\nu8(u8) first = fn(input) { input };\nu8(u8) second = fn(input) { input };\n(u8, u8, u8)(u8) triple = fn(input) { input, input, input };\nunit(u8) probe = fn(input) { u8 left, u8 right = first(input) + input, second(input) + input; left; right; u8 a, u8 b, u8 c = triple(input); a; b; c; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let validation = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let mut program = validation.program;
        let expected = [0, 1, 2].map(|index| {
            let ScalarItem::Function(function) = &program.items[index] else {
                panic!("callee")
            };
            ResolvedCallTarget::ModuleCallable {
                source: program.source.clone(),
                declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Declaration,
            }
        });
        let ScalarItem::Function(probe) = &mut program.items[3] else {
            panic!("probe")
        };
        let mut bindings = probe.body.items.iter_mut().filter_map(|item| match item {
            ScalarBlockItem::LocalBinding(binding) => Some(binding),
            _ => None,
        });
        let independent = bindings.next().unwrap();
        assert_eq!(
            independent.output_origin,
            ScalarBindingOutputOrigin::IndependentExpressions
        );
        assert_eq!(independent.output_values.len(), 2);
        assert_eq!(independent.receivers.len(), 2);
        let shared = expression_span(&independent.value);
        let ScalarExpression::Binary { span, .. } = &mut independent.output_values[1].value else {
            panic!("second independent root")
        };
        *span = shared;
        for position in 0..2 {
            let ScalarBindingOutput::Evaluate(value) = independent.output_source(position) else {
                panic!("independent evaluation")
            };
            let canonical = if position == 0 {
                &independent.value
            } else {
                &independent.output_values[position].value
            };
            assert!(std::ptr::eq(value, canonical));
            assert_eq!(expression_span(value), shared);
        }
        let ScalarBindingOutput::Evaluate(primary) = independent.output_source(0) else {
            unreachable!()
        };
        let ScalarBindingOutput::Evaluate(secondary) = independent.output_source(1) else {
            unreachable!()
        };
        assert!(!std::ptr::eq(primary, secondary));
        assert!(!std::ptr::eq(primary, &independent.output_values[0].value));
        let single = bindings.next().unwrap();
        assert_eq!(
            single.output_origin,
            ScalarBindingOutputOrigin::SingleExpression
        );
        assert_eq!(single.receivers.len(), 3);
        let ScalarBindingOutput::Evaluate(primary) = single.output_source(0) else {
            panic!("primary evaluation")
        };
        assert!(std::ptr::eq(primary, &single.value));
        for position in 1..3 {
            let ScalarBindingOutput::ProjectPrimary { output } = single.output_source(position)
            else {
                panic!("primary projection")
            };
            assert_eq!(output, position);
        }
        assert!(bindings.next().is_none());
        let before = program.clone();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let calls = resolve_program_call_sites(&program).unwrap();
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 1);
        drop(stages);
        drop(scope);
        let sites = &calls.functions[&3].sites;
        assert_eq!(sites.len(), 3);
        for (site, expected) in sites.iter().zip(&expected) {
            assert_eq!(&site.target, expected);
        }
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let result = analyze_program_references(&program, &calls);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 0);
        drop(stages);
        drop(scope);
        let (_, probe) = result
            .updates
            .iter()
            .find(|(index, _)| *index == 3)
            .unwrap();
        let calls = probe
            .cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Call { output, target, .. } => Some((*output, target)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0], (0, &expected[0]));
        assert_eq!(calls[1], (0, &expected[1]));
        for (position, call) in calls[2..].iter().enumerate() {
            assert_eq!(*call, (position, &expected[2]));
        }
        assert_eq!(program, before);
    }
}

#[test]
fn typed_binding_receiver_slots_preserve_mixed_output_order_and_drive_reference_analysis() {
    let text = "%%start\nstruct Payload { u8 value; }\n(bool, Payload)(Payload) pair = fn(input) { true, input };\nunit(Payload, Payload) probe = fn(left_input, right_input) { Payload left, bool valid = left_input, false; bool done, Payload right = pair(right_input); left; valid; done; right; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let calls = resolve_program_call_sites(&program).unwrap();
        let row = calls.function(1);
        let block = &row.structure.blocks[row.structure.root.0];
        let payload = &row.boundary.parameters[0].ty;
        assert!(matches!(payload, ScalarType::Struct(_)));
        let ScalarItem::Function(probe) = &mut program.items[1] else {
            panic!("probe")
        };
        let mut spans = Vec::new();
        for (item, expected) in [
            [("left", payload), ("valid", &ScalarType::Bool)],
            [("done", &ScalarType::Bool), ("right", payload)],
        ]
        .into_iter()
        .enumerate()
        {
            let ScalarBlockItem::LocalBinding(binding) = &probe.body.items[item] else {
                panic!("local binding")
            };
            let BlockItemKind::Binding(region) = &block.items[item].kind else {
                panic!("binding region")
            };
            let slots = region.receivers();
            assert_eq!(slots.len(), expected.len());
            for (position, (slot, (name, ty))) in slots.iter().zip(expected).enumerate() {
                assert_eq!(slot.position, position);
                assert_eq!(slot.name, name);
                assert_eq!(&slot.ty, ty);
                let span = if position == 0 {
                    binding.name_span
                } else {
                    binding.receivers[position].name_span
                };
                assert_eq!(slot.span, span);
                spans.push(span);
            }
        }

        let module_names = HashSet::from(["pair".to_owned(), "probe".to_owned()]);
        let (expected, errors) = derive_function_reference_cfg(
            &program.source,
            probe,
            &module_names,
            row,
            &program.structs,
            &program.enums,
            true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let binds = expected
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Bind { binding } => Some(binding),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(binds.len(), spans.len());
        for (binding, span) in binds.iter().zip(&spans) {
            assert_eq!(binding.source, program.source);
            assert_eq!(binding.function_span, probe.span);
            assert_eq!(binding.block_span, block.span);
            assert_eq!(binding.declaration_span, *span);
        }
        let moves = expected
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Move { owner, ty, .. } if spans.contains(&owner.declaration_span) => {
                    Some((owner.declaration_span, ty))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(moves, [(spans[0], payload), (spans[3], payload)]);

        for item in &mut probe.body.items[..2] {
            let ScalarBlockItem::LocalBinding(binding) = item else {
                unreachable!()
            };
            binding.name.clear();
            binding.name_span = ByteSpan::new(0, 0);
            binding.declared_type = ScalarType::Error;
            binding.receivers.clear();
        }
        let (actual, errors) = derive_function_reference_cfg(
            &program.source,
            probe,
            &module_names,
            row,
            &program.structs,
            &program.enums,
            true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(actual, expected);
    }
}

#[test]
fn raw_binding_outputs_use_canonical_primary_and_real_secondary() {
    let text = "%%start\nu8(u8) first = fn(input) { input };\nu8(u8) second = fn(input) { input };\n(u8, u8, u8)(u8) triple = fn(input) { input, input, input };\nunit(u8) probe = fn(input) { u8 left, u8 right = first(input) + input, second(input) + input; left; right; u8 a, u8 b, u8 c = triple(input); a; b; c; };\n%%end";
    fn rename(expression: &mut ScalarExpression, expected: &str, replacement: &str) {
        let ScalarExpression::Binary { left, .. } = expression else {
            panic!("binding output root");
        };
        let ScalarExpression::Call { name, .. } = left.as_mut() else {
            panic!("binding output call");
        };
        assert_eq!(name, expected);
        *name = replacement.into();
    }

    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let valid = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
        let project = |program: &ScalarProgram| ScalarProjectValidation {
            project: ScalarProject::new(
                vec![ScalarModule::from_program(program.clone(), Vec::new())],
                vec![source()],
            ),
            diagnostics: Vec::new(),
        };
        let baseline_raw = crate::emit_scalar_llvm(&valid).unwrap();
        let baseline_project = crate::emit_scalar_project_llvm(&project(&valid.program)).unwrap();

        for (position, canonical, same) in [(0, false, true), (1, false, false), (0, true, false)] {
            let mut changed = ScalarValidation {
                program: valid.program.clone(),
                diagnostics: Vec::new(),
            };
            let ScalarItem::Function(function) = &mut changed.program.items[3] else {
                panic!("probe function");
            };
            let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[0] else {
                panic!("independent binding");
            };
            let (expression, expected, replacement) = match (position, canonical) {
                (0, true) => (&mut binding.value, "first", "second"),
                (0, false) => (&mut binding.output_values[0].value, "first", "second"),
                (1, false) => (&mut binding.output_values[1].value, "second", "first"),
                _ => unreachable!(),
            };
            rename(expression, expected, replacement);

            let changed_project = project(&changed.program);
            let encoded_raw = serde_json::to_vec(&changed).unwrap();
            let encoded_project = serde_json::to_vec(&changed_project).unwrap();
            let decoded_raw = serde_json::from_slice(&encoded_raw).unwrap();
            let decoded_project = serde_json::from_slice(&encoded_project).unwrap();
            for (raw, project) in [
                (&changed, &changed_project),
                (&decoded_raw, &decoded_project),
            ] {
                let actual_raw = crate::emit_scalar_llvm(raw).unwrap();
                let actual_project = crate::emit_scalar_project_llvm(project).unwrap();
                assert_eq!(actual_raw == baseline_raw, same);
                assert_eq!(actual_project == baseline_project, same);
                let symbol = |name: &str| {
                    name.bytes()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                };
                for (name, count) in [
                    (
                        "first",
                        if same {
                            1
                        } else {
                            usize::from(position == 1) * 2
                        },
                    ),
                    (
                        "second",
                        if same {
                            1
                        } else {
                            usize::from(position == 0) * 2
                        },
                    ),
                ] {
                    let raw_symbol = format!("@{name}(");
                    let project_symbol = format!("__{}(", symbol(name));
                    assert_eq!(
                        actual_raw
                            .to_text()
                            .lines()
                            .filter(|line| line.contains("call i8") && line.contains(&raw_symbol))
                            .count(),
                        count
                    );
                    assert_eq!(
                        actual_project
                            .to_text()
                            .lines()
                            .filter(
                                |line| line.contains("call i8") && line.contains(&project_symbol)
                            )
                            .count(),
                        count
                    );
                }
            }
        }
    }
}

#[test]
fn checked_binding_initializer_retains_both_borrow_arms_and_the_pre_move_read() {
    let text = r#"%%start
*u8(*u8, *u8, bool) choose = fn(first, second, flag) {
    *u8 selected = if (flag) { &(*first) } else { &(*second) };
    selected
};
unit(*!u8) transfer = fn(input) {
    *!u8 output = input;
    *output = 3;
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
        let [(0, choose), (1, transfer)] = results.as_slice() else {
            panic!("expected the two resolved source bodies")
        };
        let [(bind, initializer)] = choose.proof_points.checked_binding_initializers.as_slice() else {
            panic!("expected a single lexical checked initialization")
        };
        let points = &choose.cfg.points;
        let origins = points[bind.0].possible_origins.as_ref().expect("checked binding origins");
        assert_eq!(points[initializer.0].possible_origins.as_ref(), Some(origins));
        assert_eq!(origins.len(), 2);
        let parameters = origins.iter().map(|id| match &choose.cfg.facts[id.0] {
            ReferenceOrigin::BorrowedFrom { parameter, loan } if loan.parent.is_some() => *parameter,
            origin => panic!("expected either reborrowed parameter, got {origin:?}"),
        }).collect::<BTreeSet<_>>();
        assert_eq!(parameters, BTreeSet::from([0, 1]));
        let [(bind, initializer)] = transfer.proof_points.checked_binding_initializers.as_slice() else {
            panic!("expected the exact checked initializer before the move")
        };
        let points = &transfer.cfg.points;
        assert!(matches!(points[initializer.0].kind, CfgPointKind::Read { .. }));
        assert!(points[initializer.0 + 1..bind.0].iter().any(|point| matches!(point.kind, CfgPointKind::Move { .. })));
        assert_eq!(points[bind.0].possible_origins, points[initializer.0].possible_origins);
        let public = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
    }
}

#[test]
fn prepared_fallible_guard_producer_uses_resolved_binding_and_null_polarity() {
    for (comparison, then_value, otherwise_value, present_on_then) in [
        ("&result == null", "7", "result", false),
        ("null != &result", "result", "7", true),
    ] {
        let text = format!("%%start\ni32() example = fn {{\n\ti32 result = 42;\n\tif ({comparison}) {{ {then_value} }} else {{ {otherwise_value} }}\n}};\n%%end");
        let parsed = parse_source(source(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            ScalarTargetLayout::WASM32,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let ScalarItem::Function(function) = &mut program.items[0] else { panic!("example") };
        let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[0] else {
            panic!("local receiver");
        };
        binding.fallible_assignment = Some(binding.name_span);
        let (results, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let [(0, result)] = results.as_slice() else { panic!("example reference result") };
        let [(branch, edge)] = result.proof_points.guarded_allocation_edges.as_slice() else {
            panic!("one typed guard");
        };
        let [then, otherwise] = result.cfg.points[branch.0].successors.as_slice() else {
            panic!("exact branch successors");
        };
        let carrier = result.cfg.guarded_scalar_allocation().expect("installed guarded allocation");
        assert_eq!((carrier.binding(), carrier.branch(), carrier.present_on_then()), (&edge.binding, *branch, present_on_then));
        let region = calls.functions[&0].conditionals.values().next().expect("resolved condition");
        let ConditionalRegion::Value { condition, .. } = region else { panic!("value condition"); };
        assert_eq!(carrier.condition(), condition.0);
        assert_eq!((edge.present, edge.absent), if present_on_then { (*then, *otherwise) } else { (*otherwise, *then) });
        assert!(result.cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Bind { binding } if binding == &edge.binding)));
        assert!(!result.cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Borrow { place, .. } if place.binding == edge.binding)));
    }
}

#[test]
fn prepared_fallible_reads_require_availability_on_all_projected_paths() {
    use ReferenceAnalysisErrorReason::{UnavailableFallibleAllocation as Unavailable, UnsupportedFallibleAllocationAccess as Unsupported};
    for (body, expected) in [
        ("if (&result == null) { 7 } else { result }", None),
        ("if (&result != null) { result } else { 7 }", None),
        ("result", Some(Unavailable)),
        ("if (&result == null) { result } else { 7 }", Some(Unavailable)),
        ("i32 observed = if (&result == null) { 7 } else { result };\n\tobserved;\n\tresult", Some(Unavailable)),
        ("result = 19;\n\t7", Some(Unsupported)),
        ("*i32 alias = &result;\n\talias;\n\t7", Some(Unsupported)),
    ] {
        let text = format!("%%start\ni32() example = fn {{\n\ti32 result = 42;\n\t{body}\n}};\n%%end");
        let parsed = parse_source(source(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            ScalarTargetLayout::WASM32,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let ScalarItem::Function(function) = &mut program.items[0] else { panic!("example") };
        let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[0] else {
            panic!("local receiver");
        };
        binding.fallible_assignment = Some(binding.name_span);
        let (results, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        let [(_, result)] = results.as_slice() else { panic!("example reference result"); };
        assert_eq!(result.cfg.guarded_scalar_allocation().is_none(), expected.is_some(), "{body}");
        assert_eq!(errors.len(), usize::from(expected.is_some()), "{body}: {errors:?}");
        assert!(errors.iter().all(|error| Some(error.reason()) == expected));
    }
}

#[test]
fn prepared_fallible_field_reads_require_the_exact_present_root() {
    use ReferenceAnalysisErrorReason::UnavailableFallibleAllocation;
    for (body, needs_refusal) in [
        ("if (&held == null) { 7 } else { held.value }", false),
        ("if (null != &held) { held.value } else { 7 }", false),
        ("if (&held == null) { held.value } else { 7 }", true),
        ("i32 prior = if (&held == null) { 7 } else { held.value };\n\tprior + held.value", true),
        ("held.value", true),
    ] {
        let text = format!("%%start\nstruct Record {{ i32 value; }}\ni32() example = fn {{\n\tRecord held = {{ .value = 42; }};\n\t{body}\n}};\n%%end");
        let parsed = parse_source(source(), text.clone(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            ScalarTargetLayout::WASM32,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let mut program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let [ScalarItem::Function(function)] = program.items.as_mut_slice() else { panic!("example") };
        let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[0] else { panic!("record receiver") };
        binding.fallible_assignment = Some(binding.name_span);
        let (results, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        let [(_, result)] = results.as_slice() else { panic!("example reference result"); };
        let reads = result.cfg.points.iter().filter(|point| matches!(&point.kind,
            CfgPointKind::ReadPlace { candidates, pointer_point: None, .. } if candidates.iter().any(|place|
                matches!(place.projections.as_slice(), [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(_))])
            )));
        assert!(reads.count() >= 1, "{body}");
        assert_eq!(result.cfg.guarded_scalar_allocation().is_none(), needs_refusal, "{body}");
        assert_eq!(errors.len(), usize::from(needs_refusal), "{body}: {errors:?}");
        assert!(errors.iter().all(|error| error.reason() == UnavailableFallibleAllocation), "{body}: {errors:?}");
    }
}

#[test]
fn whole_fallible_copy_overwrite_preserves_the_presence_requirement() {
    use ReferenceAnalysisErrorReason::{UnavailableFallibleAllocation as Unavailable, UnsupportedFallibleAllocationAccess as Unsupported};
    for present in [true, false] {
        assert_eq!(guarded_allocation_overwrite_error(true, true, !present), (!present).then_some(Unavailable));
        assert_eq!(guarded_allocation_overwrite_error(true, false, !present), Some(Unsupported));
        assert_eq!(guarded_allocation_overwrite_error(false, true, !present), Some(Unsupported));
    }
}
