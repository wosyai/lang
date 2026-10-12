use super::*;
use crate::parse_source;

#[test]
fn same_historical_state_new_selected_contributor_is_admitted() {
    let source = SourceIdentity::new(
        "selected-reach-proof".into(),
        "app".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(
        source,
        "%%start\nunit() inspect = fn { u64 value = 1; value; };\n%%end".to_owned(),
        &[],
    );
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
            ScalarItem::Function(function) if function.name == "inspect" => Some(function),
            _ => None,
        })
        .expect("actual function");
    let cfg = &function.reference_cfg;
    let pass = trace
        .passes
        .iter()
        .rfind(|pass| {
            pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
        })
        .expect("actual selected worker pass");
    let state = pass
        .observations
        .points
        .iter()
        .find(|snapshot| {
            snapshot.point == CfgPointId(0)
                && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
        })
        .expect("actual incoming whole state")
        .state
        .clone();
    let roots = cfg
        .evaluated_values
        .source_contributors
        .iter()
        .filter(|tuple| matches!(tuple.kind, JointProducerRuleKind::Root))
        .collect::<Vec<_>>();
    let [first, second] = roots.as_slice() else {
        panic!("formal Root and actual Entry transition");
    };
    assert_ne!(first.id, second.id);
    assert_eq!(first.output, second.output);
    let work = |tuple: &JointProducerTupleFact| ReferenceWorkItem {
        state: state.clone(),
        selected: selected_source::SelectedSourceState::Active {
            configuration: tuple.output.clone(),
            contributor: tuple.id.clone(),
            predecessor: tuple.event.clone(),
            monitor: cfg
                .evaluated_values
                .source_port_monitors
                .iter()
                .find(|monitor| monitor.contributor == tuple.id)
                .expect("actual selected port valuation")
                .id
                .clone(),
        },
    };
    let mut historical = Vec::new();
    assert!(reference_state::admit(&mut historical, &state));
    assert!(!reference_state::admit(&mut historical, &state));
    assert_eq!(historical.len(), 1);
    let mut selected = Vec::new();
    assert!(admit_reference_work_item(&mut selected, work(first)));
    assert!(admit_reference_work_item(&mut selected, work(second)));
    assert!(!admit_reference_work_item(&mut selected, work(second)));
    assert_eq!(selected.len(), 2);
}
