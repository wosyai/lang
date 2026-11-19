use super::*;

const DRIVER: &str = "%%start
i32(*!u8) step = fn(view) { if (*view == 10) { 1 } else { 0 } };
i32(*!u8, i32) drive = fn(view, outcome) {
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
            build.checked_self_tail_forwarding(0, 1),
            Some(reference::OccurrenceOrdinal(4))
        );
        assert_eq!(build.checked_self_tail_forwarding(0, 0), None);
    }
}

#[test]
fn detached_checked_forwarding_rejects_local_projection_and_extra_work() {
    for body in [
        "u8 local = 0; if (outcome == 0) { drive(view, step(&!local)) } else { outcome }",
        "u8[1] local = [0]; if (outcome == 0) { drive(view, step(&!local[0])) } else { outcome }",
        "if (outcome == 0) { step(view); drive(view, step(view)) } else { outcome }",
    ] {
        let source = parsed(&format!(
            "%%start
i32(*!u8) step = fn(view) {{ if (*view == 10) {{ 1 }} else {{ 0 }} }};
i32(*!u8, i32) drive = fn(view, outcome) {{ {body} }};
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
        let ordinal = build.checked_self_tail_forwarding(0, 1).unwrap();
        let self_call = cfg.primary_call_point(ordinal).unwrap();
        let CfgPointKind::Call {
            argument_points, ..
        } = &cfg.points[self_call.0].kind
        else {
            panic!("recursive call")
        };
        let read = argument_points[0];
        let movement = cfg.points[read.0].successors[0];
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
        for graph in [&mut broken.points, &mut broken.initial_points] {
            graph[movement.0].kind = CfgPointKind::ScopeExit;
        }
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
