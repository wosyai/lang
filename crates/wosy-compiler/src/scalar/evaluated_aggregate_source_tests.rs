use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct copy Reader { u8() read; }\nstruct copy Bundle { Reader first; Reader second; }\n(u8())() make = fn { fn { 3 } };\nBundle() build = fn { u8() current = fn { 1 }; Bundle bundle = { .first = { .read = (current = fn { 2 }); }; .second = { .read = make(); }; }; bundle };\nunit() build_array = fn { (u8())[2] items = [fn { 4 }, fn { 5 }]; items; };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "evaluated-aggregate-source-proof".into(),
        "app".into(),
        "src/members.w".into(),
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
                .expect("source module")
                .items,
            trace,
        )
    } else {
        (validation.program.items, trace)
    };
    let mut aggregates = 0;
    let mut members = 0;
    let mut saved_members = 0;
    for item in &items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        let cfg = &function.reference_cfg;
        let pass = trace
            .passes
            .iter()
            .rfind(|pass| {
                pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
            })
            .expect("actual function transfer");
        assert!(pass.errors.is_empty(), "{:#?}", pass.errors);
        for point in &cfg.initial_points {
            let CfgPointKind::Evaluate(expression) = &point.kind else {
                continue;
            };
            let children = match expression {
                ScalarExpression::StructLiteral { fields, .. } => {
                    fields.iter().map(|field| &field.value).collect::<Vec<_>>()
                }
                ScalarExpression::ArrayLiteral { elements, .. } => elements.iter().collect(),
                _ => continue,
            };
            aggregates += 1;
            for child in children {
                members += 1;
                let occurrence =
                    SourceSpan::new(point.source_span.source.clone(), expression_span(child));
                let source_values = cfg
                    .evaluated_values
                    .values
                    .iter()
                    .filter(|value| {
                        value.id.resolved.producer == cfg.producer_owner
                            && value.id.resolved.occurrence == occurrence
                            && value.id.resolved.output == 0
                    })
                    .collect::<Vec<_>>();
                let values = if let ScalarExpression::Assignment(assignment) = child {
                    let evaluations = cfg
                        .evaluated_values
                        .assignment_evaluations
                        .iter()
                        .filter(|evaluation| {
                            evaluation.operation.occurrence == *assignment.occurrence
                        })
                        .collect::<Vec<_>>();
                    let [evaluation] = evaluations.as_slice() else {
                        panic!("unique actual member operation")
                    };
                    assert_eq!(evaluation.operation.producer, cfg.producer_owner);
                    assert_eq!(evaluation.operation.occurrence.source_span, occurrence);
                    let results = evaluation
                        .results
                        .iter()
                        .filter(|result| result.output == 0)
                        .collect::<Vec<_>>();
                    let [result] = results.as_slice() else {
                        panic!("unique actual member Result")
                    };
                    let registered = cfg
                        .evaluated_values
                        .results
                        .iter()
                        .filter(|registered| {
                            registered.operation == evaluation.operation && registered.output == 0
                        })
                        .collect::<Vec<_>>();
                    let [registered] = registered.as_slice() else {
                        panic!("unique registered member Result")
                    };
                    assert_eq!(result, registered);
                    assert_eq!(result.operation, evaluation.operation);
                    let previous = evaluation
                        .previous
                        .iter()
                        .filter(|snapshot| snapshot.point == result.snapshot)
                        .collect::<Vec<_>>();
                    let [previous] = previous.as_slice() else {
                        panic!("unique actual saved member Snapshot")
                    };
                    let snapshots = cfg
                        .evaluated_values
                        .snapshots
                        .iter()
                        .filter(|snapshot| snapshot.point == result.snapshot)
                        .collect::<Vec<_>>();
                    let [snapshot] = snapshots.as_slice() else {
                        panic!("unique registered saved member Snapshot")
                    };
                    assert_eq!(previous, snapshot);
                    assert_eq!(snapshot.destination.operation, evaluation.operation);
                    assert_eq!(snapshot.destination.target, 0);
                    let SnapshotState::Saved(saved) = &snapshot.state else {
                        panic!("actual saved member value")
                    };
                    assert_eq!(saved.point, result.snapshot);
                    assert_ne!(*saved, result.value);
                    assert_ne!(saved.point, result.value.point);
                    assert_eq!(saved.resolved, result.value.resolved);
                    assert_eq!(result.value.resolved.producer, cfg.producer_owner);
                    assert_eq!(result.value.resolved.occurrence, occurrence);
                    assert_eq!(result.value.resolved.output, 0);
                    assert_eq!(
                        result.value.resolved.output_projection,
                        assignment.occurrence.output_projection
                    );
                    assert_eq!(source_values.len(), 2);
                    assert_eq!(
                        cfg.evaluated_values
                            .values
                            .iter()
                            .filter(|value| {
                                value.id.resolved.occurrence == occurrence
                                    && value.id.resolved.output == 0
                            })
                            .count(),
                        2
                    );
                    let saved_values = source_values
                        .iter()
                        .copied()
                        .filter(|value| value.id == *saved)
                        .collect::<Vec<_>>();
                    let [saved_value] = saved_values.as_slice() else {
                        panic!("exact saved Snapshot source incidence")
                    };
                    let result_values = source_values
                        .iter()
                        .copied()
                        .filter(|value| value.id == result.value)
                        .collect::<Vec<_>>();
                    let [result_value] = result_values.as_slice() else {
                        panic!("exact returned Result source incidence")
                    };
                    assert_eq!(saved_value.id.resolved, result_value.id.resolved);
                    assert_eq!(saved_value.shape, result_value.shape);
                    let destinations = evaluation
                        .destinations
                        .iter()
                        .filter(|destination| destination.id == snapshot.destination)
                        .collect::<Vec<_>>();
                    let [destination] = destinations.as_slice() else {
                        panic!("actual saved member destination")
                    };
                    let snapshot_point = &cfg.initial_points[saved.point.0];
                    assert_eq!(snapshot_point.id, saved.point);
                    assert_eq!(snapshot_point.source_span, destination.source_span);
                    assert!(matches!(snapshot_point.kind,
                        CfgPointKind::AssignmentSnapshot { destination_point, observed: true, .. }
                            if destination_point == destination.point));
                    let result_point = &cfg.initial_points[result.value.point.0];
                    assert_eq!(result_point.id, result.value.point);
                    assert_eq!(result_point.source_span, occurrence);
                    assert!(matches!(result_point.kind,
                        CfgPointKind::AssignmentResult { snapshot_point, output: 0 }
                            if snapshot_point == result.snapshot));
                    result_values
                } else {
                    source_values
                };
                let [value] = values.as_slice() else {
                    panic!("unique actual aggregate member: {values:#?}")
                };
                let shape = cfg
                    .evaluated_values
                    .shapes
                    .iter()
                    .find(|shape| shape.id == value.shape)
                    .expect("actual member shape");
                assert_eq!(shape.id.producer, cfg.producer_owner);
                assert_eq!(
                    cfg.initial_points[value.id.point.0].source_span.source,
                    occurrence.source
                );
                match child {
                    ScalarExpression::FunctionLiteral { .. } => {
                        assert!(matches!(
                            cfg.initial_points[value.id.point.0].kind,
                            CfgPointKind::Capture { .. }
                        ));
                        assert!(matches!(shape.payload, ValuePayloadShape::Callable { .. }));
                    }
                    ScalarExpression::StructLiteral { .. } => {
                        assert!(matches!(&cfg.initial_points[value.id.point.0].kind,
                            CfgPointKind::Evaluate(actual) if actual == child));
                        assert!(matches!(shape.payload, ValuePayloadShape::Struct { .. }));
                    }
                    ScalarExpression::Call { occurrence, .. } => {
                        let productions = cfg
                            .evaluated_values
                            .productions
                            .iter()
                            .filter(|production| {
                                production.producer == cfg.producer_owner
                                    && production.occurrence == occurrence.source_span
                                    && production.output_projection == occurrence.output_projection
                            })
                            .collect::<Vec<_>>();
                        let [production] = productions.as_slice() else {
                            panic!("actual member call production")
                        };
                        assert_eq!(production.outputs, vec![value.id.clone()]);
                        assert!(matches!(
                            cfg.initial_points[value.id.point.0].kind,
                            CfgPointKind::Call { output: 0, .. }
                        ));
                    }
                    ScalarExpression::Assignment(assignment) => {
                        saved_members += 1;
                        let evaluation = cfg
                            .evaluated_values
                            .assignment_evaluations
                            .iter()
                            .find(|evaluation| {
                                evaluation.operation.occurrence == *assignment.occurrence
                            })
                            .expect("actual nested member operation");
                        let [result] = evaluation.results.as_slice() else {
                            panic!("single saved member")
                        };
                        assert_eq!(result.value, value.id);
                        assert!(matches!(cfg.initial_points[value.id.point.0].kind,
                            CfgPointKind::AssignmentResult { snapshot_point, output: 0 }
                                if snapshot_point == result.snapshot));
                    }
                    _ => panic!("source member kind"),
                }
            }
        }
    }
    assert_eq!(aggregates, 4);
    assert_eq!(members, 6);
    assert_eq!(saved_members, 1);
}

#[test]
fn evaluated_aggregate_source_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn evaluated_aggregate_source_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn evaluated_aggregate_source_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn evaluated_aggregate_source_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
