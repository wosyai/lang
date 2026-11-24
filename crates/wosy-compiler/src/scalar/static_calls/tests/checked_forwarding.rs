use super::*;

const DRIVER: &str = "%%start
i32(*u8) step = fn(view) { if (*view == 10) { 1 } else { 0 } };
i32(*u8, i32) drive = fn(view, outcome) {
    if (outcome == 0) { drive(view, step(view)) } else { outcome }
};
%%end";

#[test]
fn detached_checked_forwarding_admits_only_the_returned_occurrence() {
    let source = parsed(DRIVER);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_extracted(&source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let calls = prepared.function_calls(0, 1);
        assert!(calls.self_tail_candidates.is_empty());
        assert_eq!(
            build
                .checked_self_tail_forwarding(0, 1)
                .map(|proof| proof.recursive()),
            Some(reference::OccurrenceOrdinal(4))
        );
        assert_eq!(build.checked_self_tail_forwarding(0, 0), None);
    }
}

#[test]
fn detached_checked_forwarding_rejects_local_projection_and_extra_work() {
    for body in [
        "u8 local = 0; if (outcome == 0) { drive(view, step(&local)) } else { outcome }",
        "Cell local = { .value = 0; }; if (outcome == 0) { drive(view, step(&local.value)) } else { outcome }",
        "if (outcome == 0) { step(view); drive(view, step(view)) } else { outcome }",
    ] {
        let projection_type = if body.contains("Cell local") {
            "struct copy Cell { u8 value; }"
        } else {
            ""
        };
        let source = parsed(&format!(
            "%%start
{projection_type}
i32(*u8) step = fn(view) {{ if (*view == 10) {{ 1 }} else {{ 0 }} }};
i32(*u8, i32) drive = fn(view, outcome) {{ {body} }};
%%end"
        ));
        for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let prepared = prepare_extracted(&source, layout);
            let build = analyze_scalar_build(&prepared);
            assert!(
                build.diagnostics().is_empty(),
                "{body}: {:?}",
                build.diagnostics()
            );
            assert_eq!(build.checked_self_tail_forwarding(0, 1), None, "{body}");
        }
    }
}

#[test]
fn detached_checked_forwarding_rejects_missing_or_inconsistent_build_evidence() {
    let source = parsed(DRIVER);
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_extracted(&source, layout);
        let build = analyze_scalar_build(&prepared);
        let module = &prepared.project().modules[0];
        let ScalarItem::Function(function) = &module.items[1] else {
            panic!("driver")
        };
        let calls = prepared.function_calls(0, 1);
        let cfg = build.reference_cfg(0, 1, None).unwrap();
        let prove =
            |cfg| reference::self_tail::checked::prove(&module.source, function, calls, cfg);
        let ordinal = build
            .checked_self_tail_forwarding(0, 1)
            .unwrap()
            .recursive();
        let self_call = cfg.primary_call_point(ordinal).unwrap();
        let CfgPointKind::Call {
            argument_points, ..
        } = &cfg.points[self_call.0].kind
        else {
            panic!("recursive call")
        };
        let read = argument_points[0];
        let step_read = cfg.points[read.0].successors[0];
        let mut broken = cfg.clone();
        broken.primary_call_points.remove(&ordinal);
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        broken.points[read.0].possible_origins = None;
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        let ReferenceOrigin::BorrowedFrom { parameter, .. } = &mut broken.facts[0] else {
            panic!("parameter origin")
        };
        *parameter = 1;
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        let ReferenceOrigin::BorrowedFrom { loan, .. } = &mut broken.facts[0] else {
            panic!("parameter origin")
        };
        loan.mode = ScalarReferenceMutability::Mutable;
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        for graph in [&mut broken.points, &mut broken.initial_points] {
            graph[step_read.0].kind = CfgPointKind::ScopeExit;
        }
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        broken.points[read.0].successors = cfg.points[step_read.0].successors.clone();
        broken.initial_points[read.0].successors = cfg.initial_points[step_read.0].successors.clone();
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        let CfgPointKind::Call {
            argument_points, ..
        } = &mut broken.initial_points[self_call.0].kind
        else {
            panic!("recursive call")
        };
        argument_points.swap(0, 1);
        assert_eq!(prove(&broken), None);
        let mut broken = cfg.clone();
        let after_call = cfg.points[self_call.0].successors[0];
        for graph in [&mut broken.points, &mut broken.initial_points] {
            graph[after_call.0].kind = CfgPointKind::Evaluate(ScalarExpression::Boolean {
                value: true,
                span: graph[after_call.0].source_span.range,
            });
        }
        assert_eq!(prove(&broken), None);
    }
}


#[test]
fn detached_checked_forwarding_requires_the_mutable_child_before_the_single_move() {
    let source = parsed(
        "%%start
i32(*!u8) step = fn(view) { if (*view == 10) { 1 } else { 0 } };
i32(i32, *!u8) drive = fn(outcome, view) {
    if (outcome == 0) { drive(step(&!(*view)), view) } else { outcome }
};
%%end",
    );
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_extracted(&source, layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let module = &prepared.project().modules[0];
        let ScalarItem::Function(function) = &module.items[1] else { panic!("driver") };
        let calls = prepared.function_calls(0, 1);
        let cfg = build.reference_cfg(0, 1, None).unwrap();
        let proof = build.checked_self_tail_forwarding(0, 1).unwrap();
        let [recursive, step] = proof.primitive_outputs(cfg).unwrap();
        let CfgPointKind::Call { argument_points, .. } = &cfg.points[recursive.0].kind else {
            panic!("recursive call")
        };
        let [step_result, carrier] = argument_points.as_slice() else { panic!("arguments") };
        assert_eq!(*step_result, step);
        let [movement] = cfg.points[carrier.0].successors.as_slice() else { panic!("move") };
        assert!(matches!(cfg.points[movement.0].kind, CfgPointKind::Move { .. }));
        let prove = |changed| reference::self_tail::checked::prove(&module.source, function, calls, changed);
        let mut broken = cfg.clone();
        for graph in [&mut broken.points, &mut broken.initial_points] {
            graph[step.0].successors = vec![*movement];
        }
        assert_eq!(prove(&broken), None, "cannot omit the exact carrier read");
        let mut broken = cfg.clone();
        let ReferenceOrigin::BorrowedFrom { loan, .. } = &mut broken.facts[1] else {
            panic!("child origin")
        };
        loan.parent = None;
        assert_eq!(prove(&broken), None, "cannot fabricate a parentless child");
    }
}
