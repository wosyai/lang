use super::*;
use control::CallableBodyCallNodes;

const TARGETS: [ScalarTargetLayout; 2] = [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32];
const RUN: usize = 2;
const ARM: usize = 1;
const TEXT: &str = "%%start
u64(u64) first = fn(n) { n };
u64(u64) second = fn(n) { n };
run = overload {
    u64(u8) => fn(byte) { byte; 0 };
    u64(u64) => fn(n) { u8[first(second(n))] bytes; bytes; unsafe { first(n) + second(n) } };
};
%%end";

fn function(prepared: &PreparedScalarProject, item: usize) -> &ScalarFunction {
    let ScalarItem::Function(function) = &prepared.project().modules[0].items[item] else {
        panic!("function declaration at {item}");
    };
    function
}

fn final_binary(body: &mut ScalarBlock) -> (&mut ScalarExpression, &mut ScalarExpression) {
    let Some(ScalarBlockItem::Expression(ScalarExpression::Block(block))) = body.items.last_mut()
    else {
        panic!("canonical nested output block");
    };
    let [ScalarBlockItem::Expression(ScalarExpression::Binary { left, right, .. })] =
        block.items.as_mut_slice()
    else {
        panic!("canonical binary output");
    };
    (left, right)
}

#[test]
fn binds_actual_body_nodes_by_ordinal_including_nested_allocation_lengths() {
    let parsed = parsed(TEXT);
    for target in TARGETS {
        let prepared = prepare_extracted(&parsed, target);
        let calls = prepared.calls.function_body(0, RUN, Some(ARM)).unwrap();
        let arm = &function(&prepared, RUN).overload_arms[ARM];
        let mut body = arm.body.clone();
        let (left, right) = final_binary(&mut body);
        let ScalarExpression::Call { span, .. } = right else {
            panic!("second call");
        };
        *span = typecheck::expression_span(left);
        assert_eq!(
            typecheck::expression_span(left),
            typecheck::expression_span(right)
        );

        let nodes = prepared
            .bind_callable_body_call_nodes(0, RUN, Some(ARM), &body)
            .unwrap();
        let (root, _) = body.final_expressions().next().unwrap();
        let ScalarExpression::Block(block) = root else {
            panic!("nested output block");
        };
        let (binary, _) = block.final_expressions().next().unwrap();
        let ScalarExpression::Binary { left, right, .. } = binary else {
            panic!("binary output");
        };
        let ScalarBlockItem::LocalBinding(binding) = &body.items[0] else {
            panic!("runtime allocation binding");
        };
        let length = binding.receivers[0].allocation_length.as_ref().unwrap();
        let ScalarExpression::Call { arguments, .. } = length else {
            panic!("outer allocation length call");
        };
        let check = |expression, row, declaration| {
            let selected = nodes.target(expression).unwrap();
            assert!(std::ptr::eq(selected, row));
            assert_eq!(
                selected,
                &ResolvedCallTarget::ModuleCallable {
                    source: parsed.source.clone(),
                    declaration_span: function(&prepared, declaration).name_span,
                    concrete: ConcreteCallSelection::Declaration,
                },
            );
        };
        let [left_row, right_row] = calls.call_occurrences.as_slice() else {
            panic!("two ordinary call occurrences");
        };
        let [outer, inner] = calls.allocation_length_calls.as_slice() else {
            panic!("two executable length call occurrences");
        };
        check(left, &calls.sites[left_row.site].target, 0);
        check(right, &calls.sites[right_row.site].target, 1);
        check(length, &calls.allocation_length_call_targets[outer], 0);
        check(
            &arguments[0],
            &calls.allocation_length_call_targets[inner],
            1,
        );
        assert!(nodes.target(&binding.value).is_err());
        assert!(nodes.target(&left.as_ref().clone()).is_err());

        assert!(prepared
            .bind_callable_body_call_nodes(0, RUN, Some(2), &body)
            .is_err());
    }
}

#[test]
fn rejects_missing_body_call_rows_with_prepared_source_span_and_ordinal() {
    let parsed = parsed(TEXT);
    for target in TARGETS {
        let prepared = prepare_extracted(&parsed, target);
        let body = &function(&prepared, RUN).overload_arms[ARM].body;
        let calls = prepared.calls.function_body(0, RUN, Some(ARM)).unwrap();
        let check = |calls: &reference::FunctionCallSites,
                     missing: reference::OccurrenceOrdinal| {
            let span = calls.occurrences[missing.0].span;
            assert_eq!(
                CallableBodyCallNodes::bind(&parsed.source, body, calls, None)
                    .err()
                    .expect("missing target must reject the entire binding"),
                format!(
                    "internal error: missing callable body call target in {:?} at {}..{} (occurrence {})",
                    parsed.source, span.start, span.end, missing.0,
                ),
            );
        };

        let mut missing_site = calls.clone();
        missing_site.call_occurrences[1].site = missing_site.sites.len();
        check(&missing_site, calls.call_occurrences[1].ordinal);

        let mut missing_length = calls.clone();
        let missing = calls.allocation_length_calls[1];
        assert!(missing_length
            .allocation_length_call_targets
            .remove(&missing)
            .is_some());
        check(&missing_length, missing);
    }
}
