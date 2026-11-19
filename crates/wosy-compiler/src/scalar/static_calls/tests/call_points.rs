use super::*;
use reference::OccurrenceOrdinal;

#[test]
fn detached_primary_call_points_preserve_canonical_occurrence_identity() {
    let source = parsed(
        "%%start
u8(u8) echo = fn(value) { value };
(u8, u8)(u8) pair = fn(value) { value, value };
u8(u8) probe = fn(value) {
    u8 first, u8 second = pair(echo(value));
    echo(echo(first + second))
};
%%end",
    );
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_extracted(&source, target_layout);
        let build = analyze_scalar_build(&prepared);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let calls = prepared.function_calls(0, 2);
        let cfg = build.reference_cfg(0, 2, None).unwrap();
        let recorded: Vec<_> = calls
            .call_occurrences
            .iter()
            .map(|row| {
                (
                    row.ordinal.0,
                    cfg.primary_call_point(row.ordinal).unwrap().0,
                )
            })
            .collect();
        assert_eq!(recorded, [(0, 4), (1, 3), (3, 12), (4, 11)]);
        assert_eq!(cfg.primary_call_points.len(), recorded.len());
        assert_eq!(cfg.primary_call_point(OccurrenceOrdinal(2)), None);
        for points in [&cfg.initial_points, &cfg.points] {
            for (ordinal, id, output, arguments) in [
                (1, 3, 0, &[2][..]),
                (0, 4, 0, &[3][..]),
                (0, 6, 1, &[3][..]),
                (4, 11, 0, &[10][..]),
                (3, 12, 0, &[11][..]),
            ] {
                let point = &points[id];
                assert_eq!(point.id, CfgPointId(id));
                let CfgPointKind::Call {
                    output: actual,
                    target,
                    argument_points,
                    ..
                } = &point.kind
                else {
                    panic!("call point {id}: {point:?}");
                };
                let row = calls
                    .call_occurrences
                    .iter()
                    .find(|row| row.ordinal.0 == ordinal)
                    .unwrap();
                assert_eq!(target, &calls.sites[row.site].target);
                assert_eq!(*actual, output);
                assert_eq!(
                    argument_points
                        .iter()
                        .map(|point| point.0)
                        .collect::<Vec<_>>(),
                    arguments
                );
            }
        }
        let mut cloned = cfg.clone();
        assert_eq!(cloned.primary_call_points, cfg.primary_call_points);
        cloned
            .primary_call_points
            .insert(OccurrenceOrdinal(0), CfgPointId(6));
        assert_eq!(cloned.primary_call_point(OccurrenceOrdinal(0)), None);
        let encoded = serde_json::to_value(cfg).unwrap();
        assert_eq!(encoded.as_object().unwrap().len(), 4);
        let decoded: ScalarReferenceCfg = serde_json::from_value(encoded.clone()).unwrap();
        assert!(decoded.primary_call_points.is_empty());
        assert_eq!(&decoded, cfg);
        let legacy = derive_scalar_program_with_layout(&source, target_layout);
        assert!(legacy.diagnostics.is_empty(), "{:?}", legacy.diagnostics);
        let ScalarItem::Function(function) = &legacy.program.items[2] else {
            panic!("probe function")
        };
        assert!(function.reference_cfg.primary_call_points.is_empty());
        assert_eq!(
            serde_json::to_value(&function.reference_cfg).unwrap(),
            encoded
        );
    }
}
