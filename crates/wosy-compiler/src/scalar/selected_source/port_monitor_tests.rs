use super::*;

const SAVED: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() saved_packet = fn { u64 length = 1; u8[length] first; u8[length] second; *?u8 current = null; *?u8 replacement = null; unsafe { current = &?first[0]; replacement = &?second[0]; }; *?u8 old = (current = replacement); Packet result = { .data = old; .length = length; }; &result };\n%%end";
const SAME_VIEW: &str = "%%start\nu64(bool) choose = fn(flag) { u64 current = 1; if (flag) { current = 2; } else { current = 3; }; current };\n%%end";

fn prove_saved(layout: ScalarTargetLayout, project: bool) {
    let cfg = tests::source_cfg(SAVED, "saved_packet", layout, project);
    let tables = &cfg.evaluated_values;
    assert!(!tables.source_saved_ports.is_empty());
    for saved in &tables.source_saved_ports {
        let selections = port_monitor::query(
            tables,
            &saved.monitor,
            std::slice::from_ref(&saved.selected.coordinate),
            &cfg.initial_points[saved.snapshot.point.0].source_span,
        )
        .expect("exact Snapshot source membership");
        assert_eq!(selections, vec![saved.selected.clone()]);
        assert!(saved
            .duties
            .iter()
            .any(|owns| owns.kind == RuntimeBackingKind::OwnedAllocation));
    }
    let mut distinct = false;
    for monitor in &tables.source_port_monitors {
        let rule = &tables.source_contributors[monitor.contributor.index];
        let row = &tables.source_configurations[rule.output.index];
        for saved in &row.saved {
            let Some(previous) = monitor
                .ports
                .iter()
                .find(|port| port.coordinate == saved.coordinate)
            else {
                continue;
            };
            let destination = tables
                .destinations
                .iter()
                .find(|destination| destination.id == saved.selection.destination)
                .expect("frozen source destination");
            let EvaluatedDestinationStorage::Direct(place) = &destination.storage else {
                continue;
            };
            let Ok(current) = selected_slot(
                tables,
                row,
                &SourceCoordinateRole::Binding(place.binding.clone()),
            ) else {
                continue;
            };
            let Some(replacement) = monitor
                .ports
                .iter()
                .find(|port| port.coordinate == current.coordinate)
            else {
                continue;
            };
            if previous.observation.view == replacement.observation.view {
                continue;
            }
            let tuple = port_monitor::query(
                tables,
                &monitor.id,
                &[saved.coordinate.clone(), current.coordinate.clone()],
                &cfg.initial_points[rule.event.point.0].source_span,
            )
            .expect("one joint saved/current valuation");
            assert_ne!(tuple[0].observation.case, tuple[1].observation.case);
            assert_ne!(tuple[0].sources, tuple[1].sources);
            distinct = true;
        }
    }
    assert!(
        distinct,
        "replacement leaves exact previous source membership intact"
    );
    for pending in &tables.pending_owner_consumers {
        let rule = &tables.source_contributors[pending.candidate.contributor.index];
        owner_consumer::validate_correspondence(
            tables,
            &cfg.initial_points,
            &tables.source_configurations[rule.output.index],
            &pending.candidate,
        )
        .expect("monitored ReturnOutput draft");
    }
    assert!(tables.owner_consumers.is_empty());
}

#[test]
fn selected_port_saved_source_program_native64() {
    prove_saved(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_port_saved_source_program_wasm32() {
    prove_saved(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_port_saved_source_project_native64() {
    prove_saved(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_port_saved_source_project_wasm32() {
    prove_saved(ScalarTargetLayout::WASM32, true);
}

#[test]
fn selected_port_same_view_crossed_case_is_rejected() {
    let mut cfg = tests::source_cfg(SAME_VIEW, "choose", ScalarTargetLayout::NATIVE64, false);
    let tables = &mut cfg.evaluated_values;
    let candidates = tables.pending_owner_consumers.clone();
    let pair = candidates
        .iter()
        .find_map(|first| {
            candidates
                .iter()
                .find(|second| {
                    first.candidate.input.value == second.candidate.input.value
                        && first.candidate.input != second.candidate.input
                })
                .map(|second| (first.clone(), second.clone()))
        })
        .expect("actual same-view branch alternatives");
    let first = tables.source_port_monitors[pair.0.monitor.index].clone();
    let second = tables.source_port_monitors[pair.1.monitor.index].clone();
    let original = first
        .ports
        .iter()
        .find(|port| port.observation.case == pair.0.candidate.input)
        .expect("actual first selected source")
        .clone();
    let crossed = second
        .ports
        .iter()
        .find(|port| port.observation.case == pair.1.candidate.input)
        .expect("actual second selected source")
        .clone();
    assert_eq!(original.observation.view, crossed.observation.view);
    assert_ne!(original.observation.case, crossed.observation.case);
    tables.source_port_monitors[first.id.index]
        .ports
        .iter_mut()
        .find(|port| port.coordinate == original.coordinate)
        .expect("original source port")
        .observation = crossed.observation;
    assert!(port_monitor::query(
        tables,
        &first.id,
        std::slice::from_ref(&original.coordinate),
        &cfg.initial_points[pair.0.candidate.consumer.point.0].source_span
    )
    .is_err());
}

#[test]
fn selected_port_production_input_hole_is_rejected() {
    let mut cfg = tests::source_cfg(SAVED, "saved_packet", ScalarTargetLayout::NATIVE64, false);
    let tables = &mut cfg.evaluated_values;
    let equation = tables
        .source_port_equations
        .iter_mut()
        .find(|equation| {
            equation
                .productions
                .iter()
                .any(|production| !production.inputs.is_empty())
        })
        .expect("actual dependent source production");
    equation
        .productions
        .iter_mut()
        .find(|production| !production.inputs.is_empty())
        .expect("actual source input")
        .inputs
        .clear();
    let output = equation.output.clone();
    let port = tables.source_port_monitors[output.index].ports[0]
        .coordinate
        .clone();
    assert!(port_monitor::query(
        tables,
        &output,
        std::slice::from_ref(&port),
        &cfg.initial_points[0].source_span
    )
    .is_err());
}

#[test]
fn selected_port_pending_formal_input_has_no_case_certificate() {
    let cfg = tests::source_cfg(SAME_VIEW, "choose", ScalarTargetLayout::NATIVE64, false);
    let root = cfg
        .evaluated_values
        .source_port_monitors
        .iter()
        .find(|monitor| {
            !monitor.pending.is_empty()
                && !cfg
                    .evaluated_values
                    .source_port_equations
                    .iter()
                    .any(|equation| equation.output == monitor.id)
        })
        .expect("actual quantified input Root");
    assert!(port_monitor::selection(
        &cfg.evaluated_values,
        &root.id,
        &root.pending[0],
        &cfg.initial_points[0].source_span
    )
    .expect("explicit pending input equation")
    .is_none());
}

#[test]
fn selected_port_two_outputs_require_one_child_valuation() {
    let mut cfg = tests::source_cfg(SAME_VIEW, "choose", ScalarTargetLayout::NATIVE64, false);
    let tables = &mut cfg.evaluated_values;
    let child = tables
        .source_port_monitors
        .iter()
        .find(|monitor| {
            monitor.ports.len() >= 2
                && matches!(
                    tables.source_contributors[monitor.contributor.index].kind,
                    JointProducerRuleKind::Return
                )
        })
        .expect("actual selected return tuple")
        .clone();
    let child_rule = tables.source_contributors[child.contributor.index].clone();
    let ports = child
        .ports
        .iter()
        .take(2)
        .map(|port| port.coordinate.clone())
        .collect::<Vec<_>>();
    let mut parent = child_rule.clone();
    parent.id.index = tables.source_contributors.len();
    parent.input = child_rule.output.clone();
    parent.preserved = tables.source_configurations[child_rule.output.index]
        .instances
        .clone();
    parent.fresh.clear();
    parent.children = vec![JointProducerInputPort {
        event: child_rule.event.clone(),
        configuration: child_rule.output.clone(),
        ports: ports.clone(),
    }];
    parent.productions.clear();
    parent.transports = ports
        .iter()
        .enumerate()
        .map(|(port, source)| JointValueTransportFact {
            source: source.clone(),
            destination: source.clone(),
            kind: JointValueTransportKind::Copy,
            child: JointValueChildPort::SelectedChild {
                event: child_rule.event.clone(),
                port,
            },
        })
        .collect();
    let id = parent.id.clone();
    tables.source_contributors.push(parent);
    let selected_child = JointSelectedChildFact {
        event: child_rule.event.clone(),
        monitor: child.id.clone(),
        ports: ports.clone(),
    };
    let source = &cfg.initial_points[child_rule.event.point.0].source_span;
    let output = port_monitor::transition(
        tables,
        &id,
        &child.id,
        vec![selected_child.clone()],
        Vec::new(),
        Vec::new(),
        source,
    )
    .expect("one child transports the complete two-port tuple");
    assert_eq!(
        port_monitor::query(tables, &output, &ports, source)
            .expect("joint child output query")
            .len(),
        2
    );
    let crossed = tables
        .source_port_monitors
        .iter()
        .find(|alternative| {
            alternative.id != child.id
                && alternative.contributor.index < id.index
                && tables.source_contributors[alternative.contributor.index].event
                    == child_rule.event
        })
        .expect("actual alternate child valuation")
        .id
        .clone();
    let equation = tables
        .source_port_equations
        .iter_mut()
        .find(|equation| equation.output == output)
        .expect("actual parent equation");
    equation.children.push(JointSelectedChildFact {
        monitor: crossed,
        ..selected_child
    });
    assert!(port_monitor::query(tables, &output, &ports, source).is_err());
}

#[test]
fn selected_port_unchanged_transition_adds_zero_monitor_or_equation_rows() {
    let mut cfg = tests::source_cfg(SAVED, "saved_packet", ScalarTargetLayout::NATIVE64, false);
    let tables = &mut cfg.evaluated_values;
    let equation = tables
        .source_port_equations
        .last()
        .expect("actual productive transition")
        .clone();
    let counts = (
        tables.source_port_monitors.len(),
        tables.source_port_equations.len(),
    );
    let output = port_monitor::transition(
        tables,
        &equation.contributor,
        &equation.input,
        equation.children,
        equation.productions,
        equation.moves,
        &cfg.initial_points[0].source_span,
    )
    .expect("same finite equation saturation");
    assert_eq!(output, equation.output);
    assert_eq!(
        (
            tables.source_port_monitors.len(),
            tables.source_port_equations.len()
        ),
        counts
    );
}

#[test]
fn selected_port_preserving_cycle_saturates_without_predecessor_history() {
    let mut cfg = tests::source_cfg(SAVED, "saved_packet", ScalarTargetLayout::NATIVE64, false);
    let tables = &mut cfg.evaluated_values;
    let equation = tables
        .source_port_equations
        .iter()
        .rfind(|equation| {
            let rule = &tables.source_contributors[equation.contributor.index];
            rule.input == rule.output
                && equation.productions.is_empty()
                && !tables.source_port_monitors[equation.output.index]
                    .ports
                    .is_empty()
        })
        .expect("actual unchanged port-preserving rule")
        .clone();
    let before = tables.source_port_monitors.len();
    let output = port_monitor::transition(
        tables,
        &equation.contributor,
        &equation.output,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        &cfg.initial_points[0].source_span,
    )
    .expect("finite self equation delta");
    assert_eq!(output, equation.output);
    assert_eq!(tables.source_port_monitors.len(), before);
    let equations = tables.source_port_equations.len();
    let repeated = port_monitor::transition(
        tables,
        &equation.contributor,
        &output,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        &cfg.initial_points[0].source_span,
    )
    .expect("unchanged self equation saturation");
    assert_eq!(repeated, output);
    assert_eq!(tables.source_port_equations.len(), equations);
    let requested = tables.source_port_monitors[output.index]
        .ports
        .iter()
        .map(|port| port.coordinate.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        port_monitor::query(
            tables,
            &output,
            &requested,
            &cfg.initial_points[0].source_span
        )
        .expect("finite cyclic monitor query")
        .len(),
        requested.len()
    );
}
