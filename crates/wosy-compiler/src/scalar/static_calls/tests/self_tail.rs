use super::*;

#[test]
fn selects_returned_self_calls_on_both_preparations_and_layouts() {
    let parsed = parsed(
        "%%start
unit(i32) observe = fn(n) { n; };
i32(i32, i32) tail_sum = fn(n, total) {
    observe(n);
    observe(total);
    if (n == 0) { total } else { tail_sum(n - 1, total + n) }
};
i32(i32) direct = fn(n) { direct(n) };
i32(i32) nested = fn(n) {
    if (n == 0) { unsafe { nested(n + 1) } } else { nested(n - 1) }
};
%%end",
    );
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let project = prepare_extracted(&parsed, target);
        for item in [1, 2, 3] {
            for calls in [single.function_calls(item), project.function_calls(0, item)] {
                let candidate_count = if item == 3 { 2 } else { 1 };
                let offset = calls.call_occurrences.len() - candidate_count;
                let returned: Vec<_> = calls.call_occurrences[offset..]
                    .iter()
                    .map(|row| row.ordinal)
                    .collect();
                assert_eq!(calls.self_tail_candidates, returned);
            }
            let ScalarItem::Function(function) = &project.project().modules[0].items[item] else {
                panic!("function at item {item}");
            };
            let nodes = project
                .bind_callable_body_call_nodes(0, item, None, &function.body)
                .unwrap();
            let (root, _) = function.body.final_expressions().next().unwrap();
            let call = match root {
                ScalarExpression::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    if item == 3 {
                        let (nested, _) = then_branch.final_expressions().next().unwrap();
                        let ScalarExpression::Block(block) = nested else {
                            panic!("explicit tail block");
                        };
                        assert!(nodes
                            .is_self_tail_candidate(block.final_expressions().next().unwrap().0));
                    }
                    else_branch.final_expressions().next().unwrap().0
                }
                ScalarExpression::Call { .. } => root,
                _ => panic!("returned producer"),
            };
            assert!(nodes.has_self_tail_candidates());
            assert!(nodes.is_self_tail_candidate(call));
            assert!(!nodes.is_self_tail_candidate(&call.clone()));
            if item == 1 {
                let ScalarBlockItem::Expression(effect) = &function.body.items[0] else {
                    panic!("prefix effect");
                };
                assert!(!nodes.is_self_tail_candidate(effect));
            }
        }
    }
}

#[test]
fn excludes_other_calls_and_signatures_on_both_preparations_and_layouts() {
    let parsed = parsed(
        "%%start
i32(i32) non_tail = fn(n) { if (n == 0) { n } else { non_tail(n - 1) + n } };
i32(i32) first = fn(n) { second(n) };
i32(i32) second = fn(n) { first(n) };
bool(i32) condition = fn(n) { if (condition(n)) { true } else { false } };
i32(i32) prefix = fn(n) { prefix(n); n };
i32(*?u8) pointer_param = fn(p) { pointer_param(p) };
*?u8(i32) pointer_result = fn(n) { pointer_result(n) };
unit(i32) unit_result = fn(n) { unit_result(n) };
generic T;
T(T) generic_self = fn(value) { generic_self<T>(value) };
overloaded = overload { i32(i32) => fn(n) { overloaded(n) }; };
%%end",
    );
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let project = prepare_extracted(&parsed, target);
        assert_eq!(project.project().modules[0].items.len(), 10);
        for (item, declaration) in project.project().modules[0].items.iter().enumerate() {
            let ScalarItem::Function(function) = declaration else {
                panic!("function at item {item}");
            };
            for calls in [single.function_calls(item), project.function_calls(0, item)] {
                assert!(calls.self_tail_candidates.is_empty(), "{}", function.name);
                if function.overload_arms.is_empty() {
                    assert_eq!(calls.call_occurrences.len(), 1, "{}", function.name);
                } else {
                    let [arm] = calls.arms.as_slice() else {
                        panic!("one concrete overload arm");
                    };
                    assert!(arm.self_tail_candidates.is_empty());
                    assert_eq!(arm.call_occurrences.len(), 1);
                }
            }
            let (arm_index, body) = match function.overload_arms.as_slice() {
                [] => (None, &function.body),
                [arm] => (Some(0), &arm.body),
                _ => unreachable!(),
            };
            assert!(!project
                .bind_callable_body_call_nodes(0, item, arm_index, body)
                .unwrap()
                .has_self_tail_candidates());
        }
    }
}
