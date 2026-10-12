use super::*;

const SOURCE: &str = "%%start\nenv = extern wasm \"helper\" { (u64, bool)() read; };\ni32() single = fn { i32 x = 1; x = 2 };\n(i32, u64)() pair = fn { i32 x = 1; u64 y = 2; x, y = 3, 4 };\ni32() nested = fn { i32 x = 1; x = (x = 2) };\n(i32, i32, u64, bool)() mixed = fn { i32 direct = 9; i32 x = 1; direct, (x = 2), env.read() };\n*i32(*i32, *i32) checked = fn(left, right) { left = right };\ni32() block_saved = fn { i32 x = 1; { x = 2 } };\ni32() discard_extra = fn { i32 x = 1; i32 y = 2; x, y = 3, 4 };\nunit() empty = fn {};\n%%end";

fn derive(layout: ScalarTargetLayout, project: bool, revision: &str) -> ScalarModule {
    derive_observed(layout, project, revision).0
}

fn derive_observed(layout: ScalarTargetLayout, project: bool, revision: &str) -> (
    ScalarModule,
    reference::test_observations::ReferenceValidationTestTrace,
) {
    let source = SourceIdentity::new(
        "assignment-final-consumer-proof".into(),
        "app".into(),
        "src/final.w".into(),
        revision.into(),
    );
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        (validation
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual project module"), trace)
    } else {
        (ScalarModule::from_program(validation.program, Vec::new()), trace)
    }
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let (module, trace) = derive_observed(layout, project, "r1");
    let mut functions = 0;
    for item in &module.items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        functions += 1;
        let cfg = &function.reference_cfg;
        assert_eq!(
            cfg.evaluated_values.results,
            cfg.evaluated_values
                .assignment_evaluations
                .iter()
                .flat_map(|evaluation| evaluation.results.clone())
                .collect::<Vec<_>>()
        );
        let ScalarType::Callable { outputs, .. } = &function.signature else {
            panic!("actual callable")
        };
        let returns = cfg
            .initial_points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
            .collect::<Vec<_>>();
        let inputs = &cfg.evaluated_values.final_inputs;
        let count = if function.name == "empty" {
            0
        } else {
            outputs.outputs.len()
        };
        assert_eq!(returns.len(), count, "{}", function.name);
        assert_eq!(inputs.len(), count, "{}", function.name);
        for (output, (returned, input)) in returns.iter().zip(inputs).enumerate() {
            assert_eq!(returned.scope, ReferenceScopeId(0));
            assert_eq!(
                input.consumer,
                ValueConsumerId {
                    producer: cfg.producer_owner.clone(),
                    point: returned.id,
                    kind: ValueConsumerKind::FinalOutput {
                        block: (*function.body.occurrence).clone(),
                        output
                    },
                }
            );
            assert!(
                matches!(&returned.kind, CfgPointKind::ReturnOutput { output: actual, value_point, .. }
                if *actual == output && *value_point == input.source.point)
            );
            assert_eq!(input.receiving_type, outputs.outputs[output].ty);
            let facts = cfg
                .evaluated_values
                .values
                .iter()
                .filter(|fact| fact.id == input.source)
                .collect::<Vec<_>>();
            assert_eq!(facts.len(), 1);
            let construction = trace.construction_fact(cfg, facts[0]).expect("actual initial source fact");
            assert!(construction.cases.is_empty());
            let shape = cfg
                .evaluated_values
                .shapes
                .iter()
                .find(|shape| shape.id == facts[0].shape)
                .expect("actual source shape");
            assert_eq!(shape.ty, input.receiving_type);
            assert_eq!(input.source.resolved.producer, cfg.producer_owner);
            assert_eq!(input.source.resolved.occurrence.source, module.source);
            assert!(input.source.point < returned.id);
        }
        let returned_results = cfg
            .evaluated_values
            .assignment_evaluations
            .iter()
            .flat_map(|evaluation| &evaluation.results)
            .filter(|result| inputs.iter().any(|input| input.source == result.value))
            .collect::<Vec<_>>();
        let expected_results = match function.name.as_str() {
            "single" | "nested" | "mixed" | "checked" | "discard_extra" => 1,
            "pair" => 2,
            "block_saved" | "empty" => 0,
            _ => panic!("proof source function"),
        };
        assert_eq!(
            returned_results.len(),
            expected_results,
            "{}",
            function.name
        );
        for result in returned_results {
            let input = inputs
                .iter()
                .find(|input| input.source == result.value)
                .unwrap();
            assert_eq!(result.consumers, vec![input.consumer.clone()]);
            let registered = cfg
                .evaluated_values
                .results
                .iter()
                .filter(|row| row.value == result.value)
                .collect::<Vec<_>>();
            assert_eq!(registered.len(), 1);
            assert_eq!(registered[0], result);
            assert_eq!(registered[0].consumers, vec![input.consumer.clone()]);
            let evaluation = cfg
                .evaluated_values
                .assignment_evaluations
                .iter()
                .find(|evaluation| evaluation.operation == result.operation)
                .unwrap();
            assert_eq!(
                result.value.resolved.occurrence,
                evaluation.operation.occurrence.source_span
            );
            assert_eq!(
                result.value.resolved.output_projection,
                evaluation.operation.occurrence.output_projection
            );
            assert_eq!(result.value.resolved.output, result.output);
            assert!(matches!(&cfg.initial_points[result.value.point.0].kind,
                CfgPointKind::AssignmentResult { snapshot_point, output } if *snapshot_point == result.snapshot && *output == result.output));
            let snapshot = evaluation
                .previous
                .iter()
                .find(|snapshot| snapshot.point == result.snapshot)
                .unwrap();
            let SnapshotState::Saved(saved) = &snapshot.state else {
                panic!("actual saved previous")
            };
            assert_ne!(saved, &result.value);
            let saved_fact = cfg
                .evaluated_values
                .values
                .iter()
                .find(|fact| fact.id == *saved)
                .unwrap();
            let result_fact = cfg
                .evaluated_values
                .values
                .iter()
                .find(|fact| fact.id == result.value)
                .unwrap();
            assert_eq!(saved_fact.shape, result_fact.shape);
            assert!(evaluation
                .writes
                .iter()
                .all(|commit| result.snapshot < commit.point && commit.point < result.value.point));
        }
        match function.name.as_str() {
            "nested" => {
                assert_eq!(cfg.evaluated_values.assignment_evaluations.len(), 2);
                let outer = cfg
                    .evaluated_values
                    .assignment_evaluations
                    .iter()
                    .find(|evaluation| evaluation.results[0].value == inputs[0].source)
                    .unwrap();
                let inner = cfg
                    .evaluated_values
                    .assignment_evaluations
                    .iter()
                    .find(|evaluation| evaluation.operation != outer.operation)
                    .unwrap();
                assert_eq!(outer.rhs[0].value, inner.results[0].value);
                assert!(inner.writes[0].point < outer.previous[0].point);
            }
            "mixed" => {
                assert!(matches!(
                    cfg.initial_points[inputs[0].source.point.0].kind,
                    CfgPointKind::Read { .. }
                ));
                assert_eq!(cfg.call_points.len(), 1);
                assert_eq!(cfg.call_points[0].projections.len(), 2);
                assert_eq!(
                    inputs[2].source.point,
                    cfg.call_points[0].projections[0].point
                );
                assert_eq!(
                    inputs[3].source.point,
                    cfg.call_points[0].projections[1].point
                );
                assert_eq!(inputs[2].source.resolved.output, 0);
                assert_eq!(inputs[3].source.resolved.output, 1);
                assert!(
                    inputs[0].source.point < inputs[1].source.point
                        && inputs[1].source.point < inputs[2].source.point
                );
            }
            "checked" => assert!(matches!(
                returns[0].kind,
                CfgPointKind::ReturnOutput { checked: true, .. }
            )),
            "block_saved" => {
                assert_eq!(cfg.evaluated_values.assignment_evaluations.len(), 1);
                let saved = &cfg.evaluated_values.assignment_evaluations[0].results[0].value;
                assert_ne!(cfg.initial_points[saved.point.0].scope, ReferenceScopeId(0));
                assert!(cfg
                    .evaluated_values
                    .values
                    .iter()
                    .any(|fact| fact.id == *saved));
                assert!(cfg
                    .initial_points
                    .iter()
                    .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
                    .all(|point| point.scope == ReferenceScopeId(0)));
            }
            "discard_extra" => {
                let evaluation = &cfg.evaluated_values.assignment_evaluations[0];
                assert_eq!(evaluation.results.len(), 2);
                assert_eq!(inputs[0].source, evaluation.results[0].value);
                assert!(!inputs
                    .iter()
                    .any(|input| input.source == evaluation.results[1].value));
                let discarded = &evaluation.results[1];
                assert_eq!(discarded.consumers.len(), 1);
                let consumer = &discarded.consumers[0];
                assert_eq!(consumer.producer, cfg.producer_owner);
                assert_eq!(consumer.kind, ValueConsumerKind::Discard);
                let point = &cfg.initial_points[consumer.point.0];
                assert_eq!(point.scope, ReferenceScopeId(0));
                assert_eq!(point.source_span, discarded.value.resolved.occurrence);
                assert_eq!(
                    point.kind,
                    CfgPointKind::AssignmentDispose {
                        snapshot_point: discarded.snapshot
                    }
                );
                assert!(discarded.value.point < point.id);
                assert!(cfg
                    .initial_points
                    .iter()
                    .filter(|exit| matches!(exit.kind, CfgPointKind::ScopeExit)
                        && exit.scope == ReferenceScopeId(0))
                    .all(|exit| point.id < exit.id));
                assert_eq!(
                    cfg.assignment_points
                        .iter()
                        .filter(|binding| {
                            binding.occurrence == evaluation.operation.occurrence
                                && binding.target == discarded.output
                                && binding.role == ScalarAssignmentPointRole::Dispose
                                && binding.point == point.id
                        })
                        .count(),
                    1
                );
                let registered = cfg
                    .evaluated_values
                    .results
                    .iter()
                    .filter(|row| row.value == evaluation.results[1].value)
                    .collect::<Vec<_>>();
                assert_eq!(registered.len(), 1);
                assert_eq!(registered[0], &evaluation.results[1]);
                assert_eq!(registered[0].consumers, vec![consumer.clone()]);
                let plans = cfg
                    .evaluated_values
                    .assignment_plans
                    .iter()
                    .filter(|plan| plan.operation == evaluation.operation)
                    .collect::<Vec<_>>();
                assert_eq!(plans.len(), 1);
                let AssignmentConsumption::Received { outputs, discards } = &plans[0].consumption
                else {
                    panic!("actual observed assignment plan")
                };
                assert_eq!(outputs, &vec![inputs[0].consumer.clone()]);
                assert_eq!(discards, &vec![discarded.value.resolved.clone()]);
            }
            _ => {}
        }
    }
    assert_eq!(functions, 8);
}

fn prove_remap(layout: ScalarTargetLayout, project: bool) {
    let original = derive(layout, project, "r1");
    let target = derive(layout, project, "r2");
    for (old, new) in original
        .items
        .iter()
        .zip(&target.items)
        .filter_map(|(old, new)| match (old, new) {
            (ScalarItem::Function(old), ScalarItem::Function(new)) => {
                Some((&old.reference_cfg, &new.reference_cfg))
            }
            _ => None,
        })
    {
        assert_ne!(old.producer_owner, new.producer_owner);
        assert_eq!(old.initial_points.len(), new.initial_points.len());
        let points = old
            .initial_points
            .iter()
            .zip(&new.initial_points)
            .map(|(old_point, new_point)| {
                assert_eq!(old_point.source_span.range, new_point.source_span.range);
                assert_eq!(
                    std::mem::discriminant(&old_point.kind),
                    std::mem::discriminant(&new_point.kind)
                );
                ValuePointRemap {
                    producer: old.producer_owner.clone(),
                    point: old_point.id,
                    imported: new_point.id,
                }
            })
            .collect();
        let shapes = old
            .evaluated_values
            .shapes
            .iter()
            .map(|shape| {
                let matched = new
                    .evaluated_values
                    .shapes
                    .iter()
                    .filter(|candidate| candidate.ty == shape.ty)
                    .collect::<Vec<_>>();
                assert_eq!(matched.len(), 1);
                (shape.id.clone(), matched[0].id.clone())
            })
            .collect();
        let remap = graph_correspondence::complete(&original, &target, old, new, points, shapes, Vec::new(), &|before, after| before == after);
        let context = graph_correspondence::context(&original, old);
        let sources = capture_table_sources(old, &context).expect("actual original Final closure");
        let forest = graph_correspondence::forest(&original);
        let mapped_graph = remap
            .tables(&sources, &forest)
            .expect("actual saved/final identity remap");
        let mapped = graph_correspondence::mapped_root(&old.producer_owner, &sources, &mapped_graph).clone();
        assert_eq!(
            mapped.results,
            mapped
                .assignment_evaluations
                .iter()
                .flat_map(|evaluation| evaluation.results.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            mapped.final_inputs.len(),
            old.evaluated_values.final_inputs.len()
        );
        for (before, after) in old
            .evaluated_values
            .final_inputs
            .iter()
            .zip(&mapped.final_inputs)
        {
            assert_eq!(after.consumer, remap.consumer(&before.consumer).unwrap());
            assert_eq!(after.source, remap.value(&before.source).unwrap());
            assert_eq!(after.receiving_type, before.receiving_type);
            assert_eq!(
                after.source.resolved.output_projection,
                before.source.resolved.output_projection
            );
            assert_eq!(
                after.source.resolved.occurrence,
                before.source.resolved.occurrence
            );
            let fact = mapped
                .values
                .iter()
                .find(|fact| fact.id == after.source)
                .unwrap();
            assert_eq!(fact.shape.producer, new.producer_owner);
            let ValueConsumerKind::FinalOutput { output, .. } = after.consumer.kind else {
                panic!("actual final kind")
            };
            assert!(matches!(&new.initial_points[after.consumer.point.0].kind,
                CfgPointKind::ReturnOutput { output: actual, value_point, .. } if *actual == output && *value_point == after.source.point));
            if let Some(result) = mapped
                .assignment_evaluations
                .iter()
                .flat_map(|evaluation| &evaluation.results)
                .find(|result| result.value == after.source)
            {
                assert_eq!(result.consumers, vec![after.consumer.clone()]);
                let registered = mapped
                    .results
                    .iter()
                    .filter(|row| row.value == after.source)
                    .collect::<Vec<_>>();
                assert_eq!(registered.len(), 1);
                assert_eq!(registered[0], result);
                assert_eq!(registered[0].consumers, vec![after.consumer.clone()]);
                assert!(matches!(&new.initial_points[result.value.point.0].kind,
                    CfgPointKind::AssignmentResult { snapshot_point, output } if *snapshot_point == result.snapshot && *output == result.output));
            }
        }
        let bytes = serde_json::to_vec(&mapped).unwrap();
        for (before, after) in old
            .evaluated_values
            .assignment_plans
            .iter()
            .zip(&mapped.assignment_plans)
        {
            if let AssignmentConsumption::Received {
                outputs: before_outputs,
                discards: before_discards,
            } = &before.consumption
            {
                let AssignmentConsumption::Received { outputs, discards } = &after.consumption
                else {
                    panic!("mapped received plan")
                };
                assert_eq!(
                    outputs,
                    &before_outputs
                        .iter()
                        .map(|consumer| remap.consumer(consumer).unwrap())
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    discards,
                    &before_discards
                        .iter()
                        .map(|discard| remap.output(discard).unwrap())
                        .collect::<Vec<_>>()
                );
            }
        }
        for before in old.evaluated_values.results.iter().filter(|result| {
            result
                .consumers
                .iter()
                .any(|consumer| consumer.kind == ValueConsumerKind::Discard)
        }) {
            let value = remap.value(&before.value).unwrap();
            let results = mapped
                .results
                .iter()
                .filter(|result| result.value == value)
                .collect::<Vec<_>>();
            assert_eq!(results.len(), 1);
            let result = results[0];
            assert_eq!(
                result.snapshot,
                remap.point(&old.producer_owner, before.snapshot).unwrap()
            );
            assert_eq!(
                result.consumers,
                before
                    .consumers
                    .iter()
                    .map(|consumer| remap.consumer(consumer).unwrap())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                new.initial_points[result.consumers[0].point.0].kind,
                CfgPointKind::AssignmentDispose {
                    snapshot_point: result.snapshot
                }
            );
            assert!(result.value.point < result.consumers[0].point);
        }
        let restored: EvaluatedValueTables = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, mapped);
    }
}

#[test]
fn assignment_final_consumer_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_final_consumer_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_final_consumer_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_final_consumer_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
#[test]
fn assignment_final_consumer_remap_program_native64() {
    prove_remap(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn assignment_final_consumer_remap_program_wasm32() {
    prove_remap(ScalarTargetLayout::WASM32, false);
}
#[test]
fn assignment_final_consumer_remap_project_native64() {
    prove_remap(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn assignment_final_consumer_remap_project_wasm32() {
    prove_remap(ScalarTargetLayout::WASM32, true);
}
