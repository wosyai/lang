use super::*;

#[test]
fn selected_move_declared_member_preserves_other_subplaces() {
    let cfg = tests::source_cfg("%%start\nstruct Pair { *!u8 pointer; u8 number; }\n*!u8() make = fn { u8 value = 1; Pair pair = { .pointer = &!value; .number = 2; }; pair.pointer };\n%%end", "make", ScalarTargetLayout::NATIVE64, false);
    let tables = &cfg.evaluated_values;
    let witnesses = tables
        .source_move_witnesses
        .iter()
        .filter(|witness| !witness.place.projections.is_empty())
        .collect::<Vec<_>>();
    let [witness] = witnesses.as_slice() else {
        panic!("exact declared member move");
    };
    selected_move::validate(
        tables,
        &cfg.initial_points,
        &witness.id,
        &cfg.initial_points[witness.event.point.0].source_span,
    )
    .expect("structural projected move");
    let input = &tables.source_configurations[witness.input.index];
    let output = &tables.source_configurations[witness.output.index];
    let before = selected_slot(
        tables,
        input,
        &SourceCoordinateRole::Binding(witness.place.binding.clone()),
    )
    .expect("original aggregate");
    let after = selected_slot(
        tables,
        output,
        &SourceCoordinateRole::Binding(witness.place.binding.clone()),
    )
    .expect("same aggregate with member availability");
    assert_eq!(before.view, after.view);
    let ValueAvailability::PartiallyUnavailable { states } = &after.availability else {
        panic!("member-only move");
    };
    let [state] = states.as_slice() else {
        panic!("one exact member state");
    };
    let [ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field))] =
        witness.place.projections.as_slice()
    else {
        panic!("declared source field");
    };
    assert_eq!(
        state.projection,
        vec![ValueProjection::Field(field.clone())]
    );
    assert_eq!(
        state.state,
        SubplaceState::Moved {
            point: witness.event.point,
            source: cfg.initial_points[witness.event.point.0]
                .source_span
                .clone()
        }
    );
}

#[test]
fn selected_move_copy_source_has_zero_move_witnesses() {
    let cfg = tests::source_cfg(
        "%%start\n*u8() make = fn { u8 value = 1; *u8 pointer = &value; pointer };\n%%end",
        "make",
        ScalarTargetLayout::NATIVE64,
        false,
    );
    assert!(cfg.evaluated_values.source_move_inputs.is_empty());
    assert!(cfg.evaluated_values.source_move_witnesses.is_empty());
}

const MUTABLE: &str =
    "%%start\n*!u8() make = fn { u8 value = 1; *!u8 pointer = &!value; pointer };\n%%end";
const RUNTIME: &str = "%%start\nu8[](u64) make = fn(count) { u8[count] bytes; bytes };\n%%end";
const SAVED: &str = "%%start\n*!u8() make = fn { u8 left = 1; u8 right = 2; *!u8 current = &!left; *!u8 replacement = &!right; *!u8 old = (current = replacement); old };\n%%end";

fn mutable(layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    tests::source_cfg(MUTABLE, "make", layout, project)
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let cfg = mutable(layout, project);
    let tables = &cfg.evaluated_values;
    assert!(!tables.source_move_witnesses.is_empty());
    for witness in &tables.source_move_witnesses {
        selected_move::validate(
            tables,
            &cfg.initial_points,
            &witness.id,
            &cfg.initial_points[witness.event.point.0].source_span,
        )
        .expect("actual structural move");
        let input = &tables.source_configurations[witness.input.index];
        let output = &tables.source_configurations[witness.output.index];
        let original = selected_slot(
            tables,
            input,
            &SourceCoordinateRole::Binding(witness.place.binding.clone()),
        )
        .expect("input storage");
        let storage = selected_slot(
            tables,
            output,
            &SourceCoordinateRole::Binding(witness.place.binding.clone()),
        )
        .expect("moved storage");
        let value = selected_slot(
            tables,
            output,
            &SourceCoordinateRole::Evaluated(witness.source.clone()),
        )
        .expect("available evaluated payload");
        assert_eq!(original.availability, ValueAvailability::Available);
        assert_eq!(value.availability, ValueAvailability::Available);
        assert_eq!(
            storage.availability,
            ValueAvailability::Moved {
                place: witness.place.clone(),
                move_point: witness.event.point,
                source: cfg.initial_points[witness.event.point.0]
                    .source_span
                    .clone()
            }
        );
        assert_eq!(original.view, storage.view);
        assert_eq!(original.view, value.view);
        assert_eq!(input.created, output.created);
        assert_eq!(input.instances, output.instances);
        assert_eq!(input.owns, output.owns);
        assert!(!witness.selection.moves.contains(&witness.id));
        let equation = tables
            .source_port_equations
            .iter()
            .find(|equation| {
                equation.contributor == witness.contributor && equation.moves.contains(&witness.id)
            })
            .expect("same contributor equation");
        let monitor = &tables.source_port_monitors[equation.output.index];
        assert!(monitor.pending.contains(&storage.coordinate));
        let selections = port_monitor::query(
            tables,
            &monitor.id,
            std::slice::from_ref(&value.coordinate),
            &cfg.initial_points[witness.event.point.0].source_span,
        )
        .expect("available evaluated membership");
        let [membership] = selections.as_slice() else {
            panic!("exact evaluated port");
        };
        assert!(membership.moves.contains(&witness.id));
        assert_eq!(membership.observation, witness.selection.observation);
    }
    assert!(!tables.pending_owner_consumers.is_empty());
    for pending in &tables.pending_owner_consumers {
        assert!(pending
            .requirements
            .contains(&OwnerConsumerRequirement::SourceMoveWitness(
                pending.candidate.input.value.clone()
            )));
        let row = &tables.source_configurations[tables.source_contributors
            [pending.candidate.contributor.index]
            .output
            .index];
        owner_consumer::validate_correspondence(
            tables,
            &cfg.initial_points,
            row,
            &pending.candidate,
        )
        .expect("return structural reader");
        let value = selected_slot(
            tables,
            row,
            &SourceCoordinateRole::Evaluated(pending.candidate.input.value.clone()),
        )
        .expect("actual final source");
        let membership = port_monitor::selection(
            tables,
            &pending.monitor,
            &value.coordinate,
            &cfg.initial_points[pending.candidate.consumer.point.0].source_span,
        )
        .expect("monitored final source")
        .expect("available final value");
        assert!(!membership.moves.is_empty());
    }
    assert!(tables.owner_consumers.is_empty());
}

fn prove_runtime_pending(layout: ScalarTargetLayout, project: bool) {
    let cfg = tests::source_cfg(RUNTIME, "make", layout, project);
    let tables = &cfg.evaluated_values;
    backing_producer::validate(&cfg).expect("actual canonical storage creation");
    let [producer] = tables.source_backing_producers.as_slice() else {
        panic!("one actual Backing producer");
    };
    let point = &cfg.initial_points[producer.event.point.0];
    assert_eq!(producer.value.point, point.id);
    assert_eq!(producer.value.resolved.occurrence, point.source_span);
    assert_eq!(producer.value.resolved.producer, producer.event.owner);
    assert_eq!(producer.value.resolved.output, 0);
    assert!(producer.value.resolved.output_projection.is_empty());
    let CfgPointKind::Backing {
        target,
        transfer: RuntimeBackingTransfer::Allocate,
        ..
    } = &point.kind
    else {
        panic!("original Allocate storage event");
    };
    assert_eq!(*target, producer.target);
    let schemas = tables
        .source_binding_shapes
        .iter()
        .filter(|schema| schema.binding == producer.target)
        .collect::<Vec<_>>();
    let [schema] = schemas.as_slice() else {
        panic!("one actual declared shape");
    };
    assert_eq!(schema.shape, producer.shape);
    let creations = tables
        .source_contributors
        .iter()
        .filter(|rule| rule.event == producer.event)
        .collect::<Vec<_>>();
    assert!(!creations.is_empty());
    for creation in creations {
        let row = &tables.source_configurations[creation.output.index];
        let evaluated = selected_slot(
            tables,
            row,
            &SourceCoordinateRole::Evaluated(producer.value.clone()),
        )
        .expect("evaluated creation port");
        let binding = selected_slot(
            tables,
            row,
            &SourceCoordinateRole::Binding(producer.target.clone()),
        )
        .expect("actual binding port");
        let equations = tables
            .source_port_equations
            .iter()
            .filter(|equation| equation.contributor == creation.id)
            .collect::<Vec<_>>();
        assert!(!equations.is_empty());
        for equation in equations {
            let ports = port_monitor::query(
                tables,
                &equation.output,
                &[evaluated.coordinate.clone(), binding.coordinate.clone()],
                &point.source_span,
            )
            .expect("ONE joint creation valuation");
            let [created, stored] = ports.as_slice() else {
                panic!("two exact production ports");
            };
            assert_eq!(created.observation.case, stored.observation.case);
            assert_eq!(created.observation.path, stored.observation.path);
            assert_eq!(created.observation.contributor, creation.id);
            assert_eq!(stored.observation.contributor, creation.id);
            assert_eq!(created.observation.configuration, creation.output);
            assert_eq!(stored.observation.configuration, creation.output);
            assert_eq!(created.observation.view, stored.observation.view);
            assert_eq!(stored.observation.case.value, producer.value);
            let SourcePayloadView::Backing { instance, .. } =
                &tables.source_views[stored.observation.view.index].payload
            else {
                panic!("actual created payload");
            };
            assert!(row.created.iter().any(|fact| fact.instance == *instance
                && fact.event == producer.event
                && fact.role == SourceObjectRole::Payload));
            let witnesses = tables
                .source_move_witnesses
                .iter()
                .filter(|witness| witness.selection.sources.contains(&stored.observation))
                .collect::<Vec<_>>();
            assert!(
                !witnesses.is_empty(),
                "Read retains exact original binding leaf"
            );
            for witness in witnesses {
                assert_ne!(witness.source, producer.value);
                assert_eq!(witness.selection.observation.case.value, witness.source);
                assert_eq!(witness.place.binding, producer.target);
                assert!(
                    matches!(&cfg.initial_points[witness.source.point.0].kind, CfgPointKind::Read { binding } if *binding == producer.target)
                );
                selected_move::validate(
                    tables,
                    &cfg.initial_points,
                    &witness.id,
                    &cfg.initial_points[witness.event.point.0].source_span,
                )
                .expect("exact source/input/output/contributor/place move evidence");
                let before = &tables.source_configurations[witness.input.index];
                let after = &tables.source_configurations[witness.output.index];
                let value = selected_slot(
                    tables,
                    after,
                    &SourceCoordinateRole::Evaluated(witness.source.clone()),
                )
                .expect("available evaluated source");
                let storage = selected_slot(
                    tables,
                    after,
                    &SourceCoordinateRole::Binding(producer.target.clone()),
                )
                .expect("original moved storage");
                assert_eq!(value.availability, ValueAvailability::Available);
                assert_eq!(
                    storage.availability,
                    ValueAvailability::Moved {
                        place: witness.place.clone(),
                        move_point: witness.event.point,
                        source: cfg.initial_points[witness.event.point.0]
                            .source_span
                            .clone()
                    }
                );
                assert_eq!(value.view, stored.observation.view);
                assert_eq!(before.created, after.created);
                assert_eq!(before.instances, after.instances);
                let moved = tables
                    .source_contributors
                    .iter()
                    .filter(|rule| rule.id == witness.contributor)
                    .collect::<Vec<_>>();
                let [moved] = moved.as_slice() else {
                    panic!("same actual Move contributor");
                };
                assert_eq!(moved.input, witness.input);
                assert_eq!(moved.output, witness.output);
                assert_eq!(moved.event, witness.event);
            }
        }
    }
    assert!(!tables.pending_owner_consumers.is_empty());
    for pending in &tables.pending_owner_consumers {
        let consumer = &pending.candidate;
        let rule = &tables.source_contributors[consumer.contributor.index];
        let output = &tables.source_configurations[rule.output.index];
        owner_consumer::validate_correspondence(tables, &cfg.initial_points, output, consumer)
            .expect("same monitored ReturnOutput tuple");
        let transitions = tables
            .owner_receiving_transitions
            .iter()
            .filter(|transition| {
                transition.event == rule.event
                    && transition.input == rule.input
                    && transition.output == rule.output
                    && transition.receiving == consumer.receiving
                    && transition.dependencies == consumer.dependencies
            })
            .collect::<Vec<_>>();
        let [transition] = transitions.as_slice() else {
            panic!("one exact receiving transition");
        };
        let input = &tables.source_configurations[transition.input.index];
        assert_eq!(
            output
                .created
                .iter()
                .filter(|fact| fact.role == SourceObjectRole::Payload)
                .collect::<Vec<_>>(),
            input
                .created
                .iter()
                .filter(|fact| fact.role == SourceObjectRole::Payload)
                .collect::<Vec<_>>()
        );
        let [selection] = transition.dependencies.as_slice() else {
            panic!("one actual unique payload dependency");
        };
        let DependencyAction::TransferUnique {
            source_owner,
            receiving_owner,
        } = &selection.dependency.action
        else {
            panic!("actual unique receiving duty");
        };
        assert_eq!(*receiving_owner, transition.receiving);
        let donors = input
            .owns
            .iter()
            .filter(|owns| {
                owns.payload == selection.source_instance
                    && owns.capability == *source_owner
                    && owns.owner_instance == selection.owner_instance
                    && owns.state == SourceObligationState::Outstanding
            })
            .collect::<Vec<_>>();
        assert!(!donors.is_empty());
        for donor in donors {
            assert!(output.owns.iter().any(|owns| owns.payload == donor.payload
                && owns.capability == donor.capability
                && owns.owner_instance == donor.owner_instance
                && owns.endpoint == donor.endpoint
                && owns.state == SourceObligationState::Transferred));
        }
        assert!(!transition.endpoints.is_empty());
        for endpoint in &transition.endpoints {
            assert!(output
                .owns
                .iter()
                .any(|owns| owns.payload == selection.source_instance
                    && owns.capability == transition.receiving
                    && owns.owner_instance == transition.receiving_instance
                    && owns.endpoint == *endpoint
                    && owns.state == SourceObligationState::Outstanding));
        }
        assert_eq!(
            output
                .owns
                .iter()
                .filter(|owns| owns.payload == selection.source_instance
                    && owns.state == SourceObligationState::Outstanding)
                .count(),
            transition.endpoints.len()
        );
        assert!(pending
            .requirements
            .contains(&OwnerConsumerRequirement::SourceMoveWitness(
                consumer.input.value.clone()
            )));
        assert!(pending
            .requirements
            .contains(&OwnerConsumerRequirement::RemainingDependencyUses(
                transition.input.clone()
            )));
        assert!(pending.requirements.contains(
            &OwnerConsumerRequirement::CompleteSourcePredicateProduct(transition.output.clone())
        ));
        assert!(pending.requirements.contains(
            &OwnerConsumerRequirement::FiniteSupportNormalization(transition.output.clone())
        ));
    }
    assert!(tables.owner_consumers.is_empty());
}

#[test]
fn selected_move_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_move_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_move_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_move_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
#[test]
fn selected_move_runtime_pending_program_native64() {
    prove_runtime_pending(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_move_runtime_pending_program_wasm32() {
    prove_runtime_pending(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_move_runtime_pending_project_native64() {
    prove_runtime_pending(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_move_runtime_pending_project_wasm32() {
    prove_runtime_pending(ScalarTargetLayout::WASM32, true);
}

fn rejection(cfg: &ScalarReferenceCfg) {
    assert!(selected_move::validate(
        &cfg.evaluated_values,
        &cfg.initial_points,
        &cfg.evaluated_values.source_move_witnesses[0].id,
        &cfg.initial_points[0].source_span
    )
    .is_err());
}

#[test]
fn selected_move_wrong_source_is_rejected() {
    let mut cfg = mutable(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_move_witnesses[0].source =
        cfg.evaluated_values.values[0].id.clone();
    rejection(&cfg);
}
#[test]
fn selected_move_wrong_event_is_rejected() {
    let mut cfg = mutable(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_move_witnesses[0].event.point = CfgPointId(0);
    rejection(&cfg);
}
#[test]
fn selected_move_wrong_contributor_is_rejected() {
    let mut cfg = mutable(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_move_witnesses[0].contributor =
        cfg.evaluated_values.source_contributors[0].id.clone();
    rejection(&cfg);
}
#[test]
fn selected_move_wrong_place_is_rejected() {
    let mut cfg = mutable(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_move_witnesses[0].place.place = ScalarPlace::Name {
        name: "wrong".into(),
        span: cfg.initial_points[0].source_span.range,
    };
    rejection(&cfg);
}
#[test]
fn selected_move_wrong_available_payload_is_rejected() {
    let mut cfg = mutable(ScalarTargetLayout::NATIVE64, false);
    let witness = cfg.evaluated_values.source_move_witnesses[0].clone();
    cfg.evaluated_values.source_configurations[witness.output.index]
        .slots
        .iter_mut()
        .find(|slot| slot.coordinate == witness.selection.coordinate)
        .expect("evaluated payload slot")
        .availability = ValueAvailability::Moved {
        place: witness.place,
        move_point: witness.event.point,
        source: cfg.initial_points[witness.event.point.0]
            .source_span
            .clone(),
    };
    rejection(&cfg);
}
#[test]
fn selected_move_self_capture_is_rejected() {
    let mut cfg = mutable(ScalarTargetLayout::NATIVE64, false);
    let id = cfg.evaluated_values.source_move_witnesses[0].id.clone();
    cfg.evaluated_values.source_move_witnesses[0]
        .selection
        .moves
        .push(id);
    rejection(&cfg);
}
#[test]
fn selected_move_saved_old_preserves_original_membership() {
    let cfg = tests::source_cfg(SAVED, "make", ScalarTargetLayout::NATIVE64, false);
    let tables = &cfg.evaluated_values;
    assert!(!tables.source_saved_ports.is_empty());
    for saved in &tables.source_saved_ports {
        let results = tables
            .source_port_monitors
            .iter()
            .flat_map(|monitor| &monitor.ports)
            .filter(|port| {
                port.sources.contains(&saved.selected.observation)
                    && tables.source_slots[port.coordinate.index].role
                        != tables.source_slots[saved.selected.coordinate.index].role
            })
            .collect::<Vec<_>>();
        assert!(!results.is_empty());
        for port in results {
            assert!(saved
                .selected
                .moves
                .iter()
                .all(|id| port.moves.contains(id)));
        }
    }
    assert!(tables.owner_consumers.is_empty());
}
