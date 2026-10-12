use super::*;

const POLICIES: &str = "%%start\n*?u8() raw = fn { null };\n*u8() shared = fn { u8 value = 1; &value };\n*!u8() mutable = fn { u8 value = 1; &!value };\nu8[](u64) runtime = fn(count) { u8[count] bytes; bytes };\nu8[]() fixed_copy = fn { u8[2] bytes; bytes[0] = 1; bytes[1] = 2; bytes };\n(*!u8)[]() fixed_move = fn { u8 a = 1; u8 b = 2; (*!u8)[2] pointers; pointers[0] = &!a; pointers[1] = &!b; pointers };\n%%end";
const PACKET: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() make_packet = fn { u64 length = 1; u8[length] bytes; bytes[0] = 4; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result };\n%%end";
const SAVED: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() saved_packet = fn { u64 length = 1; u8[length] first; u8[length] second; *?u8 current = null; *?u8 replacement = null; unsafe { current = &?first[0]; replacement = &?second[0]; }; *?u8 old = (current = replacement); Packet result = { .data = old; .length = length; }; &result };\n%%end";

fn prove_policy(layout: ScalarTargetLayout, project: bool) {
    for (name, policy) in [
        ("raw", CopyPolicy::Copy),
        ("shared", CopyPolicy::Copy),
        ("mutable", CopyPolicy::Move),
        ("runtime", CopyPolicy::Move),
        ("fixed_copy", CopyPolicy::Copy),
        ("fixed_move", CopyPolicy::Move),
    ] {
        let cfg = tests::source_cfg(POLICIES, name, layout, project);
        let finals = cfg.evaluated_values.disposition_inputs.iter().filter(|input|
            matches!(input.consumer.kind, ValueConsumerKind::FinalOutput { .. })).collect::<Vec<_>>();
        let [input] = finals.as_slice() else {
            panic!("one real source final-output disposition");
        };
        for record in &cfg.evaluated_values.disposition_inputs {
            if record.consumer == input.consumer { continue; }
            let ValueConsumerKind::Binding { binding, projection } = &record.consumer.kind else { panic!("actual Binding policy role"); };
            assert!(projection.is_empty());
            let ports = cfg.evaluated_values.binding_inputs.iter().filter(|port| port.producer == record.consumer.producer
                && port.consumer == record.consumer.point && port.source == record.source).collect::<Vec<_>>();
            let [port] = ports.as_slice() else { panic!("actual Binding policy port singleton"); };
            assert_eq!(reference::binding_disposition(&cfg.evaluated_values, port, binding).unwrap(), record);
            assert_eq!(cfg.evaluated_values.disposition_inputs.iter().filter(|other| other.consumer == record.consumer).count(), 1);
            let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id == record.source).unwrap();
            assert_eq!(cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == fact.shape).count(), 1);
            assert!(matches!(&cfg.initial_points[record.consumer.point.0].kind, CfgPointKind::Bind { binding: actual } if actual == binding));
        }
        assert_eq!(input.policy, policy);
        let final_input = cfg
            .evaluated_values
            .final_inputs
            .iter()
            .find(|port| port.consumer == input.consumer)
            .expect("actual receiving conversion correspondence");
        assert_eq!(input.source, final_input.source);
        let expected = match policy {
            CopyPolicy::Copy => ValueDispositionAction::CopyTo(input.consumer.clone()),
            CopyPolicy::Move => ValueDispositionAction::MoveTo(input.consumer.clone()),
            CopyPolicy::Deferred => panic!("resolved concrete source"),
        };
        assert_eq!(
            owner_receiving::disposition(
                &cfg.evaluated_values,
                &input.source,
                &input.consumer,
                &cfg.initial_points[input.consumer.point.0].source_span
            )
            .expect("actual source policy"),
            Some(expected.clone())
        );
        let CfgPointKind::ReturnOutput { backing_action, .. } =
            &cfg.initial_points[input.consumer.point.0].kind
        else {
            panic!("actual return consumer");
        };
        match name {
            "fixed_copy" => assert!(matches!(
                backing_action,
                RuntimeReturnBackingAction::CopyFixed { .. }
            )),
            "fixed_move" => assert!(matches!(
                backing_action,
                RuntimeReturnBackingAction::MoveFixed { .. }
            )),
            _ => assert_eq!(*backing_action, RuntimeReturnBackingAction::Transfer),
        }
        for pending in &cfg.evaluated_values.pending_owner_consumers {
            assert_eq!(pending.candidate.disposition.action, expected);
        }
        if matches!(name, "raw" | "shared" | "mutable") {
            assert!(
                !cfg.evaluated_values.pending_owner_consumers.is_empty(),
                "actual resolved pointer disposition reaches its consuming draft"
            );
        }
        assert!(cfg.evaluated_values.owner_consumers.is_empty());
    }
}

fn prove_receiving(layout: ScalarTargetLayout, project: bool) {
    for (source, name) in [(PACKET, "make_packet"), (SAVED, "saved_packet")] {
        let cfg = tests::source_cfg(source, name, layout, project);
        let tables = &cfg.evaluated_values;
        assert!(!tables.owner_receiving_transitions.is_empty());
        assert!(!tables.pending_owner_consumers.is_empty());
        for pending in &tables.pending_owner_consumers {
            let consumer = &pending.candidate;
            let tuple = &tables.source_contributors[consumer.contributor.index];
            let output = &tables.source_configurations[tuple.output.index];
            owner_consumer::validate_correspondence(tables, &cfg.initial_points, output, consumer)
                .expect("actual input donor and output recipient evidence");
            assert!(
                matches!(
                    consumer.disposition.action,
                    ValueDispositionAction::CopyTo(_)
                ),
                "checked pointer bits copy independently of owned bytes"
            );
            let transition = tables
                .owner_receiving_transitions
                .iter()
                .find(|transition| {
                    transition.output == output.id
                        && transition.receiving == consumer.receiving
                        && transition.dependencies == consumer.dependencies
                })
                .expect("same monitored receiving transition");
            let input = &tables.source_configurations[transition.input.index];
            assert_eq!(input.created, output.created.iter().filter(|created| !matches!(created.role, SourceObjectRole::Owner(ref id) if *id == consumer.receiving)).cloned().collect::<Vec<_>>(), "transfer creates zero payload or allocation");
            let unique = transition
                .dependencies
                .iter()
                .filter(|dependency| {
                    matches!(
                        dependency.dependency.action,
                        DependencyAction::TransferUnique { .. }
                    )
                })
                .collect::<Vec<_>>();
            assert!(
                !unique.is_empty(),
                "Packet.data transitive automatic bytes duty"
            );
            for dependency in unique {
                let original = input
                    .owns
                    .iter()
                    .filter(|owns| {
                        owns.payload == dependency.source_instance
                            && owns.owner_instance == dependency.owner_instance
                            && owns.state == SourceObligationState::Outstanding
                    })
                    .collect::<Vec<_>>();
                assert!(!original.is_empty());
                for donor in original {
                    assert!(output.owns.iter().any(|owns| owns.payload == donor.payload
                        && owns.owner_instance == donor.owner_instance
                        && owns.capability == donor.capability
                        && owns.endpoint == donor.endpoint
                        && owns.state == SourceObligationState::Transferred));
                    for endpoint in &transition.endpoints {
                        assert!(output.owns.iter().any(|owns| owns.payload == donor.payload
                            && owns.owner_instance == transition.receiving_instance
                            && owns.capability == transition.receiving
                            && owns.endpoint == *endpoint
                            && owns.state == SourceObligationState::Outstanding));
                    }
                    assert_eq!(
                        output
                            .owns
                            .iter()
                            .filter(|owns| owns.payload == donor.payload
                                && owns.state == SourceObligationState::Outstanding)
                            .count(),
                        transition.endpoints.len(),
                        "one recipient duty with its complete endpoint alternatives"
                    );
                    let exits = tables
                        .source_configurations
                        .iter()
                        .filter(|row| {
                            row.owns.iter().any(|owns| {
                                owns.payload == donor.payload
                                    && owns.capability == donor.capability
                                    && owns.state == SourceObligationState::Transferred
                            }) && tables.source_contributors.iter().any(|rule| {
                                rule.output == row.id && rule.event == donor.endpoint.event
                            })
                        })
                        .collect::<Vec<_>>();
                    assert!(!exits.is_empty(), "actual donor ScopeExit transition");
                    assert!(
                        exits.iter().all(|row| row
                            .owns
                            .iter()
                            .any(|owns| owns.payload == donor.payload
                                && owns.capability == transition.receiving
                                && owns.state == SourceObligationState::Outstanding)),
                        "result duty survives donor cleanup"
                    );
                }
                assert!(output
                    .owner_views
                    .iter()
                    .any(|view| view.payload == dependency.source_instance
                        && view.capability == transition.receiving
                        && view.owner_instance == transition.receiving_instance));
                if name == "saved_packet" {
                    assert!(
                        input.saved.iter().any(|saved| {
                            let SourcePayloadView::Raw { referent: old, .. } =
                                &tables.source_views[saved.view.index].payload
                            else {
                                return false;
                            };
                            *old == dependency.source_instance
                                && input.owns.iter().any(|owns| {
                                    owns.payload != *old
                                        && owns.kind == RuntimeBackingKind::OwnedAllocation
                                        && owns.state == SourceObligationState::Outstanding
                                        && output.owns.contains(owns)
                                })
                        }),
                        "saved previous bytes duty transfers independently of replacement"
                    );
                }
            }
            assert!(pending.requirements.contains(
                &OwnerConsumerRequirement::RemainingDependencyUses(transition.input.clone())
            ));
        }
        assert!(tables.owner_consumers.is_empty());
    }
}

#[test]
fn return_source_disposition_program_native64() {
    prove_policy(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn return_source_disposition_program_wasm32() {
    prove_policy(ScalarTargetLayout::WASM32, false);
}
#[test]
fn return_source_disposition_project_native64() {
    prove_policy(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn return_source_disposition_project_wasm32() {
    prove_policy(ScalarTargetLayout::WASM32, true);
}
#[test]
fn return_receiving_conservation_program_native64() {
    prove_receiving(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn return_receiving_conservation_program_wasm32() {
    prove_receiving(ScalarTargetLayout::WASM32, false);
}
#[test]
fn return_receiving_conservation_project_native64() {
    prove_receiving(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn return_receiving_conservation_project_wasm32() {
    prove_receiving(ScalarTargetLayout::WASM32, true);
}

fn packet() -> ScalarReferenceCfg {
    tests::source_cfg(PACKET, "make_packet", ScalarTargetLayout::NATIVE64, false)
}

#[test]
fn return_receiving_wrong_owner_instance_is_rejected() {
    let mut cfg = packet();
    let consumer = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    let transition = cfg
        .evaluated_values
        .owner_receiving_transitions
        .iter_mut()
        .find(|transition| transition.receiving == consumer.receiving)
        .expect("actual receiver transition");
    transition.receiving_instance = transition.dependencies[0].owner_instance.clone();
    let row = &cfg.evaluated_values.source_configurations[cfg.evaluated_values.source_contributors
        [consumer.contributor.index]
        .output
        .index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &consumer
    )
    .is_err());
}

#[test]
fn return_receiving_wrong_endpoint_is_rejected() {
    let mut cfg = packet();
    let consumer = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    let transition = cfg
        .evaluated_values
        .owner_receiving_transitions
        .iter_mut()
        .find(|transition| transition.receiving == consumer.receiving)
        .expect("actual receiving endpoint");
    transition.endpoints.clear();
    let row = &cfg.evaluated_values.source_configurations[cfg.evaluated_values.source_contributors
        [consumer.contributor.index]
        .output
        .index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &consumer
    )
    .is_err());
}

#[test]
fn return_receiving_pointer_move_cannot_replace_resolved_copy_disposition() {
    let cfg = packet();
    let mut consumer = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    consumer.disposition.action = ValueDispositionAction::MoveTo(consumer.consumer.clone());
    let row = &cfg.evaluated_values.source_configurations[cfg.evaluated_values.source_contributors
        [consumer.contributor.index]
        .output
        .index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &consumer
    )
    .is_err());
}

#[test]
fn return_receiving_changed_source_policy_is_rejected() {
    let mut cfg = packet();
    let consumer = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    cfg.evaluated_values
        .disposition_inputs
        .iter_mut()
        .find(|input| input.consumer == consumer.consumer)
        .expect("original checked source policy")
        .policy = CopyPolicy::Move;
    let row = &cfg.evaluated_values.source_configurations[cfg.evaluated_values.source_contributors
        [consumer.contributor.index]
        .output
        .index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &consumer
    )
    .is_err());
}

#[test]
fn return_receiving_duplicate_unique_donor_duty_is_rejected() {
    let mut cfg = packet();
    let consumer = cfg.evaluated_values.pending_owner_consumers[0]
        .candidate
        .clone();
    let row = &mut cfg.evaluated_values.source_configurations[cfg
        .evaluated_values
        .source_contributors[consumer.contributor.index]
        .output
        .index];
    let mut duplicate = row
        .owns
        .iter()
        .find(|owns| {
            owns.payload == consumer.dependencies[0].source_instance
                && owns.state == SourceObligationState::Transferred
        })
        .expect("transferred donor duty")
        .clone();
    duplicate.state = SourceObligationState::Outstanding;
    row.owns.push(duplicate);
    let row = &cfg.evaluated_values.source_configurations[cfg.evaluated_values.source_contributors
        [consumer.contributor.index]
        .output
        .index];
    assert!(owner_consumer::validate_correspondence(
        &cfg.evaluated_values,
        &cfg.initial_points,
        row,
        &consumer
    )
    .is_err());
}

#[test]
fn return_receiving_deferred_policy_remains_unresolved() {
    let mut cfg = packet();
    let [final_input] = cfg.evaluated_values.final_inputs.as_slice() else { panic!("actual final input singleton"); };
    let matches = cfg.evaluated_values.disposition_inputs.iter().enumerate().filter(|(_, input)|
        input.source == final_input.source && input.consumer == final_input.consumer).collect::<Vec<_>>();
    let [(index, original)] = matches.as_slice() else { panic!("actual final policy singleton"); };
    let input = (*original).clone();
    let index = *index;
    cfg.evaluated_values.disposition_inputs[index].policy = CopyPolicy::Deferred;
    assert!(owner_receiving::disposition(
        &cfg.evaluated_values,
        &input.source,
        &input.consumer,
        &cfg.initial_points[input.consumer.point.0].source_span
    )
    .expect("explicit specialization premise")
    .is_none());
}
