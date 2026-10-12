use super::*;

const RUNTIME: &str = "%%start\nu8[](u64) make = fn(count) { u8[count] bytes; bytes };\n%%end";

fn cfg(layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    tests::source_cfg(RUNTIME, "make", layout, project)
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let cfg = cfg(layout, project);
    let tables = &cfg.evaluated_values;
    backing_producer::validate(&cfg).expect("actual canonical backing producer");
    let [producer] = tables.source_backing_producers.as_slice() else {
        panic!("one original allocation event");
    };
    assert_eq!(producer.value.point, producer.event.point);
    assert_eq!(
        producer.value.resolved.occurrence,
        cfg.initial_points[producer.event.point.0].source_span
    );
    assert_eq!(producer.value.resolved.output, 0);
    assert!(producer.value.resolved.output_projection.is_empty());
    let rules = tables
        .source_contributors
        .iter()
        .filter(|rule| rule.event == producer.event)
        .collect::<Vec<_>>();
    assert!(!rules.is_empty());
    for rule in rules {
        let row = &tables.source_configurations[rule.output.index];
        let input = &tables.source_configurations[rule.input.index];
        let evaluated = selected_slot(
            tables,
            row,
            &SourceCoordinateRole::Evaluated(producer.value.clone()),
        )
        .expect("real evaluated creation");
        let binding = selected_slot(
            tables,
            row,
            &SourceCoordinateRole::Binding(producer.target.clone()),
        )
        .expect("real target binding");
        let observations = tables
            .source_case_views
            .iter()
            .filter(|observation| {
                observation.contributor == rule.id && observation.case.value == producer.value
            })
            .collect::<Vec<_>>();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].case, observations[1].case);
        assert_eq!(observations[0].path, observations[1].path);
        assert_eq!(observations[0].view, observations[1].view);
        assert_eq!(evaluated.view, binding.view);
        assert_eq!(rule.productions.len(), 2);
        assert!(rule
            .productions
            .iter()
            .all(|production| production.inputs.is_empty()));
        assert!(rule
            .transports
            .iter()
            .all(|transport| transport.destination != evaluated.coordinate
                && transport.destination != binding.coordinate));
        let case = &tables
            .values
            .iter()
            .find(|value| value.id == producer.value)
            .expect("registered original creation")
            .cases[observations[0].case.alternative];
        assert!(matches!(
            case.payload,
            EvaluatedPayloadFact::RuntimeBacking { .. }
        ));
        let [dependency] = case.dependencies.as_slice() else {
            panic!("genuine allocation dependency");
        };
        assert_eq!(
            dependency.source,
            DependencySource::Allocation(OwnerRecordFamilyId {
                producer: producer.event.owner.clone(),
                creation: producer.event.point,
                projection: Vec::new()
            })
        );
        assert_eq!(
            dependency.witness,
            cfg.initial_points[producer.event.point.0].source_span
        );
        assert!(matches!(
            dependency.action,
            DependencyAction::TransferUnique { .. }
        ));
        let SourcePayloadView::Backing { instance, .. } =
            &tables.source_views[binding.view.index].payload
        else {
            panic!("same actual payload");
        };
        assert_eq!(
            row.created
                .iter()
                .filter(|created| created.role == SourceObjectRole::Payload)
                .count(),
            input
                .created
                .iter()
                .filter(|created| created.role == SourceObjectRole::Payload)
                .count()
                + 1
        );
        assert_eq!(
            row.created
                .iter()
                .filter(|created| created.event == producer.event
                    && created.instance == *instance
                    && created.role == SourceObjectRole::Payload)
                .count(),
            1
        );
        let equation = tables
            .source_port_equations
            .iter()
            .find(|equation| equation.contributor == rule.id)
            .expect("creation equation");
        let selected = port_monitor::query(
            tables,
            &equation.output,
            &[evaluated.coordinate.clone(), binding.coordinate.clone()],
            &cfg.initial_points[producer.event.point.0].source_span,
        )
        .expect("joint two-port creation");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].observation.case, selected[1].observation.case);
        let binding_observation = &selected[1].observation;
        let witnesses = tables
            .source_move_witnesses
            .iter()
            .filter(|witness| witness.selection.sources.contains(binding_observation))
            .collect::<Vec<_>>();
        assert!(
            !witnesses.is_empty(),
            "Read consumes original binding leaf before real Move"
        );
        for witness in witnesses {
            selected_move::validate(
                tables,
                &cfg.initial_points,
                &witness.id,
                &cfg.initial_points[witness.event.point.0].source_span,
            )
            .expect("actual runtime move witness");
            assert_ne!(witness.source, producer.value);
            assert_eq!(witness.selection.observation.view, binding.view);
            assert_eq!(witness.place.binding, producer.target);
            let output = &tables.source_configurations[witness.output.index];
            let value = selected_slot(
                tables,
                output,
                &SourceCoordinateRole::Evaluated(witness.source.clone()),
            )
            .expect("available consumed result");
            let storage = selected_slot(
                tables,
                output,
                &SourceCoordinateRole::Binding(producer.target.clone()),
            )
            .expect("moved donor storage");
            assert_eq!(value.availability, ValueAvailability::Available);
            assert!(matches!(
                storage.availability,
                ValueAvailability::Moved { .. }
            ));
        }
    }
    assert!(!tables.pending_owner_consumers.is_empty());
    for pending in &tables.pending_owner_consumers {
        let consumer = &pending.candidate;
        let row = &tables.source_configurations[tables.source_contributors
            [consumer.contributor.index]
            .output
            .index];
        owner_consumer::validate_correspondence(tables, &cfg.initial_points, row, consumer)
            .expect("same selected return tuple");
        assert!(matches!(
            consumer.disposition.action,
            ValueDispositionAction::MoveTo(_)
        ));
        let [dependency] = consumer.dependencies.as_slice() else {
            panic!("one same-instance unique duty");
        };
        let transition = tables
            .owner_receiving_transitions
            .iter()
            .find(|transition| {
                transition.output == row.id
                    && transition.receiving == consumer.receiving
                    && transition.dependencies == consumer.dependencies
            })
            .expect("actual receiver transition");
        let donor = &tables.source_configurations[transition.input.index];
        assert!(donor
            .owns
            .iter()
            .any(|owns| owns.payload == dependency.source_instance
                && owns.owner_instance == dependency.owner_instance
                && owns.state == SourceObligationState::Outstanding));
        assert!(row
            .owns
            .iter()
            .any(|owns| owns.payload == dependency.source_instance
                && owns.owner_instance == dependency.owner_instance
                && owns.state == SourceObligationState::Transferred));
        assert_eq!(
            row.owns
                .iter()
                .filter(|owns| owns.payload == dependency.source_instance
                    && owns.capability == transition.receiving
                    && owns.owner_instance == transition.receiving_instance
                    && owns.state == SourceObligationState::Outstanding)
                .count(),
            transition.endpoints.len()
        );
        assert_eq!(
            row.created
                .iter()
                .filter(|created| created.role == SourceObjectRole::Payload)
                .count(),
            donor
                .created
                .iter()
                .filter(|created| created.role == SourceObjectRole::Payload)
                .count()
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
            &OwnerConsumerRequirement::FiniteSupportNormalization(row.id.clone())
        ));
    }
    assert!(tables.owner_consumers.is_empty());
}

#[test]
fn backing_source_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn backing_source_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn backing_source_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn backing_source_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}

#[test]
fn backing_source_wrong_event_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_backing_producers[0].event.point = CfgPointId(0);
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_wrong_target_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_backing_producers[0].target = cfg
        .evaluated_values
        .source_binding_shapes
        .iter()
        .find(|shape| shape.binding != cfg.evaluated_values.source_backing_producers[0].target)
        .expect("independent length parameter")
        .binding
        .clone();
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_wrong_shape_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_backing_producers[0].shape = cfg
        .evaluated_values
        .shapes
        .iter()
        .find(|shape| shape.id != cfg.evaluated_values.source_backing_producers[0].shape)
        .expect("independent scalar shape")
        .id
        .clone();
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_wrong_evaluated_source_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    cfg.evaluated_values.source_backing_producers[0].value =
        cfg.evaluated_values.source_move_inputs[0].source.clone();
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_duplicate_output_port_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let source = cfg.evaluated_values.source_backing_producers[0]
        .value
        .clone();
    let ports = cfg
        .evaluated_values
        .source_case_views
        .iter()
        .filter(|port| port.case.value == source)
        .cloned()
        .collect::<Vec<_>>();
    let [evaluated, binding] = ports.as_slice() else {
        panic!("two actual creation ports");
    };
    let coordinate = evaluated.port.clone();
    cfg.evaluated_values
        .source_case_views
        .iter_mut()
        .find(|port| **port == *binding)
        .expect("actual binding association")
        .port = coordinate;
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_same_view_unrelated_contributor_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let source = cfg.evaluated_values.source_backing_producers[0]
        .value
        .clone();
    let observation = cfg
        .evaluated_values
        .source_case_views
        .iter()
        .find(|port| port.case.value == source)
        .expect("actual creation association")
        .clone();
    let other = cfg
        .evaluated_values
        .source_case_views
        .iter()
        .find(|port| port.case.value != source && port.view == observation.view)
        .expect("distinct Read contributor with same view")
        .clone();
    cfg.evaluated_values
        .source_case_views
        .iter_mut()
        .find(|port| **port == observation)
        .expect("creation port")
        .contributor = other.contributor;
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_duplicate_registration_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    assert!(backing_producer::register(&mut cfg).is_err());
}
#[test]
fn backing_source_empty_owned_dependency_is_rejected() {
    let mut cfg = cfg(ScalarTargetLayout::NATIVE64, false);
    let source = cfg.evaluated_values.source_backing_producers[0]
        .value
        .clone();
    let view = cfg
        .evaluated_values
        .source_case_views
        .iter()
        .find(|port| port.case.value == source)
        .expect("allocation source")
        .view
        .clone();
    cfg.evaluated_values.source_views[view.index]
        .dependencies
        .clear();
    assert!(backing_producer::validate(&cfg).is_err());
}
#[test]
fn backing_source_fixed_creation_keeps_unwritten_elements_separate() {
    let cfg = tests::source_cfg(
        "%%start\nu8[]() make = fn { u8[2] bytes; bytes };\n%%end",
        "make",
        ScalarTargetLayout::NATIVE64,
        false,
    );
    backing_producer::validate(&cfg).expect("actual fixed storage creation");
    assert!(cfg
        .evaluated_values
        .source_backing_producers
        .iter()
        .any(|producer| matches!(
            cfg.initial_points[producer.event.point.0].kind,
            CfgPointKind::Backing {
                transfer: RuntimeBackingTransfer::CreateFixed,
                ..
            }
        )));
    for producer in &cfg.evaluated_values.source_backing_producers {
        let value = cfg
            .evaluated_values
            .values
            .iter()
            .find(|value| value.id == producer.value)
            .expect("actual fixed producer");
        assert!(!value.cases.is_empty());
        for case in &value.cases {
            let EvaluatedPayloadFact::RuntimeBacking { backing, elements } = &case.payload else {
                panic!("fixed residence observation");
            };
            assert_eq!(backing.kind, RuntimeBackingKind::FixedValue);
            assert!(
                elements.selections.is_empty(),
                "creation certifies zero initialized element selections"
            );
            assert!(
                case.dependencies.is_empty(),
                "fixed residence supplies zero unique allocation transfer"
            );
        }
    }
    assert!(cfg.evaluated_values.owner_consumers.is_empty());
}
