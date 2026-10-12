use super::*;

const COPY: &str = "%%start\nunit() inspect = fn { u32 value = 1; *u32 root = &value; *u32 copy = root; *copy; *root; };\n%%end";
const CHAIN: &str = "%%start\nunit() inspect = fn { u32 value = 1; *!u32 root = &!value; *!u32 child = &!(*root); *!u32 grandchild = &!(*child); *grandchild; *child; *root; };\n%%end";
const SAVED: &str = "%%start\nu32() inspect = fn { u32 left = 1; u32 right = 2; *!u32 target = &!left; *!u32 replacement = &!right; u32 old = (*(target = replacement) = 9); old + left + *target };\n%%end";

fn graph(cfg: &ScalarReferenceCfg) -> &SourceUseGraph {
    let rows = cfg
        .evaluated_values
        .source_use_graphs
        .iter()
        .filter(|graph| graph.id.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        panic!("one actual owner graph");
    };
    row
}

fn prove(cfg: &ScalarReferenceCfg) {
    let graph = graph(cfg);
    source_use::validate(cfg).expect("total reconstructed source incidence");
    for usage in &graph.uses {
        let rule = &cfg.evaluated_values.source_contributors[usage.contributor.index];
        assert_eq!(rule.id, usage.equation.contributor);
        assert_eq!(rule.input, usage.configuration);
        let node = graph
            .nodes
            .iter()
            .find(|node| node.id == usage.support.node)
            .expect("exact input node");
        assert_eq!(node.id.monitor, usage.equation.input);
        assert_eq!(node.configuration, rule.input);
        assert!(node.supports.contains(&usage.support));
        assert_eq!(
            usage.source_span,
            cfg.initial_points[rule.event.point.0].source_span
        );
        let input = &cfg.evaluated_values.source_configurations[rule.input.index];
        match &usage.input {
            SourceUseInput::Selected(selected) => assert_eq!(
                port_monitor::query(
                    &cfg.evaluated_values,
                    &usage.equation.input,
                    std::slice::from_ref(&selected.coordinate),
                    &usage.source_span
                )
                .expect("one monitor membership"),
                vec![selected.clone()]
            ),
            SourceUseInput::Storage(slot) => assert!(input.slots.contains(slot)),
        }
        if let SourceUseSubject::Grant { class, referent } = &usage.support.subject {
            assert!(input
                .grants
                .iter()
                .any(|grant| grant.class == *class && grant.referent == *referent));
        }
    }
    for row in &graph.reachable {
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.id == row.support.node && node.supports.contains(&row.support)));
        for predecessor in &row.predecessors {
            let edge = &graph.edges[predecessor.edge];
            let next = &graph.reachable[predecessor.reachable];
            assert_eq!(edge.source, row.support);
            assert_eq!(edge.destination, next.support);
            assert_eq!(row.direct_use, next.direct_use);
        }
    }
    for row in &graph.returns {
        assert!(cfg
            .evaluated_values
            .owner_receiving_transitions
            .contains(&row.transition));
        let input_node = graph
            .nodes
            .iter()
            .find(|node| node.id == row.input.node)
            .expect("donor input node");
        let output_node = graph
            .nodes
            .iter()
            .find(|node| node.id == row.output)
            .expect("recipient output node");
        assert_eq!(input_node.configuration, row.transition.input);
        assert_eq!(output_node.configuration, row.transition.output);
        assert_eq!(
            row.input,
            source_use::observe(graph, &row.input.node).expect("actual return reader")
        );
        let draft = cfg
            .evaluated_values
            .pending_owner_consumers
            .iter()
            .find(|draft| {
                draft.candidate.consumer == row.consumer
                    && draft.monitor == row.output.monitor
                    && draft.candidate.contributor == row.output.contributor
            })
            .expect("same qualified return draft");
        assert!(draft
            .requirements
            .contains(&OwnerConsumerRequirement::SelectedRemainingUses(
                row.input.clone()
            )));
        assert!(draft
            .requirements
            .contains(&OwnerConsumerRequirement::RemainingDependencyUses(
                row.transition.input.clone()
            )));
        for dependency in &row.transition.dependencies {
            let original = &cfg.evaluated_values.source_configurations[row.transition.input.index];
            assert!(original
                .owns
                .iter()
                .any(|owns| owns.payload == dependency.source_instance
                    && owns.owner_instance == dependency.owner_instance
                    && owns.state == SourceObligationState::Outstanding));
        }
    }
    for analysis in &cfg.evaluated_values.staged_loan_queries {
        if let StagedLoanQueryResult::Candidates { queries, .. } = &analysis.result {
            for query in queries {
                assert_eq!(query.remaining_uses.node.monitor, query.equation.input);
                assert_eq!(
                    query.remaining_uses,
                    source_use::observe(graph, &query.remaining_uses.node)
                        .expect("actual graph reader")
                );
            }
        }
    }
    assert!(cfg.evaluated_values.owner_consumers.is_empty());
}

fn copy(layout: ScalarTargetLayout, project: bool) {
    let cfg = common_loan::tests::cfg(COPY, layout, project);
    prove(&cfg);
    let graph = graph(&cfg);
    let borrowed = graph
        .uses
        .iter()
        .find(|usage| {
            usage.purpose == SourceUsePurpose::Borrow
                && matches!(usage.support.subject, SourceUseSubject::Payload { .. })
        })
        .expect("declared storage before root grant");
    assert!(matches!(borrowed.input, SourceUseInput::Storage(_)));
    for usage in graph
        .uses
        .iter()
        .filter(|usage| matches!(usage.support.subject, SourceUseSubject::Grant { .. }))
    {
        let SourceUseSubject::Grant { class, referent } = &usage.support.subject else {
            unreachable!()
        };
        let witness = cfg
            .evaluated_values
            .source_grant_creations
            .iter()
            .find(|witness| {
                witness.child == *class && matches!(witness.parent, SourceGrantParent::Root)
            })
            .expect("actual original grant");
        let original = &cfg.evaluated_values.source_configurations[witness.output.index];
        assert!(original
            .grants
            .iter()
            .any(|grant| grant.class == *class && grant.referent == *referent));
    }
    let edge = graph.edges.iter().find(|edge| matches!(&edge.witness, SourceUseEdgeWitness::Transport(transport) if transport.kind == JointValueTransportKind::Copy) && matches!(edge.source.subject, SourceUseSubject::Grant { .. })).expect("exact copied grant");
    assert_eq!(edge.source.subject, edge.destination.subject);
    assert!(graph.reachable.iter().any(|row| row
        .predecessors
        .iter()
        .any(|predecessor| graph.edges[predecessor.edge] == *edge)));
}

fn chain(layout: ScalarTargetLayout, project: bool) {
    let cfg = common_loan::tests::cfg(CHAIN, layout, project);
    prove(&cfg);
    let graph = graph(&cfg);
    let parents = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| matches!(edge.witness, SourceUseEdgeWitness::Parent(_)))
        .collect::<Vec<_>>();
    assert!(parents.len() >= 2);
    for (index, edge) in parents {
        let SourceUseEdgeWitness::Parent(witness) = &edge.witness else {
            unreachable!()
        };
        assert_eq!(edge.source.node.monitor, edge.equation.input);
        assert_eq!(edge.destination.node.monitor, edge.equation.output);
        let SourceGrantParent::Selected {
            class, selection, ..
        } = &witness.parent
        else {
            panic!("saved parent input");
        };
        assert_eq!(edge.source.coordinate, selection.coordinate);
        assert!(
            matches!(&edge.source.subject, SourceUseSubject::Grant { class: actual, .. } if actual == class)
        );
        assert!(
            matches!(&edge.destination.subject, SourceUseSubject::Grant { class, .. } if *class == witness.child)
        );
        assert!(graph.reachable.iter().any(|row| row
            .predecessors
            .iter()
            .any(|predecessor| predecessor.edge == index)));
    }
}

fn saved(layout: ScalarTargetLayout, project: bool) {
    let cfg = common_loan::tests::cfg(SAVED, layout, project);
    prove(&cfg);
    prove_move(&cfg);
    let graph = graph(&cfg);
    let results = graph
        .uses
        .iter()
        .filter(|usage| {
            matches!(
                cfg.initial_points[usage.event.point.0].kind,
                CfgPointKind::AssignmentResult { .. }
            ) && matches!(usage.support.subject, SourceUseSubject::Grant { .. })
        })
        .collect::<Vec<_>>();
    assert!(!results.is_empty());
    for usage in results {
        let slot = &cfg.evaluated_values.source_slots[usage.support.coordinate.index];
        assert!(matches!(slot.role, SourceCoordinateRole::Saved(_)));
        let edge = graph.edges.iter().find(|edge| edge.source == usage.support && matches!(&edge.witness, SourceUseEdgeWitness::Transport(transport) if transport.kind == JointValueTransportKind::Result)).expect("actual saved-to-result conservation");
        assert_eq!(edge.source.subject, edge.destination.subject);
        let input = &cfg.evaluated_values.source_configurations[usage.configuration.index];
        let SourceCoordinateRole::Saved(selection) = &slot.role else {
            unreachable!()
        };
        let destination = cfg
            .evaluated_values
            .destinations
            .iter()
            .find(|destination| destination.id == selection.destination)
            .expect("original frozen destination");
        let EvaluatedDestinationStorage::Direct(place) = &destination.storage else {
            panic!("actual direct target");
        };
        let target = input
            .slots
            .iter()
            .find(|slot| {
                cfg.evaluated_values.source_slots[slot.coordinate.index].role
                    == SourceCoordinateRole::Binding(place.binding.clone())
            })
            .expect("replacement target storage");
        assert_ne!(
            target.view,
            match &usage.input {
                SourceUseInput::Selected(selected) => selected.observation.view.clone(),
                _ => panic!("saved selected input"),
            }
        );
    }
}

#[test]
fn selected_source_use_copy_program_native64() {
    copy(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_source_use_copy_program_wasm32() {
    copy(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_source_use_copy_project_native64() {
    copy(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_source_use_copy_project_wasm32() {
    copy(ScalarTargetLayout::WASM32, true);
}
#[test]
fn selected_source_use_chain_program_native64() {
    chain(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_source_use_chain_program_wasm32() {
    chain(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_source_use_chain_project_native64() {
    chain(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_source_use_chain_project_wasm32() {
    chain(ScalarTargetLayout::WASM32, true);
}
#[test]
fn selected_source_use_saved_program_native64() {
    saved(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn selected_source_use_saved_program_wasm32() {
    saved(ScalarTargetLayout::WASM32, false);
}
#[test]
fn selected_source_use_saved_project_native64() {
    saved(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn selected_source_use_saved_project_wasm32() {
    saved(ScalarTargetLayout::WASM32, true);
}

#[test]
fn selected_source_use_missing_node_rejected() {
    let mut cfg = common_loan::tests::cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_use_graphs
        .iter_mut()
        .find(|row| row.id.owner == cfg.producer_owner)
        .expect("graph");
    row.nodes.pop();
    assert!(source_use::validate(&cfg).is_err());
}

#[test]
fn selected_source_use_crossed_contributor_rejected() {
    let mut cfg = common_loan::tests::cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_use_graphs
        .iter_mut()
        .find(|row| row.id.owner == cfg.producer_owner)
        .expect("graph");
    let other = row
        .nodes
        .iter()
        .find(|node| node.id != row.uses[0].support.node)
        .expect("other source node")
        .id
        .clone();
    row.uses[0].support.node = other;
    assert!(source_use::validate(&cfg).is_err());
}

#[test]
fn selected_source_use_missing_port_rejected() {
    let mut cfg = common_loan::tests::cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_use_graphs
        .iter_mut()
        .find(|row| row.id.owner == cfg.producer_owner)
        .expect("graph");
    row.uses[0].support.coordinate.index = usize::MAX;
    assert!(source_use::validate(&cfg).is_err());
}

#[test]
fn selected_source_use_foreign_node_rejected() {
    let mut cfg = common_loan::tests::cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let row = cfg
        .evaluated_values
        .source_use_graphs
        .iter_mut()
        .find(|row| row.id.owner == cfg.producer_owner)
        .expect("graph");
    row.nodes[0].id.monitor.owner = ProducerCfgOwnerId::Module(SourceIdentity::new(
        "foreign-use".into(),
        "app".into(),
        "src/foreign.w".into(),
        "r1".into(),
    ));
    assert!(source_use::validate(&cfg).is_err());
}

fn prove_move(cfg: &ScalarReferenceCfg) {
    let graph = graph(cfg);
    let witnesses = cfg
        .evaluated_values
        .source_move_witnesses
        .iter()
        .filter(|witness| witness.id.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    assert!(!witnesses.is_empty());
    for witness in witnesses {
        for node in graph.nodes.iter().filter(|node| {
            node.configuration == witness.output && node.id.contributor == witness.contributor
        }) {
            let row = &cfg.evaluated_values.source_configurations[node.configuration.index];
            let storage = selected_slot(
                &cfg.evaluated_values,
                row,
                &SourceCoordinateRole::Binding(witness.place.binding.clone()),
            )
            .expect("actual moved storage slot");
            assert_ne!(storage.availability, ValueAvailability::Available);
            assert!(!node
                .supports
                .iter()
                .any(|support| support.coordinate == storage.coordinate));
            assert!(node
                .supports
                .iter()
                .any(|support| support.coordinate == witness.selection.coordinate
                    && matches!(support.subject, SourceUseSubject::Grant { .. })));
            assert!(graph.coverage.iter().any(|coverage| matches!(&coverage.reason, SourceUseCoverageReason::Unavailable { coordinate, state } if *coordinate == storage.coordinate && *state == storage.availability)));
        }
    }
}

#[test]
fn selected_source_use_unavailable_storage_retains_evaluated_support() {
    let cfg = common_loan::tests::cfg(SAVED, ScalarTargetLayout::NATIVE64, false);
    prove_move(&cfg);
}

#[test]
fn selected_source_use_same_site_subject_identity_stays_qualified() {
    let cfg = common_loan::tests::cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let graph = graph(&cfg);
    let usage = graph
        .uses
        .iter()
        .find(|usage| matches!(usage.support.subject, SourceUseSubject::Grant { .. }))
        .expect("actual grant use");
    let mut crossed = usage.support.clone();
    let SourceUseSubject::Grant {
        referent: SourceInstanceRef::Supported(id),
        ..
    } = &mut crossed.subject
    else {
        panic!("actual supported referent");
    };
    id.index += cfg.evaluated_values.source_instances.len();
    assert_ne!(crossed.subject, usage.support.subject);
    assert!(!graph.reachable.iter().any(|row| row.support == crossed));
    let mut modified = cfg.clone();
    let row = modified
        .evaluated_values
        .source_use_graphs
        .iter_mut()
        .find(|row| row.id.owner == cfg.producer_owner)
        .expect("graph");
    let index = row
        .uses
        .iter()
        .position(|row| row == usage)
        .expect("same source event and class");
    row.uses[index].support = crossed;
    assert!(source_use::validate(&modified).is_err());
}

#[test]
fn selected_source_use_serde_retains_witness_incidence() {
    let cfg = common_loan::tests::cfg(CHAIN, ScalarTargetLayout::NATIVE64, false);
    let original = graph(&cfg);
    let encoded = serde_json::to_vec(original).expect("typed graph serialization");
    let decoded: SourceUseGraph =
        serde_json::from_slice(&encoded).expect("typed graph deserialization");
    assert_eq!(*original, decoded);
}
