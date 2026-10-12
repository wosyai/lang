use super::*;

const RUNTIME: &str = "%%start\nu8[](u64) make = fn(count) { u8[count] bytes; bytes };\n%%end";

fn missing_producer(layout: ScalarTargetLayout, project: bool) {
    let mut cfg = tests::source_cfg(RUNTIME, "make", layout, project);
    let [producer] = cfg.evaluated_values.source_backing_producers.as_slice() else {
        panic!("one actual producer");
    };
    let producer = producer.clone();
    let point = &cfg.initial_points[producer.event.point.0];
    cfg.evaluated_values.source_backing_producers.clear();
    let error = backing_producer::resolve(&cfg.evaluated_values, &cfg.producer_owner, point)
        .expect_err("missing canonical producer is qualified construction failure");
    let ReferenceAnalysisError::MissingContext {
        source_span,
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    } = error
    else {
        panic!("original qualified construction diagnostic");
    };
    assert_eq!(source_span, point.source_span);
    assert!(cfg
        .evaluated_values
        .values
        .iter()
        .any(|value| value.id == producer.value));
    let error = backing_producer::validate(&cfg)
        .expect_err("whole producer validation retains missing registration error");
    let ReferenceAnalysisError::MissingContext {
        source_span,
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    } = error
    else {
        panic!("same qualified diagnostic");
    };
    assert_eq!(source_span, point.source_span);
}

fn rebuild(
    tables: &mut EvaluatedValueTables,
    original: &JointPortMonitorRef,
    creation: &JointPortMonitorRef,
    replacement: &JointPortMonitorRef,
    points: &[ReferenceFlowPoint],
) -> JointPortMonitorRef {
    if original == creation {
        return replacement.clone();
    }
    let equations = tables
        .source_port_equations
        .iter()
        .filter(|equation| equation.output == *original)
        .cloned()
        .collect::<Vec<_>>();
    if equations.is_empty() {
        let valuation = &tables.source_port_monitors[original.index];
        assert_eq!(valuation.id, *original);
        assert!(matches!(
            tables.source_contributors[valuation.contributor.index].kind,
            JointProducerRuleKind::Root
        ));
        return original.clone();
    }
    let [equation] = equations.as_slice() else {
        panic!("one exact original equation");
    };
    assert!(
        equation.moves.is_empty(),
        "the original pre-Move derivation carries zero prior Move witness attachments"
    );
    let input = rebuild(tables, &equation.input, creation, replacement, points);
    let mut children = equation.children.clone();
    for child in &mut children {
        child.monitor = rebuild(tables, &child.monitor, creation, replacement, points);
    }
    let rule = tables.source_contributors[equation.contributor.index].clone();
    let monitor = port_monitor::transition(
        tables,
        &equation.contributor,
        &input,
        children,
        equation.productions.clone(),
        equation.moves.clone(),
        &points[rule.event.point.0].source_span,
    )
    .expect("real transition preserves original rule input/output correspondence");
    assert_eq!(
        tables.source_port_monitors[monitor.index].contributor,
        rule.id
    );
    monitor
}

fn missing_binding_port(layout: ScalarTargetLayout, project: bool) {
    let cfg = tests::source_cfg(RUNTIME, "make", layout, project);
    let mut tables = cfg.evaluated_values.clone();
    let [producer] = tables.source_backing_producers.as_slice() else {
        panic!("one actual producer");
    };
    let producer = producer.clone();
    let creations = tables
        .source_contributors
        .iter()
        .filter(|rule| rule.event == producer.event)
        .cloned()
        .collect::<Vec<_>>();
    let [creation] = creations.as_slice() else {
        panic!("one actual creation tuple");
    };
    let row = tables.source_configurations[creation.output.index].clone();
    let binding = selected_slot(
        &tables,
        &row,
        &SourceCoordinateRole::Binding(producer.target.clone()),
    )
    .expect("actual binding coordinate")
    .coordinate
    .clone();
    let evaluated = selected_slot(
        &tables,
        &row,
        &SourceCoordinateRole::Evaluated(producer.value.clone()),
    )
    .expect("actual evaluated creation coordinate")
    .coordinate
    .clone();
    let equations = tables
        .source_port_equations
        .iter()
        .filter(|equation| equation.contributor == creation.id)
        .cloned()
        .collect::<Vec<_>>();
    let [equation] = equations.as_slice() else {
        panic!("one actual creation equation");
    };
    let original_ports = port_monitor::query(
        &tables,
        &equation.output,
        &[evaluated.clone(), binding.clone()],
        &cfg.initial_points[producer.event.point.0].source_span,
    )
    .expect("independent productive original valuation");
    assert_eq!(original_ports.len(), 2);
    tables.source_contributors[creation.id.index]
        .productions
        .retain(|production| production.destination != binding);
    let productions = equation
        .productions
        .iter()
        .filter(|production| production.observation.port != binding)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(productions.len(), 1);
    assert_eq!(productions[0].observation.port, evaluated);
    let pending_creation = port_monitor::transition(
        &mut tables,
        &creation.id,
        &equation.input,
        equation.children.clone(),
        productions,
        equation.moves.clone(),
        &cfg.initial_points[producer.event.point.0].source_span,
    )
    .expect("deliberately incomplete creation equation through actual transition");
    assert_eq!(tables.source_configurations[creation.output.index], row);
    assert!(tables.source_port_monitors[pending_creation.index]
        .pending
        .contains(&binding));
    assert!(port_monitor::query(
        &tables,
        &pending_creation,
        &[binding.clone()],
        &cfg.initial_points[producer.event.point.0].source_span
    )
    .expect("explicit pending binding query")
    .is_empty());
    let retained = port_monitor::query(
        &tables,
        &pending_creation,
        &[evaluated],
        &cfg.initial_points[producer.event.point.0].source_span,
    )
    .expect("retained evaluated leaf");
    assert_eq!(retained, vec![original_ports[0].clone()]);
    let error = backing_producer::validate(&ScalarReferenceCfg {
        evaluated_values: tables.clone(),
        ..cfg.clone()
    })
    .expect_err("generic producer validation keeps missing source-port error");
    let ReferenceAnalysisError::MissingContext {
        source_span,
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    } = error
    else {
        panic!("qualified creation error");
    };
    assert_eq!(
        source_span,
        cfg.initial_points[producer.event.point.0].source_span
    );
    let witnesses = cfg
        .evaluated_values
        .source_move_witnesses
        .iter()
        .filter(|witness| {
            witness
                .selection
                .sources
                .contains(&original_ports[1].observation)
        })
        .cloned()
        .collect::<Vec<_>>();
    let [witness] = witnesses.as_slice() else {
        panic!("one exact original Move consuming this binding leaf");
    };
    let move_equations = cfg
        .evaluated_values
        .source_port_equations
        .iter()
        .filter(|equation| equation.contributor == witness.contributor)
        .cloned()
        .collect::<Vec<_>>();
    let [move_equation] = move_equations.as_slice() else {
        panic!("one actual Move equation");
    };
    let read_monitors = cfg
        .evaluated_values
        .source_port_monitors
        .iter()
        .filter(|monitor| {
            let rule = &cfg.evaluated_values.source_contributors[monitor.contributor.index];
            rule.event.point == witness.source.point && rule.outputs.contains(&witness.source)
        })
        .cloned()
        .collect::<Vec<_>>();
    let [read] = read_monitors.as_slice() else {
        panic!("one original subsequent Read valuation");
    };
    let pending_read = rebuild(
        &mut tables,
        &read.id,
        &equation.output,
        &pending_creation,
        &cfg.initial_points,
    );
    let read_rule = &tables.source_contributors[read.contributor.index];
    let read_row = &tables.source_configurations[read_rule.output.index];
    let read_coordinate = selected_slot(
        &tables,
        read_row,
        &SourceCoordinateRole::Evaluated(witness.source.clone()),
    )
    .expect("original Read output")
    .coordinate
    .clone();
    assert!(tables.source_port_monitors[pending_read.index]
        .pending
        .contains(&read_coordinate));
    assert!(port_monitor::query(
        &tables,
        &pending_read,
        &[read_coordinate.clone()],
        &cfg.initial_points[witness.source.point.0].source_span
    )
    .expect("pending Read query")
    .is_empty());
    let pending_input = rebuild(
        &mut tables,
        &move_equation.input,
        &equation.output,
        &pending_creation,
        &cfg.initial_points,
    );
    assert!(tables.source_port_monitors[pending_input.index]
        .pending
        .contains(&read_coordinate));
    let input = tables.source_configurations[witness.input.index].clone();
    let mut output = input.clone();
    output.id = witness.output.clone();
    let initial_output = output.clone();
    let witness_count = tables.source_move_witnesses.len();
    let result = selected_move::prepare(
        &mut tables,
        &cfg.initial_points,
        &input,
        &mut output,
        &cfg.initial_points[witness.event.point.0],
        &witness.event,
        &pending_input,
    )
    .expect("actual pending Move preparation");
    assert!(result.is_none());
    assert_eq!(
        tables.source_move_witnesses.len(),
        witness_count,
        "zero additional positive witness from this construction"
    );
    assert_eq!(
        output, initial_output,
        "pending capture mutates zero storage"
    );
    assert!(tables
        .pending_source_moves
        .iter()
        .any(|pending| pending.event == witness.event
            && pending.reason
                == SourceMovePendingReason::SelectedSource(SourceMoveInput {
                    event: witness.event.clone(),
                    source: witness.source.clone(),
                    place: witness.place.clone()
                })));
    assert!(tables.owner_consumers.is_empty());
}

#[test]
fn runtime_structural_missing_producer_program_native64() {
    missing_producer(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn runtime_structural_missing_producer_program_wasm32() {
    missing_producer(ScalarTargetLayout::WASM32, false);
}
#[test]
fn runtime_structural_missing_producer_project_native64() {
    missing_producer(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn runtime_structural_missing_producer_project_wasm32() {
    missing_producer(ScalarTargetLayout::WASM32, true);
}
#[test]
fn runtime_structural_missing_binding_port_program_native64() {
    missing_binding_port(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn runtime_structural_missing_binding_port_program_wasm32() {
    missing_binding_port(ScalarTargetLayout::WASM32, false);
}
#[test]
fn runtime_structural_missing_binding_port_project_native64() {
    missing_binding_port(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn runtime_structural_missing_binding_port_project_wasm32() {
    missing_binding_port(ScalarTargetLayout::WASM32, true);
}
