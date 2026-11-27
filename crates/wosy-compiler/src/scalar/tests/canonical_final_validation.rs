use super::*;

#[test]
fn sole_final_extern_call_rejects_explicit_type_arguments_on_both_routes() {
    let call = "disk.read<i32>()";
    let text = "%%start
disk = extern wasm \"disk_abi\" { (i32, u64)() read; };
(i32, u64)() run = fn { disk.read<i32>() };
%%end";
    let root = source();
    let parsed = parse_source(root.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let cst = parsed.result.canonical_cst();
    let start = text.find(call).unwrap() as u32;
    let span = SourceSpan::new(
        root.clone(),
        ByteSpan::new(start, start + call.len() as u32),
    );
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = derive_scalar_program_from_cst_with_layout(cst, target_layout);
        let Err(project) = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(cst), Vec::new())],
            vec![root.clone()],
            ScalarPreparationInputs { target_layout },
        ) else {
            panic!("{target_layout:?}: project accepted explicit extern type arguments");
        };
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            let [diagnostic] = diagnostics.as_slice() else {
                panic!("{target_layout:?}: {diagnostics:?}");
            };
            assert_eq!(diagnostic.code, "B0004");
            assert_eq!(
                diagnostic.message,
                "generic argument arity does not match callable declaration"
            );
            let [label] = diagnostic.labels.as_slice() else {
                panic!("{diagnostic:?}");
            };
            assert_eq!(label.kind, crate::DiagnosticLabelKind::Primary);
            assert_eq!(label.span, span);
        }
    }
}

#[test]
fn sole_final_namespace_call_reports_missing_member_on_both_targets() {
    let text = "%%start
lib = namespace package \"library.w\";
(i32, u64)() run = fn { lib.missing() };
%%end";
    let root = source();
    let dependency = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "library.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(root.clone(), text.into(), &[]);
    let imported = parse_source(
        dependency.clone(),
        "%%start\ni32 present = 1;\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert!(
        imported.diagnostics.is_empty(),
        "{:?}",
        imported.diagnostics
    );
    let start = text.find("missing").unwrap() as u32;
    let span = SourceSpan::new(root.clone(), ByteSpan::new(start, start + 7));
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let namespace = declarations.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            target: dependency.clone(),
            span: namespace.binding_span,
        };
        let Err(failure) = prepare_scalar_project(
            vec![
                (declarations, vec![binding]),
                (
                    derive_scalar_declarations_from_cst(imported.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![dependency.clone(), root.clone()],
            ScalarPreparationInputs { target_layout },
        ) else {
            panic!("{target_layout:?}: project accepted missing namespace member");
        };
        let [diagnostic] = failure.diagnostics.as_slice() else {
            panic!("{target_layout:?}: {:?}", failure.diagnostics);
        };
        assert_eq!(diagnostic.code, "M0002");
        assert_eq!(diagnostic.message, "unknown module member");
        let [primary, secondary] = diagnostic.labels.as_slice() else {
            panic!("{diagnostic:?}");
        };
        assert_eq!(primary.kind, crate::DiagnosticLabelKind::Primary);
        assert_eq!(primary.span, span);
        assert_eq!(secondary.kind, crate::DiagnosticLabelKind::Secondary);
        assert_eq!(
            secondary.span,
            SourceSpan::new(dependency.clone(), ByteSpan::new(0, 0))
        );
    }
}

#[test]
fn sole_final_contextual_overload_validates_selected_output_positions() {
    let call = "choose(1)";
    for (second_output, valid) in [("bool", true), ("u8", false)] {
        let text = format!(
            "%%start
choose = overload {{
    (i32, u8)(i32) => fn(value) {{ u8 byte = 1; value, byte }};
    (i64, bool)(i64) => fn(value) {{ value, true }};
}};
(i64, {second_output})() run = fn {{ {call} }};
%%end"
        );
        let root = source();
        let start = text.find(call).unwrap() as u32;
        let span = SourceSpan::new(
            root.clone(),
            ByteSpan::new(start, start + call.len() as u32),
        );
        let parsed = parse_source(root.clone(), text, &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let cst = parsed.result.canonical_cst();
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let single = derive_scalar_program_from_cst_with_layout(cst, target_layout);
            let project = prepare_scalar_project(
                vec![(derive_scalar_declarations_from_cst(cst), Vec::new())],
                vec![root.clone()],
                ScalarPreparationInputs { target_layout },
            );
            if valid {
                assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
                assert!(project.is_ok(), "{:?}", project.err());
            } else {
                let Err(project) = project else {
                    panic!("{target_layout:?}: project accepted mismatched second output");
                };
                for diagnostics in [&single.diagnostics, &project.diagnostics] {
                    let [diagnostic] = diagnostics.as_slice() else {
                        panic!("{target_layout:?}: {diagnostics:?}");
                    };
                    assert_eq!(diagnostic.code, "B0003");
                    assert_eq!(
                        diagnostic.message,
                        "expression type does not match expected type"
                    );
                    assert_eq!(diagnostic.labels[0].span, span);
                }
            }
        }
    }
}

#[test]
fn sole_final_call_with_known_empty_outputs_reports_arity_on_both_routes() {
    let call = "disk.discard()";
    let text = "%%start
disk = extern wasm \"disk_abi\" { unit() discard; };
(i32, u64)() run = fn { disk.discard() };
%%end";
    let root = source();
    let parsed = parse_source(root.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || {
        let mut declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let ScalarItem::Extern(extern_decl) = &mut declarations.items[0] else {
            panic!("extern declaration");
        };
        let ScalarType::Callable { outputs, .. } = &mut extern_decl.functions[0].signature else {
            panic!("extern callable");
        };
        outputs.outputs.clear();
        declarations
    };
    let start = text.find(call).unwrap() as u32;
    let span = SourceSpan::new(
        root.clone(),
        ByteSpan::new(start, start + call.len() as u32),
    );
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = typecheck_scalar_program(declarations(), target_layout);
        let Err(project) = prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![root.clone()],
            ScalarPreparationInputs { target_layout },
        ) else {
            panic!("{target_layout:?}: project accepted empty final call outputs");
        };
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            let [diagnostic] = diagnostics.as_slice() else {
                panic!("{target_layout:?}: {diagnostics:?}");
            };
            assert_eq!(diagnostic.code, "B0004");
            assert_eq!(
                diagnostic.message,
                "function output arity does not match its final output list"
            );
            assert_eq!(diagnostic.labels[0].span, span);
        }
    }
}

#[test]
fn raw_project_preparation_resolves_branch_local_aggregate_types() {
    let root = source();
    let dependency = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "types.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        root.clone(),
        "%%start
types = namespace package \"types.w\";
struct Payload { u8 value; }
u8(bool) choose = fn(flag) {
    if (flag) {
        Payload payload = { .value = 42; };
        payload.value
    } else {
        types.Status status = types.Status::ready;
        if (status == types.Status::ready) { 1 } else { 0 }
    }
};
%%end"
            .into(),
        &[],
    );
    let imported = parse_source(
        dependency.clone(),
        "%%start\nenum Status { ready; waiting; }\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    assert!(
        imported.diagnostics.is_empty(),
        "{:?}",
        imported.diagnostics
    );
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let namespace = declarations.namespaces().next().expect("types namespace");
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            target: dependency.clone(),
            span: namespace.span,
        };
        prepare_scalar_project(
            vec![
                (declarations, vec![binding]),
                (
                    derive_scalar_declarations_from_cst(imported.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![dependency.clone(), root.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
    }
}

#[test]
fn canonical_final_validation_borrows_mixed_forwarded_and_conditional_roots_on_both_routes() {
    let text = "%%start\n(u8, bool)(u8, bool) mixed = fn(value, flag) { value, flag };\n(u8, bool)(u8, bool) forward = fn(value, flag) { mixed(value, flag) };\n(u8, bool)(u8, bool) choose = fn(value, flag) { if (flag) { value, flag } else { value, flag } };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        for item in &declarations.items {
            let ScalarItem::Function(function) = item else {
                unreachable!()
            };
            let start = function.body.items.len() - function.body.final_output_values.len();
            for (position, (expression, metadata)) in function.body.final_expressions().enumerate()
            {
                let ScalarBlockItem::Expression(canonical) = &function.body.items[start + position]
                else {
                    unreachable!()
                };
                assert!(std::ptr::eq(expression, canonical));
                assert!(std::ptr::eq(
                    metadata,
                    &function.body.final_output_values[position]
                ));
            }
        }
        let single = typecheck_scalar_program(declarations, target);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let module = ScalarModule::from_program(single.program, Vec::new());
        let project = typecheck_scalar_project(ScalarProject::new(vec![module], vec![source()]));
        assert!(project.diagnostics.is_empty(), "{:?}", project.diagnostics);
    }
}

#[test]
fn canonical_final_validation_uses_changed_item_with_original_slot_diagnostic_span() {
    let text = "%%start\n(u8, bool)(u8, bool) mixed = fn(value, flag) { value, flag };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let mut declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let baseline = typecheck_scalar_program(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            target,
        );
        assert!(
            baseline.diagnostics.is_empty(),
            "{:?}",
            baseline.diagnostics
        );
        let ScalarItem::Function(function) = &mut declarations.items[0] else {
            unreachable!()
        };
        let metadata = function.body.final_output_values.clone();
        let parameter_span = function.parameter_spans[0];
        let slot_span = metadata[0].span;
        let replacement = ScalarExpression::Boolean {
            value: true,
            span: metadata[1].span,
        };
        function.body.items[0] = ScalarBlockItem::Expression(replacement.clone());
        assert_eq!(function.body.final_output_values, metadata);
        let single = typecheck_scalar_program(declarations, target);
        let mut module = ScalarModule::from_program(baseline.program, Vec::new());
        let ScalarItem::Function(function) = &mut module.items[0] else {
            unreachable!()
        };
        function.body.items[0] = ScalarBlockItem::Expression(replacement);
        assert_eq!(function.body.final_output_values, metadata);
        let project = typecheck_scalar_project(ScalarProject::new(vec![module], vec![source()]));
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.code, "B0003");
            assert_eq!(
                diagnostic.message,
                "expression type does not match expected type"
            );
            assert_eq!(
                diagnostic.labels[0].span,
                SourceSpan::new(source(), slot_span)
            );
            let unused = &diagnostics[1];
            assert_eq!(unused.code, "B0009");
            assert_eq!(
                unused.message,
                "local binding or parameter has no required static use"
            );
            assert_eq!(
                unused.labels[0].span,
                SourceSpan::new(source(), parameter_span)
            );
        }
    }
}

#[test]
fn reads_every_assignment_destination_projection_and_rhs_on_both_routes() {
    for text in [
        "%%start\nu8[2] values = [1, 1];\nunit(u64) set = fn(index) { values[index] = 2; };\nset(1);\n%%end",
        "%%start\nunit(u64, u8, u8) set = fn(index, first, second) { u8[2] left = [1, 1]; u8[2] right = [1, 1]; left[index] = first; left[0], right[1] = first, second; };\nset(0, 2, 3);\n%%end",
        "%%start\nstruct Buffer { u8[2] bytes; }\nunit(u64, *!Buffer, u8) store = fn(offset, writer, input) { (*writer).bytes[offset] = input; };\n%%end",
    ] {
        let root = source();
        let parsed = parse_source(root.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let cst = parsed.result.canonical_cst();
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let single = derive_scalar_program_from_cst_with_layout(cst, target_layout);
            assert!(single.diagnostics.is_empty(), "{text}: {:?}", single.diagnostics);
            let prepared = prepare_scalar_project(
                vec![(derive_scalar_declarations_from_cst(cst), Vec::new())],
                vec![root.clone()],
                ScalarPreparationInputs { target_layout },
            ).unwrap_or_else(|failure| panic!("{text}: {:?}", failure.diagnostics));
            let checked = analyze_scalar_build(&prepared);
            assert!(checked.diagnostics().is_empty(), "{text}: {:?}", checked.diagnostics());
        }
    }
}

#[test]
fn dynamic_second_destination_index_is_used_even_when_distinctness_is_rejected() {
    let text = "%%start\nu8[2] values = [1, 1];\nunit(u64, u8, u8) set = fn(index, first, second) { u8[2] local = [1, 1]; local[0], values[index] = first, second; };\n%%end";
    let root = source();
    let parsed = parse_source(root.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let cst = parsed.result.canonical_cst();
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = derive_scalar_program_from_cst_with_layout(cst, target_layout);
        let Err(project) = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(cst), Vec::new())],
            vec![root.clone()],
            ScalarPreparationInputs { target_layout },
        ) else {
            panic!("{target_layout:?}: expected a distinctness diagnostic");
        };
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            let [diagnostic] = diagnostics.as_slice() else {
                panic!("{target_layout:?}: {diagnostics:?}");
            };
            assert_eq!(diagnostic.code, "B0002");
            assert_eq!(diagnostic.message, "assignment targets require a distinctness proof");
        }
    }
}

#[test]
fn assignment_rhs_is_checked_once_for_nested_unused_bindings() {
    let text = "%%start\ni32() assign = fn { i32 result = 0; result = if (true) { i32 unused_inner = 1; 2 } else { 3 }; result };\n%%end";
    let declaration = "i32 unused_inner = 1;";
    let root = source();
    let parsed = parse_source(root.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let cst = parsed.result.canonical_cst();
    let start = text.find(declaration).unwrap() as u32;
    let span = ByteSpan::new(start, start + declaration.len() as u32);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = derive_scalar_program_from_cst_with_layout(cst, target_layout);
        let Err(project) = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(cst), Vec::new())],
            vec![root.clone()],
            ScalarPreparationInputs { target_layout },
        ) else {
            panic!("{target_layout:?}: prepared build accepted unread binding");
        };
        for diagnostics in [&single.diagnostics, &project.diagnostics] {
            let [diagnostic] = diagnostics.as_slice() else {
                panic!("{target_layout:?}: {diagnostics:?}");
            };
            assert_eq!(diagnostic.code, "B0009");
            assert_eq!(diagnostic.labels[0].span.range, span);
        }
    }
}
