use super::*;
use crate::parse_source;

#[test]
fn selected_saved_payload_receives_exact_source_before_replacement() {
    let source = SourceIdentity::new(
        "selected-saved-port-proof".into(),
        "app".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let text = "%%start\nunit() save = fn { u8 a = 1; u8 b = 2; *?u8 current = null; *?u8 replacement = null; unsafe { current = &?a; replacement = &?b; }; *?u8 old = (current = replacement); old; };\n%%end";
    let parsed = parse_source(source, text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) =
        derive_scalar_program_observed_test(&parsed.result, ScalarTargetLayout::NATIVE64);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let function = validation
        .program
        .items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "save" => Some(function),
            _ => None,
        })
        .expect("actual saved raw source function");
    let cfg = &function.reference_cfg;
    let pass = trace
        .passes
        .iter()
        .rfind(|pass| {
            pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
        })
        .expect("actual selected worker pass");
    let snapshots = pass
        .observations
        .points
        .iter()
        .filter(|snapshot| {
            snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
                && matches!(
                    cfg.initial_points[snapshot.point.0].kind,
                    CfgPointKind::AssignmentSnapshot { observed: true, .. }
                )
        })
        .collect::<Vec<_>>();
    assert!(!snapshots.is_empty());
    for snapshot in snapshots {
        let payloads = &snapshot.state.assignment_saved[&snapshot.point];
        assert!(!payloads.is_empty());
        for payload in payloads {
            assert!(
                !payload.selected.is_empty(),
                "actual membership attached before the replacement event"
            );
            for selected in &payload.selected {
                assert_eq!(selected.selection, payload.selection);
                assert_eq!(selected.snapshot.point, snapshot.point);
                let membership = selected_source::port_monitor::query(
                    &cfg.evaluated_values,
                    &selected.monitor,
                    std::slice::from_ref(&selected.selected.coordinate),
                    &cfg.initial_points[snapshot.point.0].source_span,
                )
                .expect("same worker tuple and frozen source port");
                assert_eq!(membership, vec![selected.selected.clone()]);
            }
        }
        let mut reporting_only = snapshot.state.clone();
        for payload in reporting_only
            .assignment_saved
            .get_mut(&snapshot.point)
            .expect("actual historical saved projection")
        {
            payload.selected.clear();
        }
        assert!(
            reporting_only == snapshot.state,
            "historical whole-state equality remains unchanged"
        );
    }
}
