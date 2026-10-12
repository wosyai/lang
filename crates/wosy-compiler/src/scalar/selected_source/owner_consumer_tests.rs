use super::*;

const PACKET: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() make_packet = fn { u64 length = 1; u8[length] bytes; bytes[0] = 4; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result };\n%%end";
const SAVED: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() saved_packet = fn { u64 length = 1; u8[length] first; u8[length] second; *?u8 current = null; *?u8 replacement = null; unsafe { current = &?first[0]; replacement = &?second[0]; }; *?u8 old = (current = replacement); Packet result = { .data = old; .length = length; }; &result };\n%%end";

fn packet(layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    tests::source_cfg(PACKET, "make_packet", layout, project)
}

fn prove_candidate(layout: ScalarTargetLayout, project: bool) {
    let cfg = packet(layout, project);
    validate(&cfg).expect("selected source tuple correspondence");
    let candidates = &cfg.evaluated_values.pending_owner_consumers;
    assert!(
        !candidates.is_empty(),
        "actual Packet owner consumer construction"
    );
    assert!(
        cfg.evaluated_values.owner_consumers.is_empty(),
        "incomplete finite premises carry zero lifetime certification"
    );
    for pending in candidates {
        let candidate = &pending.candidate;
        let tuple = &cfg.evaluated_values.source_contributors[candidate.contributor.index];
        let row = &cfg.evaluated_values.source_configurations[tuple.output.index];
        owner_consumer::validate_correspondence(
            &cfg.evaluated_values,
            &cfg.initial_points,
            row,
            candidate,
        )
        .expect("actual source/owner/endpoint correspondence");
        assert!(pending.requirements.iter().any(|requirement| matches!(
            requirement,
            OwnerConsumerRequirement::CompleteSourcePredicateProduct(_)
        )));
        assert!(pending.requirements.iter().any(|requirement| matches!(
            requirement,
            OwnerConsumerRequirement::FiniteSupportNormalization(_)
        )));
        assert!(pending.requirements.iter().any(|requirement| matches!(
            requirement,
            OwnerConsumerRequirement::FixedReturnMaterialization(_)
        )));
        assert!(candidate.dependencies.iter().any(|selection| matches!(
            selection.dependency.action,
            DependencyAction::TransferUnique { .. }
        ) && matches!(
            selection.projection.as_slice(),
            [ValueProjection::Field(_)]
        )));
    }
    let saved = tests::source_cfg(SAVED, "saved_packet", layout, project);
    assert!(
        saved
            .evaluated_values
            .source_configurations
            .iter()
            .any(|row| row.saved.iter().any(|old| {
                let SourcePayloadView::Raw {
                    referent: previous, ..
                } = &saved.evaluated_values.source_views[old.view.index].payload
                else {
                    return false;
                };
                let destination = saved
                    .evaluated_values
                    .destinations
                    .iter()
                    .find(|destination| destination.id == old.selection.destination)
                    .expect("actual frozen destination");
                let EvaluatedDestinationStorage::Direct(place) = &destination.storage else {
                    panic!("actual direct source destination");
                };
                row.slots.iter().any(|slot| {
                    match (
                        &saved.evaluated_values.source_slots[slot.coordinate.index].role,
                        &saved.evaluated_values.source_views[slot.view.index].payload,
                    ) {
                        (
                            SourceCoordinateRole::Binding(binding),
                            SourcePayloadView::Raw {
                                referent: replacement,
                                ..
                            },
                        ) if binding == &place.binding => previous != replacement,
                        _ => false,
                    }
                })
            })),
        "saved previous instance survives independently of replacement"
    );
}

#[test]
fn selected_owner_consumer_candidate_program_native64() {
    prove_candidate(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_owner_consumer_candidate_program_wasm32() {
    prove_candidate(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_owner_consumer_candidate_project_native64() {
    prove_candidate(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_owner_consumer_candidate_project_wasm32() {
    prove_candidate(ScalarTargetLayout::WASM32, true);
}

#[test]
fn selected_owner_consumer_crossed_payload_instance_is_rejected() {
    let cfg = packet(ScalarTargetLayout::NATIVE64, false);
    let mut candidate = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    candidate.dependencies[0].source_instance = candidate.dependencies[0].owner_instance.clone();
    let tuple = &cfg.evaluated_values.source_contributors[candidate.contributor.index];
    let row = &cfg.evaluated_values.source_configurations[tuple.output.index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &candidate
    )
    .is_err());
}

#[test]
fn selected_owner_consumer_crossed_source_owner_is_rejected() {
    let cfg = packet(ScalarTargetLayout::NATIVE64, false);
    let mut candidate = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    let DependencyAction::TransferUnique { source_owner, .. } =
        &mut candidate.dependencies[0].dependency.action
    else {
        panic!("automatic owned bytes");
    };
    *source_owner = candidate.receiving.clone();
    let tuple = &cfg.evaluated_values.source_contributors[candidate.contributor.index];
    let row = &cfg.evaluated_values.source_configurations[tuple.output.index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &candidate
    )
    .is_err());
}

#[test]
fn selected_owner_consumer_missing_endpoint_is_rejected() {
    let mut cfg = packet(ScalarTargetLayout::NATIVE64, false);
    let candidate = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    let tuple = cfg.evaluated_values.source_contributors[candidate.contributor.index].clone();
    cfg.evaluated_values.source_configurations[tuple.output.index]
        .owns
        .retain(|owns| owns.payload != candidate.dependencies[0].source_instance);
    let row = &cfg.evaluated_values.source_configurations[tuple.output.index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &candidate
    )
    .is_err());
}

#[test]
fn selected_owner_consumer_crossed_child_configuration_is_rejected() {
    let mut cfg = packet(ScalarTargetLayout::NATIVE64, false);
    let candidate = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    let mut row = cfg.evaluated_values.source_configurations[cfg
        .evaluated_values
        .source_contributors[candidate.contributor.index]
        .output
        .index]
        .clone();
    row.id.index = usize::MAX;
    cfg.evaluated_values.source_contributors[candidate.contributor.index].output = row.id.clone();
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        &row,
        &candidate
    )
    .is_err());
}
