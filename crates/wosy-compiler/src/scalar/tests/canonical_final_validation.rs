use super::*;

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
                assert!(!std::ptr::eq(expression, &metadata.value));
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
