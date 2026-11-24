use super::*;
use crate::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, emit_scalar_build_project_llvm,
    emit_scalar_build_single_module_llvm, parse_source, prepare_scalar_project,
    PreparedScalarProject, ScalarPreparationInputs, ScalarTargetLayout,
};

const TARGETS: [ScalarTargetLayout; 2] = [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32];

fn prepare(target_layout: ScalarTargetLayout) -> PreparedScalarProject {
    let source = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "main.w".into(),
        "r1".into(),
    );
    let text = "%%start
i32() first = fn { 11 };
i32() second = fn { 22 };
i32 value = unsafe { first() + second() };
%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    prepare_scalar_project(
        vec![(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            Vec::new(),
        )],
        vec![source],
        ScalarPreparationInputs { target_layout },
    )
    .unwrap_or_else(|error| panic!("{target_layout:?}: {:?}", error.diagnostics))
}

fn operands(items: &mut [ScalarItem]) -> (&mut ScalarExpression, &mut ScalarExpression) {
    let ScalarItem::Binding(binding) = &mut items[2] else {
        panic!("module binding");
    };
    let ScalarExpression::Block(block) = &mut binding.value else {
        panic!("initializer block");
    };
    let [ScalarBlockItem::Expression(ScalarExpression::Binary { left, right, .. })] =
        block.items.as_mut_slice()
    else {
        panic!("canonical final binary expression");
    };
    (left, right)
}

#[test]
fn binds_distinct_equal_span_calls_to_their_structural_rows() {
    for layout in TARGETS {
        let prepared = prepare(layout);
        let build = analyze_scalar_build(&prepared);
        for emitted in [
            emit_scalar_build_single_module_llvm(&build),
            emit_scalar_build_project_llvm(&build),
        ] {
            let ir = emitted.expect("valid initializer emission").to_text();
            assert_eq!(ir.matches("call i32 @").count(), 2, "{ir}");
        }

        let region = prepared.initialization_region(0).unwrap();
        let mut items = prepared.project().modules[0].items.clone();
        let (left, right) = operands(&mut items);
        let ScalarExpression::Call { span, .. } = right else {
            panic!("right call");
        };
        *span = expression_span(left);
        assert_eq!(expression_span(left), expression_span(right));
        let nodes = region.bind_call_nodes(&items).unwrap();
        let ScalarItem::Binding(binding) = &items[2] else {
            panic!("module binding")
        };
        let ScalarExpression::Block(block) = &binding.value else {
            panic!("initializer block")
        };
        let (expression, _) = block.final_expressions().next().unwrap();
        let ScalarExpression::Binary {
            left: first,
            right: second,
            ..
        } = expression
        else {
            panic!("binary");
        };
        for (expression, item) in [(second.as_ref(), &items[1]), (first.as_ref(), &items[0])] {
            let ScalarItem::Function(function) = item else {
                panic!("function declaration")
            };
            assert_eq!(
                nodes.target(expression).unwrap(),
                &ModuleInitializationCallTarget::Direct {
                    declaration: SourceSpan::new(region.source.clone(), function.name_span),
                    concrete: ConcreteCallSelection::Declaration,
                },
            );
        }
        let copy = second.as_ref().clone();
        assert!(nodes
            .target(&copy)
            .unwrap_err()
            .contains("missing canonical initializer call node"));
    }
}

#[test]
fn binding_rejects_a_missing_row_at_the_canonical_call_edge() {
    for layout in TARGETS {
        let prepared = prepare(layout);
        let mut region = prepared.initialization_region(0).unwrap().clone();
        let ModuleInitializationDetail::Occurrences(topology) = &mut region.detail else {
            panic!("prepared topology");
        };
        let &[_, missing] = topology.calls.as_slice() else {
            panic!("two calls")
        };
        assert!(topology.call_targets.remove(&missing).is_some());
        let span = topology.occurrences[missing.0].span;
        let error = region
            .bind_call_nodes(&prepared.project().modules[0].items)
            .err()
            .unwrap();
        assert_eq!(
            error,
            format!(
                "internal error: missing initializer call target in {:?} at {}..{} (occurrence {})",
                region.source, span.start, span.end, missing.0,
            )
        );
    }
}
