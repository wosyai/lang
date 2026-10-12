use super::*;
use crate::parse_source;

const RETURNED_SAVED: &str = "%%start\nu32() inspect = fn { u32 value = 1; u32 replacement = 2; value = replacement };\n%%end";

fn parsed_cfg(package: &str, layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    let source = SourceIdentity::new(
        package.into(),
        "app".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), RETURNED_SAVED.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let items = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        validation
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual module")
            .items
    } else {
        validation.program.items
    };
    items
        .into_iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "inspect" => {
                Some(function.reference_cfg)
            }
            _ => None,
        })
        .expect("actual parsed Assignment owner")
}

fn common(cfg: &ScalarReferenceCfg) -> &CommonLoanAnalysis {
    let rows = cfg
        .evaluated_values
        .common_loans
        .iter()
        .filter(|row| row.input.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        panic!("one actual immutable owner epoch");
    };
    row
}

fn graph(cfg: &ScalarReferenceCfg) -> &SourceUseGraph {
    let rows = cfg
        .evaluated_values
        .source_use_graphs
        .iter()
        .filter(|row| row.id.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        panic!("one actual selected graph");
    };
    row
}

fn collide(cfg: &mut ScalarReferenceCfg, foreign: &ScalarReferenceCfg) {
    assert_ne!(cfg.producer_owner, foreign.producer_owner);
    assert_eq!(cfg.initial_points.len(), foreign.initial_points.len());
    let tables = &cfg.evaluated_values;
    let actual = &foreign.evaluated_values;
    let [local_destination] = tables.destinations.as_slice() else {
        panic!("one original destination");
    };
    let [foreign_destination] = actual.destinations.as_slice() else {
        panic!("one real foreign destination");
    };
    assert_eq!(local_destination.point, foreign_destination.point);
    assert_ne!(local_destination.id, foreign_destination.id);
    assert_eq!(
        foreign_destination.id.operation.producer,
        foreign.producer_owner
    );
    assert_eq!(
        foreign_destination.source_span.source,
        foreign.initial_points[foreign_destination.point.0]
            .source_span
            .source
    );
    let [local_commit] = tables.commits.as_slice() else {
        panic!("one original commit");
    };
    let [foreign_commit] = actual.commits.as_slice() else {
        panic!("one real foreign commit");
    };
    assert_eq!(local_commit.point, foreign_commit.point);
    assert_eq!(
        local_commit.replacement.point,
        foreign_commit.replacement.point
    );
    assert_eq!(foreign_commit.destination, foreign_destination.id);
    assert_eq!(
        foreign_commit.replacement.resolved.producer,
        foreign.producer_owner
    );
    assert!(actual
        .values
        .iter()
        .any(|value| value.id == foreign_commit.replacement));
    let [local_snapshot] = tables.snapshots.as_slice() else {
        panic!("one original snapshot");
    };
    let [foreign_snapshot] = actual.snapshots.as_slice() else {
        panic!("one real foreign snapshot");
    };
    assert_eq!(local_snapshot.point, foreign_snapshot.point);
    assert_eq!(foreign_snapshot.destination, foreign_destination.id);
    let SnapshotState::Saved(foreign_saved) = &foreign_snapshot.state else {
        panic!("actual foreign saved value");
    };
    assert_eq!(foreign_saved.resolved.producer, foreign.producer_owner);
    assert_eq!(foreign_saved.point, foreign_snapshot.point);
    assert!(actual.values.iter().any(|value| value.id == *foreign_saved));
    let [local_result] = tables.results.as_slice() else {
        panic!("one original result");
    };
    let [foreign_result] = actual.results.as_slice() else {
        panic!("one real foreign result");
    };
    assert_eq!(local_result.value.point, foreign_result.value.point);
    assert_eq!(local_result.output, foreign_result.output);
    assert_eq!(foreign_result.snapshot, foreign_snapshot.point);
    assert_eq!(foreign_result.operation, foreign_destination.id.operation);
    assert_eq!(
        foreign_result.value.resolved.producer,
        foreign.producer_owner
    );
    assert!(actual
        .values
        .iter()
        .any(|value| value.id == foreign_result.value));
    cfg.evaluated_values
        .destinations
        .extend(actual.destinations.clone());
    cfg.evaluated_values.commits.extend(actual.commits.clone());
    cfg.evaluated_values
        .snapshots
        .extend(actual.snapshots.clone());
    cfg.evaluated_values.results.extend(actual.results.clone());
    cfg.evaluated_values
        .assignment_plans
        .extend(actual.assignment_plans.clone());
    cfg.evaluated_values
        .assignment_evaluations
        .extend(actual.assignment_evaluations.clone());
    cfg.evaluated_values.values.extend(actual.values.clone());
    cfg.evaluated_values.shapes.extend(actual.shapes.clone());
    cfg.evaluated_values
        .common_loans
        .extend(actual.common_loans.clone());
    cfg.evaluated_values
        .source_use_graphs
        .extend(actual.source_use_graphs.clone());
    cfg.evaluated_values
        .pending_owner_consumers
        .extend(actual.pending_owner_consumers.clone());
    for commit in &actual.commits {
        assert!(cfg
            .evaluated_values
            .destinations
            .iter()
            .any(|row| row.id == commit.destination));
        assert!(cfg
            .evaluated_values
            .values
            .iter()
            .any(|row| row.id == commit.replacement));
        assert!(cfg
            .evaluated_values
            .assignment_plans
            .iter()
            .any(|row| row.operation == commit.destination.operation));
    }
    for result in &actual.results {
        assert!(cfg.evaluated_values.snapshots.iter().any(
            |row| row.point == result.snapshot && row.destination.operation == result.operation
        ));
        assert!(cfg
            .evaluated_values
            .values
            .iter()
            .any(|row| row.id == result.value));
    }
}

fn conserved(layout: ScalarTargetLayout, project: bool) {
    let mut cfg = parsed_cfg("qualified-local", layout, project);
    let foreign = parsed_cfg("qualified-foreign", layout, project);
    let original_common = common(&cfg).clone();
    let original_graph = graph(&cfg).clone();
    let original_drafts = cfg.evaluated_values.pending_owner_consumers.clone();
    collide(&mut cfg, &foreign);
    let original_tables = cfg.evaluated_values.clone();
    let recomputed = extract(
        &cfg.evaluated_values,
        &cfg.producer_owner,
        &cfg.initial_points,
    )
    .expect("qualified immutable extraction with real foreign rows");
    assert_eq!(recomputed, original_common.input);
    common_loan::validate_epoch(&cfg, common(&cfg))
        .expect("original immutable epoch remains authoritative");
    selected_source::source_use::register(&mut cfg).expect("complete source-use registration");
    selected_source::source_use::validate(&cfg).expect("complete source-use validation");
    assert_eq!(*graph(&cfg), original_graph);
    assert_eq!(*common(&cfg), original_common);
    assert_eq!(
        cfg.evaluated_values.common_loans,
        original_tables.common_loans
    );
    assert_eq!(
        cfg.evaluated_values.destinations,
        original_tables.destinations
    );
    assert_eq!(cfg.evaluated_values.commits, original_tables.commits);
    assert_eq!(cfg.evaluated_values.snapshots, original_tables.snapshots);
    assert_eq!(cfg.evaluated_values.results, original_tables.results);
    assert!(foreign
        .evaluated_values
        .source_use_graphs
        .iter()
        .all(|row| cfg.evaluated_values.source_use_graphs.contains(row)));
    assert!(foreign
        .evaluated_values
        .pending_owner_consumers
        .iter()
        .all(|row| cfg.evaluated_values.pending_owner_consumers.contains(row)));
    for original in original_drafts {
        let draft = cfg
            .evaluated_values
            .pending_owner_consumers
            .iter()
            .find(|draft| {
                draft.candidate == original.candidate && draft.monitor == original.monitor
            })
            .expect("same qualified current-owner draft");
        assert_eq!(draft.requirements.len(), original.requirements.len());
        assert!(original
            .requirements
            .iter()
            .all(|requirement| draft.requirements.contains(requirement)));
    }
    let results = original_tables
        .results
        .iter()
        .filter(|row| row.operation.producer == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [result] = results.as_slice() else {
        panic!("actual local result");
    };
    let final_inputs = cfg
        .evaluated_values
        .final_inputs
        .iter()
        .filter(|input| {
            input.consumer.producer == cfg.producer_owner && input.source == result.value
        })
        .collect::<Vec<_>>();
    let [final_input] = final_inputs.as_slice() else {
        panic!("actual returned saved result");
    };
    let returned = recomputed
        .events
        .iter()
        .find(|event| event.event.point == final_input.consumer.point)
        .expect("actual ReturnOutput event");
    let destinations = original_tables
        .destinations
        .iter()
        .filter(|row| row.id.operation.producer == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [destination] = destinations.as_slice() else {
        panic!("actual local saved destination");
    };
    let EvaluatedDestinationStorage::Direct(place) = &destination.storage else {
        panic!("actual declared storage");
    };
    assert!(returned.facts.iter().any(|fact| matches!(&fact.kind, CommonLoanFactKind::Use { target: CommonLoanTarget::Storage { binding, projection }, kind: CommonLoanUseKind::Escape, .. } if *binding == place.binding && projection.is_empty())));
    assert_eq!(
        returned.source_span,
        cfg.initial_points[final_input.consumer.point.0].source_span
    );
}

enum MissingLocal {
    Destination,
    Snapshot,
    Commit,
    Result,
}

fn missing_local(kind: MissingLocal) {
    let mut cfg = parsed_cfg("qualified-local", ScalarTargetLayout::NATIVE64, false);
    let foreign = parsed_cfg("qualified-foreign", ScalarTargetLayout::NATIVE64, false);
    let epoch = common(&cfg).clone();
    let original_graph = graph(&cfg).clone();
    let point = match kind {
        MissingLocal::Destination => cfg.evaluated_values.destinations[0].point,
        MissingLocal::Snapshot => cfg.evaluated_values.snapshots[0].point,
        MissingLocal::Commit => cfg.evaluated_values.commits[0].point,
        MissingLocal::Result => cfg.evaluated_values.results[0].value.point,
    };
    collide(&mut cfg, &foreign);
    match kind {
        MissingLocal::Destination => cfg
            .evaluated_values
            .destinations
            .retain(|row| row.id.operation.producer != cfg.producer_owner),
        MissingLocal::Snapshot => cfg
            .evaluated_values
            .snapshots
            .retain(|row| row.destination.operation.producer != cfg.producer_owner),
        MissingLocal::Commit => cfg
            .evaluated_values
            .commits
            .retain(|row| row.destination.operation.producer != cfg.producer_owner),
        MissingLocal::Result => cfg
            .evaluated_values
            .results
            .retain(|row| row.operation.producer != cfg.producer_owner),
    }
    let extracted = extract(
        &cfg.evaluated_values,
        &cfg.producer_owner,
        &cfg.initial_points,
    );
    match kind {
        MissingLocal::Destination | MissingLocal::Snapshot => {
            let error =
                extracted.expect_err("missing total local destination/returned snapshot join");
            let expected = match kind {
                MissingLocal::Destination => {
                    cfg.evaluated_values
                        .commits
                        .iter()
                        .find(|row| row.destination.operation.producer == cfg.producer_owner)
                        .expect("actual local commit")
                        .point
                }
                MissingLocal::Snapshot => {
                    cfg.evaluated_values
                        .final_inputs
                        .iter()
                        .find(|row| row.consumer.producer == cfg.producer_owner)
                        .expect("actual local return")
                        .consumer
                        .point
                }
                _ => unreachable!(),
            };
            assert!(
                matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == cfg.initial_points[expected.0].source_span)
            );
        }
        MissingLocal::Commit | MissingLocal::Result => {
            let changed = extracted.expect("actual missing-local typed coverage");
            assert_ne!(changed, epoch.input);
            assert!(changed.pending.iter().any(|pending| pending.event.owner
                == cfg.producer_owner
                && pending.event.point == point
                && pending.source_span == cfg.initial_points[point.0].source_span
                && pending.reason == CommonLoanPendingReason::MissingAssignmentInput));
            assert!(changed
                .events
                .iter()
                .find(|event| event.event.point == point)
                .expect("actual missing event")
                .facts
                .is_empty());
        }
    }
    assert!(common_loan::validate_epoch(&cfg, common(&cfg)).is_err());
    assert!(selected_source::source_use::register(&mut cfg).is_err());
    assert!(selected_source::source_use::validate(&cfg).is_err());
    assert_eq!(*common(&cfg), epoch);
    assert_eq!(*graph(&cfg), original_graph);
    assert!(foreign
        .evaluated_values
        .destinations
        .iter()
        .all(|row| cfg.evaluated_values.destinations.contains(row)));
    assert!(foreign
        .evaluated_values
        .snapshots
        .iter()
        .all(|row| cfg.evaluated_values.snapshots.contains(row)));
    assert!(foreign
        .evaluated_values
        .commits
        .iter()
        .all(|row| cfg.evaluated_values.commits.contains(row)));
    assert!(foreign
        .evaluated_values
        .results
        .iter()
        .all(|row| cfg.evaluated_values.results.contains(row)));
}

#[test]
fn qualified_assignment_collision_program_native64() {
    conserved(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn qualified_assignment_collision_program_wasm32() {
    conserved(ScalarTargetLayout::WASM32, false);
}
#[test]
fn qualified_assignment_collision_project_native64() {
    conserved(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn qualified_assignment_collision_project_wasm32() {
    conserved(ScalarTargetLayout::WASM32, true);
}
#[test]
fn qualified_assignment_foreign_only_destination_rejects_original_epoch() {
    missing_local(MissingLocal::Destination);
}
#[test]
fn qualified_assignment_foreign_only_snapshot_rejects_original_epoch() {
    missing_local(MissingLocal::Snapshot);
}
#[test]
fn qualified_assignment_foreign_only_commit_rejects_original_epoch() {
    missing_local(MissingLocal::Commit);
}
#[test]
fn qualified_assignment_foreign_only_result_rejects_original_epoch() {
    missing_local(MissingLocal::Result);
}

enum CrossedIncidence {
    CommitReplacement,
    ResultOperation,
    ResultValue,
    SnapshotSaved,
}

fn crossed_incidence(kind: CrossedIncidence) {
    let mut cfg = parsed_cfg("qualified-local", ScalarTargetLayout::NATIVE64, false);
    let foreign = parsed_cfg("qualified-foreign", ScalarTargetLayout::NATIVE64, false);
    let epoch = common(&cfg).clone();
    let original_graph = graph(&cfg).clone();
    collide(&mut cfg, &foreign);
    let point = match kind {
        CrossedIncidence::CommitReplacement => {
            let row = cfg
                .evaluated_values
                .commits
                .iter_mut()
                .find(|row| row.destination.operation.producer == cfg.producer_owner)
                .expect("actual local commit");
            row.replacement = foreign.evaluated_values.commits[0].replacement.clone();
            row.point
        }
        CrossedIncidence::ResultOperation => {
            let row = cfg
                .evaluated_values
                .results
                .iter_mut()
                .find(|row| row.value.resolved.producer == cfg.producer_owner)
                .expect("actual local result");
            row.operation = foreign.evaluated_values.results[0].operation.clone();
            row.value.point
        }
        CrossedIncidence::ResultValue => {
            let row = cfg
                .evaluated_values
                .results
                .iter_mut()
                .find(|row| row.operation.producer == cfg.producer_owner)
                .expect("actual local result operation");
            row.value = foreign.evaluated_values.results[0].value.clone();
            row.value.point
        }
        CrossedIncidence::SnapshotSaved => {
            let row = cfg
                .evaluated_values
                .snapshots
                .iter_mut()
                .find(|row| row.destination.operation.producer == cfg.producer_owner)
                .expect("actual local snapshot");
            row.state = foreign.evaluated_values.snapshots[0].state.clone();
            row.point
        }
    };
    let extracted = extract(
        &cfg.evaluated_values,
        &cfg.producer_owner,
        &cfg.initial_points,
    );
    match kind {
        CrossedIncidence::SnapshotSaved => {
            let error =
                extracted.expect_err("foreign saved value supplies zero local snapshot incidence");
            assert!(
                matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == cfg.initial_points[point.0].source_span)
            );
        }
        _ => {
            let changed = extracted.expect("crossed operation/value remains typed missing input");
            assert_ne!(changed, epoch.input);
            assert!(changed
                .pending
                .iter()
                .any(|pending| pending.event.owner == cfg.producer_owner
                    && pending.event.point == point
                    && pending.reason == CommonLoanPendingReason::MissingAssignmentInput));
        }
    }
    assert!(common_loan::validate_epoch(&cfg, common(&cfg)).is_err());
    assert!(selected_source::source_use::register(&mut cfg).is_err());
    assert!(selected_source::source_use::validate(&cfg).is_err());
    assert_eq!(*common(&cfg), epoch);
    assert_eq!(*graph(&cfg), original_graph);
    assert!(foreign
        .evaluated_values
        .destinations
        .iter()
        .all(|row| cfg.evaluated_values.destinations.contains(row)));
    assert!(foreign
        .evaluated_values
        .snapshots
        .iter()
        .all(|row| cfg.evaluated_values.snapshots.contains(row)));
    assert!(foreign
        .evaluated_values
        .commits
        .iter()
        .all(|row| cfg.evaluated_values.commits.contains(row)));
    assert!(foreign
        .evaluated_values
        .results
        .iter()
        .all(|row| cfg.evaluated_values.results.contains(row)));
}

#[test]
fn qualified_assignment_crossed_commit_replacement_rejects_original_epoch() {
    crossed_incidence(CrossedIncidence::CommitReplacement);
}
#[test]
fn qualified_assignment_crossed_result_operation_rejects_original_epoch() {
    crossed_incidence(CrossedIncidence::ResultOperation);
}
#[test]
fn qualified_assignment_crossed_result_value_rejects_original_epoch() {
    crossed_incidence(CrossedIncidence::ResultValue);
}
#[test]
fn qualified_assignment_crossed_snapshot_saved_value_rejects_original_epoch() {
    crossed_incidence(CrossedIncidence::SnapshotSaved);
}
