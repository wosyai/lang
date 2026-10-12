use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct copy Reader { u8() read; }\nstruct copy References { *i32 first; *i32 second; }\nstruct Storage { u8[] bytes; }\n(u8())() callable = fn { u8 a = 1; u8 b = 2; u8() current = fn { a }; u8() next = fn { b }; current = next };\n(u8())() projection = fn { u8 a = 1; u8 b = 2; Reader current = { .read = fn { a }; }; u8() next = fn { b }; current.read = next };\nu8[](u64, u64) backing = fn(first, second) { u8[first] current; u8[second] next; current = next };\nReferences(*i32, *i32) aggregate_checked = fn(left, right) { References current = { .first = left; .second = right; }; References next = { .first = right; .second = left; }; current = next };\nStorage(u64, u64) aggregate_backing = fn(first, second) { u8[first] before; Storage current = { .bytes = before; }; u8[second] after; Storage next = { .bytes = after; }; current = next };\n(i32, i32)() ordered = fn { i32 left = 1; i32 right = 2; left, right = 3, 4 };\n(u8())() nested = fn { u8 a = 1; u8 b = 2; u8() current = fn { a }; u8() next = fn { b }; current = (current = next) };\nReader(u64) shared = fn(count) { u8[count] slots; slots[0] = 6; Reader current = { .read = fn { slots[0] }; }; Reader next = { .read = fn { 2 }; }; Reader old = (current = next); u8[count] replacement; replacement[0] = 9; slots = replacement; old };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "assignment-saved-flow-proof".into(),
        "app".into(),
        "src/saved.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let (items, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        (
            validation
                .project
                .modules
                .into_iter()
                .next()
                .expect("actual source module")
                .items,
            trace,
        )
    } else {
        (validation.program.items, trace)
    };
    let mut functions = 0;
    let mut saved = 0;
    for item in &items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        functions += 1;
        let cfg = &function.reference_cfg;
        let pass = trace
            .passes
            .iter()
            .rfind(|pass| {
                pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
            })
            .expect("actual function transfer");
        assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
        for fact in &cfg.evaluated_values.values {
            let construction = trace.construction_fact(cfg, fact).expect("actual initial source fact");
            assert!(construction.cases.is_empty());
        }
        for point in &cfg.points {
            let CfgPointKind::AssignmentSnapshot {
                destination_point,
                holder,
                observed: true,
            } = &point.kind
            else {
                continue;
            };
            saved += 1;
            let before = pass
                .observations
                .points
                .iter()
                .find(|row| {
                    row.point == point.id && row.phase == ReferenceTransferPhase::BeforeEvaluation
                })
                .expect("actual pre-snapshot state");
            let after = pass
                .observations
                .points
                .iter()
                .find(|row| {
                    row.point == point.id && row.phase == ReferenceTransferPhase::BeforeInputCleanup
                })
                .expect("actual saved state");
            let rows = &after.state.assignment_saved[&point.id];
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(
                row.source,
                before.state.assignment_destinations[destination_point][0].place
            );
            let frozen = &before.state.assignment_destinations[destination_point][0];
            let destination = cfg
                .evaluated_values
                .destinations
                .iter()
                .find(|destination| destination.point == *destination_point)
                .expect("actual snapshot destination");
            assert_eq!(frozen.destination, destination.id);
            assert_eq!(frozen.point, destination.point);
            if let FrozenAssignmentAccess::Indexed { holder, .. }
            | FrozenAssignmentAccess::ThroughIndexed { holder, .. } = &frozen.access
            {
                assert!(
                    destination
                        .selector_inputs
                        .iter()
                        .any(|input| input.point == holder.index_point)
                        || matches!(
                            &frozen.access,
                            FrozenAssignmentAccess::ThroughIndexed { .. }
                        )
                );
            }
            assert_eq!(&row.holder, holder);
            assert_eq!(row.released, before.state.released);
            assert_eq!(row.invalidated, before.state.invalidated);
            assert_eq!(point.possible_origins.as_ref(), Some(&row.checked));
            assert_eq!(point.possible_backings, row.backings);
            assert_eq!(point.possible_projected_backings, row.projected_backings);
            assert_eq!(point.possible_environments, row.environments);
            for producer in &row.producers {
                assert_eq!(
                    after
                        .state
                        .holder_provenance
                        .lookup(&producer.environment, &producer.holder),
                    Some(&producer.producer)
                );
                assert!(before
                    .state
                    .holder_provenance
                    .entries
                    .iter()
                    .any(|entry| entry.environment == producer.environment
                        && entry.producer == producer.producer));
            }
            let edges = cfg
                .holder_edges
                .iter()
                .filter(|edge| edge.from == point.id)
                .collect::<Vec<_>>();
            assert!(
                edges.iter().all(|edge| edge.actions.is_empty()),
                "snapshot adds zero counted actions"
            );
            let results = cfg.points.iter().filter(|result| matches!(result.kind, CfgPointKind::AssignmentResult { snapshot_point, .. } if snapshot_point == point.id)).collect::<Vec<_>>();
            assert_eq!(results.len(), 1);
            let result = results[0];
            assert_eq!(result.possible_origins, point.possible_origins);
            assert_eq!(result.possible_environments, point.possible_environments);
            assert_eq!(result.possible_backings, point.possible_backings);
            assert_eq!(
                result.possible_projected_backings,
                point.possible_projected_backings
            );
            for returned in cfg.points.iter().filter(|returned| matches!(returned.kind, CfgPointKind::ReturnOutput { value_point, .. } if value_point == result.id)) {
                assert_eq!(returned.possible_projected_backings, row.projected_backings);
                for backing in &row.backings { assert!(returned.possible_backings.contains(backing)); }
                for environment in &row.environments { assert!(returned.possible_environments.contains(environment)); }
                assert!(cfg.evaluated_values.final_inputs.iter().any(|input| input.source.point == result.id && input.consumer.point == returned.id));
            }
            match function.name.as_str() {
                "callable" | "projection" => {
                    assert_eq!(row.environments.len(), 1);
                    let evaluation = cfg
                        .evaluated_values
                        .assignment_evaluations
                        .iter()
                        .find(|evaluation| {
                            evaluation
                                .previous
                                .iter()
                                .any(|snapshot| snapshot.point == point.id)
                        })
                        .expect("same operation");
                    assert!(row.environments.is_disjoint(
                        &cfg.points[evaluation.rhs[0].value.point.0].possible_environments
                    ));
                    assert_eq!(row.producers.len(), 1);
                }
                "backing" => {
                    assert_eq!(row.backings.len(), 1);
                    let storage = available_place(&row.source);
                    assert_eq!(
                        row.backings,
                        before.state.backing_bindings[&storage.binding]
                    );
                    let result_state = pass
                        .observations
                        .points
                        .iter()
                        .find(|state| {
                            state.point == result.id
                                && state.phase == ReferenceTransferPhase::BeforeInputCleanup
                        })
                        .unwrap();
                    assert!(result_state.state.backing_bindings[&storage.binding]
                        .iter()
                        .all(|replacement| replacement.value != row.backings[0].value));
                }
                "aggregate_checked" => {
                    assert_eq!(row.projected_checked.len(), 2);
                    assert!(row
                        .projected_checked
                        .iter()
                        .all(|field| field.place.binding == *holder
                            && field.place.projections.len() == 1));
                    assert_ne!(
                        row.projected_checked[0].origins,
                        row.projected_checked[1].origins
                    );
                }
                "aggregate_backing" => {
                    assert_eq!(row.projected_backings.len(), 1);
                    assert_eq!(row.projected_backings[0].place.binding, *holder);
                    assert_eq!(row.projected_backings[0].place.projections.len(), 1);
                }
                "shared" => {
                    assert_eq!(row.relations.len(), 1);
                    let returned = cfg
                        .points
                        .iter()
                        .find(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
                        .unwrap();
                    assert!(returned
                        .possible_environments
                        .contains(&row.relations[0].environment));
                    let output = pass
                        .observations
                        .points
                        .iter()
                        .find(|state| {
                            state.point == returned.id
                                && state.phase == ReferenceTransferPhase::BeforeInputCleanup
                        })
                        .unwrap();
                    let live = output
                        .state
                        .holder_relations
                        .iter()
                        .find(|relation| {
                            relation.environment == row.relations[0].environment
                                && relation.holders.iter().any(|holder| {
                                    matches!(
                                        holder,
                                        CallableHolderPlace::OutputProjection { output: 0, .. }
                                    )
                                })
                        })
                        .unwrap();
                    let original = &row.relations[0].capture_paths[0][0];
                    let current = live
                        .capture_paths
                        .iter()
                        .flatten()
                        .find(|input| input.binding == original.binding)
                        .unwrap();
                    assert!(!original.backings.is_empty());
                    assert!(!current.backings.is_empty());
                    assert_ne!(original.backings[0].value, current.backings[0].value);
                }
                "ordered" | "nested" => {}
                _ => panic!("unexpected source function"),
            }
        }
    }
    assert_eq!(functions, 8);
    assert_eq!(saved, 10);
}

#[test]
fn assignment_saved_flow_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_saved_flow_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_saved_flow_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_saved_flow_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
