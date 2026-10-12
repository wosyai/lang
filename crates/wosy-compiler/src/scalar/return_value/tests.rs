use super::*;
use crate::parse_source;

const SIMPLE: &str = "%%start\ni32(bool) choose = fn(flag) { if (flag) { 1 } else { 2 } };\ni32(bool) forward = fn(flag) { choose(flag) };\ni32(bool) named = fn(flag) { i32 result = forward(flag); result };\n%%end";
const FIXED: &str = "%%start\nstruct Payload { u8 value; }\nu8[]() copy_out = fn { u8[2] fixed = [4, 5]; fixed[0]; fixed };\nPayload[]() make = fn { Payload[1] fixed = [{ .value = 5; }]; fixed };\nstruct Factory { u8[](u8[2]) widen; }\nFactory factory = { .widen = fn(fixed) { fixed }; };\n%%end";

fn assert_source_fact(
    cfg: &ScalarReferenceCfg,
    trace: &reference::test_observations::ReferenceValidationTestTrace,
    expression: &ScalarExpression,
    source: &SourceIdentity,
    source_type: &ScalarType,
) -> CfgPointId {
    let occurrence = SourceSpan::new(source.clone(), expression_span(expression));
    let facts = cfg
        .evaluated_values
        .values
        .iter()
        .filter(|fact| fact.id.resolved.occurrence == occurrence)
        .collect::<Vec<_>>();
    assert_eq!(facts.len(), 1);
    let fact = facts[0];
    assert_eq!(fact.id.resolved.producer, cfg.producer_owner);
    assert_eq!(fact.id.resolved.output, 0);
    let projection = match expression {
        ScalarExpression::Call { occurrence, .. } => occurrence.output_projection.clone(),
        _ => Vec::new(),
    };
    assert_eq!(fact.id.resolved.output_projection, projection);
    let construction = trace.construction_fact(cfg, fact).expect("actual initial source fact");
    assert!(construction.cases.is_empty());
    if source_type == &ScalarType::I32 {
        trace.selected_plain_fact(cfg, fact).expect("qualified actual selected Plain case incidence");
    }
    assert_eq!(fact.shape.producer, cfg.producer_owner);
    let shape = cfg
        .evaluated_values
        .shapes
        .iter()
        .find(|shape| shape.id == fact.shape)
        .expect("actual resolved source shape");
    assert_eq!(&shape.ty, source_type);
    let point = &cfg.initial_points[fact.id.point.0];
    assert_eq!(point.source_span, occurrence);
    match expression {
        ScalarExpression::Name { .. } => assert!(matches!(point.kind, CfgPointKind::Read { .. })),
        ScalarExpression::Integer { .. } => {
            assert_eq!(point.kind, CfgPointKind::Evaluate(expression.clone()))
        }
        ScalarExpression::If { .. } => assert_eq!(point.kind, CfgPointKind::Join),
        ScalarExpression::Call { .. } => assert!(matches!(&point.kind,
            CfgPointKind::Call { expression: actual, output: 0, .. } if actual == expression)),
        _ => panic!("proof source category"),
    }
    assert_eq!(
        cfg.initial_points
            .iter()
            .filter(|candidate| {
                candidate.source_span == occurrence
                    && std::mem::discriminant(&candidate.kind)
                        == std::mem::discriminant(&point.kind)
            })
            .count(),
        1,
        "actual source evaluated exactly once"
    );
    fact.id.point
}

fn root_output(cfg: &ScalarReferenceCfg, value: CfgPointId) -> &RuntimeReturnBackingAction {
    let returns = cfg
        .initial_points
        .iter()
        .filter_map(|point| match &point.kind {
            CfgPointKind::ReturnOutput {
                output: 0,
                value_point,
                backing_action,
                ..
            } => {
                assert_eq!(point.scope, ReferenceScopeId(0));
                assert_eq!(*value_point, value);
                Some(backing_action)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(returns.len(), 1);
    assert_eq!(
        cfg.initial_points
            .iter()
            .filter(|point| { matches!(point.kind, CfgPointKind::ReturnOutput { .. }) })
            .count(),
        1
    );
    returns[0]
}

fn prove(layout: ScalarTargetLayout, project: bool, fixed: bool) {
    let (text, path) = if fixed {
        (FIXED, "src/fixed.w")
    } else {
        (SIMPLE, "src/simple.w")
    };
    let source = SourceIdentity::new(
        "return-producer-proof".into(),
        "app".into(),
        path.into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, original_trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let original_items = validation.program.items.clone();
    let original_facts = validation
        .program
        .resolved_expression_outputs
        .borrow()
        .clone();
    let (items, facts, top_cfg, trace) = if project {
        let (validated, trace) = validate_scalar_project_observed_test(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(
            validated.diagnostics.is_empty(),
            "{:#?}",
            validated.diagnostics
        );
        let module = &validated.project.modules[0];
        let facts = module.resolved_expression_outputs.borrow().clone();
        (
            module.items.clone(),
            facts,
            module.top_level_reference_cfg.clone(),
            trace,
        )
    } else {
        (
            validation.program.items,
            original_facts.clone(),
            validation.program.top_level_reference_cfg.clone(),
            original_trace.clone(),
        )
    };
    for item in &items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        let original = original_items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(original) if original.name == function.name => Some(original),
                _ => None,
            })
            .expect("original parsed-source function");
        assert_eq!(function.body.occurrence, original.body.occurrence);
        assert_eq!(
            function.body.final_output_values,
            original.body.final_output_values
        );
        let cfg = &function.reference_cfg;
        assert!(matches!(&cfg.producer_owner, ProducerCfgOwnerId::Function {
            source: actual, declaration, body: ProducerFunctionBodyId::Declaration
        } if actual == &source && *declaration == function.name_span));
        let expression = &function.body.final_output_values[0].value;
        let occurrence = SourceSpan::new(source.clone(), expression_span(expression));
        let resolved = facts
            .iter()
            .find(|fact| fact.source_span == occurrence)
            .expect("resolved source outputs");
        assert_eq!(resolved.outputs.outputs.len(), 1);
        let source_type = &resolved.outputs.outputs[0].ty;
        let original_resolved = original_facts
            .iter()
            .find(|fact| fact.source_span == occurrence)
            .expect("original source output correspondence");
        assert_eq!(resolved, original_resolved);
        let point = assert_source_fact(cfg, &trace, expression, &source, source_type);
        let action = root_output(cfg, point);
        let original_point =
            assert_source_fact(&original.reference_cfg, &original_trace, expression, &source, source_type);
        assert_eq!(action, root_output(&original.reference_cfg, original_point));
        if fixed {
            assert!(matches!(source_type, ScalarType::Array { .. }));
            assert!(matches!(
                function.body.final_output_values[0].ty,
                ScalarType::RuntimeArray { .. }
            ));
            let ScalarType::Callable { outputs, .. } = &function.signature else {
                panic!("callable signature")
            };
            assert!(matches!(
                outputs.outputs[0].ty,
                ScalarType::RuntimeArray { .. }
            ));
            let source_binding = match action {
                RuntimeReturnBackingAction::CopyFixed { source } if function.name == "copy_out" => {
                    source
                }
                RuntimeReturnBackingAction::MoveFixed { source } if function.name == "make" => {
                    source
                }
                _ => panic!("unchanged actual fixed return action {action:?}"),
            };
            assert!(
                matches!(&cfg.initial_points[point.0].kind, CfgPointKind::Read { binding } if binding == source_binding)
            );
        } else {
            assert_eq!(source_type, &ScalarType::I32);
            assert_eq!(action, &RuntimeReturnBackingAction::Transfer);
            if let ScalarExpression::If {
                then_branch,
                else_branch,
                ..
            } = expression
            {
                for branch in [then_branch, else_branch] {
                    assert_eq!(branch.occurrence.source_span.source, source);
                    let expression = &branch.final_output_values[0].value;
                    let branch_point =
                        assert_source_fact(cfg, &trace, expression, &source, &ScalarType::I32);
                    assert_ne!(
                        cfg.initial_points[branch_point.0].scope,
                        ReferenceScopeId(0)
                    );
                    assert!(branch_point < point);
                }
                assert_eq!(cfg.evaluated_values.values.len(), 3);
            } else if function.name == "forward" {
                assert!(
                    matches!(expression, ScalarExpression::Call { name, .. } if name == "choose")
                );
                assert_eq!(cfg.initial_points.iter().filter(|point| matches!(&point.kind,
                    CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, output: 0, .. }
                        if name == "choose")).count(), 1);
            } else {
                assert!(
                    matches!(expression, ScalarExpression::Name { name, .. } if name == "result")
                );
                assert_eq!(cfg.initial_points.iter().filter(|point| matches!(&point.kind,
                    CfgPointKind::Call { expression: ScalarExpression::Call { name, .. }, output: 0, .. }
                        if name == "forward")).count(), 1);
            }
        }
    }
    if fixed {
        let literals = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Binding(binding) if binding.name == "factory" => Some(binding),
                _ => None,
            })
            .expect("actual callable-field source");
        let ScalarExpression::StructLiteral { fields, .. } = &literals.value else {
            panic!("factory initializer")
        };
        let literal_expression = &fields[0].value;
        let ScalarExpression::FunctionLiteral { body, span, .. } = literal_expression else {
            panic!("captureless field literal")
        };
        assert_eq!(body.items.len(), 1);
        let literal = SourceSpan::new(source.clone(), *span);
        let literal_cfg = top_cfg
            .literal_cfgs
            .iter()
            .find(|cfg| cfg.literal == literal)
            .expect("actual source field literal CFG");
        assert!(literal_cfg.captures.is_empty());
        let cfg = &literal_cfg.reference_cfg;
        assert!(matches!(&cfg.producer_owner, ProducerCfgOwnerId::Literal {
            literal: actual, enclosing, capture_point
        } if actual == &literal && enclosing.as_ref() == &top_cfg.producer_owner
            && *capture_point == literal_cfg.capture_point));
        let CfgPointKind::Capture {
            signature:
                ScalarType::Callable {
                    outputs,
                    parameters,
                },
            ..
        } = &top_cfg.initial_points[literal_cfg.capture_point.0].kind
        else {
            panic!("actual field literal signature producer")
        };
        assert!(matches!(
            outputs.outputs[0].ty,
            ScalarType::RuntimeArray { .. }
        ));
        assert!(matches!(parameters[0], ScalarType::Array { length: 2, .. }));
        let expression = &body.final_output_values[0].value;
        let resolved = facts
            .iter()
            .find(|fact| {
                fact.source_span == SourceSpan::new(source.clone(), expression_span(expression))
            })
            .expect("literal resolved fixed parameter source");
        assert_eq!(resolved.outputs.outputs.len(), 1);
        assert!(matches!(
            resolved.outputs.outputs[0].ty,
            ScalarType::Array { length: 2, .. }
        ));
        let point = assert_source_fact(cfg, &trace, expression, &source, &resolved.outputs.outputs[0].ty);
        let fact = cfg.evaluated_values.values.iter().find(|fact| fact.id.point == point)
            .expect("actual literal source fact");
        trace.assert_construction_rejections(cfg, fact);
        let parent_capture = &top_cfg.initial_points[literal_cfg.capture_point.0];
        assert_eq!(parent_capture.source_span, literal);
        assert!(matches!(&parent_capture.kind,
            CfgPointKind::Capture { literal: actual, body: actual_body, signature: ScalarType::Callable { outputs: actual_outputs, parameters: actual_parameters }, .. }
            if *actual == *span && actual_body.occurrence == body.occurrence
                && actual_body.span == body.span && actual_outputs == outputs
                && actual_parameters == parameters));
        assert!(matches!(
            root_output(cfg, point),
            RuntimeReturnBackingAction::CopyFixed { .. }
        ));
    }
}

#[test]
fn return_final_output_producer_correspondence_simple_scalar_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false, false);
}
#[test]
fn return_final_output_producer_correspondence_simple_scalar_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false, false);
}
#[test]
fn return_final_output_producer_correspondence_simple_scalar_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true, false);
}
#[test]
fn return_final_output_producer_correspondence_simple_scalar_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true, false);
}
#[test]
fn return_final_output_producer_correspondence_fixed_scalar_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false, true);
}
#[test]
fn return_final_output_producer_correspondence_fixed_scalar_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false, true);
}
#[test]
fn return_final_output_producer_correspondence_fixed_scalar_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true, true);
}
#[test]
fn return_final_output_producer_correspondence_fixed_scalar_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true, true);
}

mod call_proofs;
mod graph_correspondence;
mod final_consumer_proofs;
mod construction_case_proofs;
mod control_tuple_proofs;

#[test]
fn return_ordered_output_correspondence_call_tuple_scalar_program_native64() {
    call_proofs::prove_calls(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn return_ordered_output_correspondence_call_tuple_scalar_program_wasm32() {
    call_proofs::prove_calls(ScalarTargetLayout::WASM32, false);
}
#[test]
fn return_ordered_output_correspondence_call_tuple_scalar_project_native64() {
    call_proofs::prove_calls(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn return_ordered_output_correspondence_call_tuple_scalar_project_wasm32() {
    call_proofs::prove_calls(ScalarTargetLayout::WASM32, true);
}
#[test]
fn return_ordered_output_correspondence_qualified_table_roundtrip() {
    call_proofs::prove_roundtrip();
}
#[test]
fn return_ordered_output_correspondence_qualified_projection_remap() {
    call_proofs::prove_remap();
}

#[test]
fn return_ordered_output_correspondence_callable_first_receiver_scalar_program_native64() {
    call_proofs::prove_callable_receivers(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn return_ordered_output_correspondence_callable_first_receiver_scalar_program_wasm32() {
    call_proofs::prove_callable_receivers(ScalarTargetLayout::WASM32, false);
}
#[test]
fn return_ordered_output_correspondence_callable_first_receiver_scalar_project_native64() {
    call_proofs::prove_callable_receivers(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn return_ordered_output_correspondence_callable_first_receiver_scalar_project_wasm32() {
    call_proofs::prove_callable_receivers(ScalarTargetLayout::WASM32, true);
}

fn prove_return_semantic_array_agreement(
    layout: ScalarTargetLayout,
    unequal_types: bool,
    corrupt_identity: bool,
) {
    let source = SourceIdentity::new(
        "return-semantic-array-proof".into(),
        "app".into(),
        "src/simple.w".into(),
        "r1".into(),
    );
    let original_source = source.clone();
    let text = SIMPLE.to_owned();
    let original_text = text.clone();
    let parsed = parse_source(source.clone(), text.clone(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let original_program = validation.program.clone();
    let functions = validation.program.items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) if function.name == "named" => Some(function),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(functions.len(), 1);
    let function = functions[0];
    assert_eq!(function.body.final_output_values.len(), 1);
    let expression = &function.body.final_output_values[0].value;
    assert!(matches!(expression, ScalarExpression::Name { name, .. } if name == "result"));
    let original_expression = expression.clone();
    let occurrence = SourceSpan::new(source.clone(), expression_span(expression));
    let original_cfg = function.reference_cfg.clone();
    let original_facts = validation.program.resolved_expression_outputs.borrow().clone();
    let resolved_indices = original_facts.iter().enumerate()
        .filter(|(_, fact)| fact.source_span == occurrence)
        .map(|(index, _)| index).collect::<Vec<_>>();
    assert_eq!(resolved_indices.len(), 1);
    let resolved_index = resolved_indices[0];
    let original_outputs = original_facts[resolved_index].outputs.clone();
    assert_eq!(original_outputs.outputs.len(), 1);
    assert_eq!(original_outputs.outputs[0].ty, ScalarType::I32);
    let value_indices = original_cfg.evaluated_values.values.iter().enumerate()
        .filter(|(_, fact)| fact.id.resolved.occurrence == occurrence && fact.id.resolved.output == 0)
        .map(|(index, _)| index).collect::<Vec<_>>();
    assert_eq!(value_indices.len(), 1);
    let value_index = value_indices[0];
    let original_fact = original_cfg.evaluated_values.values[value_index].clone();
    assert_eq!(original_fact.id.resolved.producer, original_cfg.producer_owner);
    assert_eq!(original_fact.id.resolved.occurrence, occurrence);
    assert_eq!(original_fact.id.resolved.output, 0);
    assert!(original_fact.id.resolved.output_projection.is_empty());
    assert_eq!(original_fact.shape.producer, original_cfg.producer_owner);
    let shape_indices = original_cfg.evaluated_values.shapes.iter().enumerate()
        .filter(|(_, shape)| shape.id == original_fact.shape)
        .map(|(index, _)| index).collect::<Vec<_>>();
    assert_eq!(shape_indices.len(), 1);
    let shape_index = shape_indices[0];
    let original_shape = original_cfg.evaluated_values.shapes[shape_index].clone();
    assert_eq!(original_shape.ty, ScalarType::I32);
    let point = &original_cfg.points[original_fact.id.point.0];
    assert_eq!(point.id, original_fact.id.point);
    assert_eq!(point.source_span, occurrence);
    assert!(matches!(&point.kind, CfgPointKind::Read { .. }));
    assert_eq!(original_cfg.points.iter().filter(|point| point.id == original_fact.id.point).count(), 1);
    let sequence = assignment::EvaluatedOutputSequence {
        entries: vec![assignment::EvaluatedOutputEntry {
            value: original_fact.id.clone(),
            shape: original_fact.shape.clone(),
        }],
    };
    let original_sequence = sequence.clone();
    let shape_type = ScalarType::Array {
        element: Box::new(ScalarType::U8),
        length: 2,
        length_span: ByteSpan::new(31, 32),
        span: ByteSpan::new(28, 33),
    };
    let receiving_type = ScalarType::Array {
        element: Box::new(ScalarType::U8),
        length: 2,
        length_span: ByteSpan::new(55, 56),
        span: ByteSpan::new(52, 57),
    };
    assert_ne!(shape_type, receiving_type);
    assert!(scalar_type_equal(&shape_type, &receiving_type));
    let mut builder = ReferenceFlowBuilder::test_builder(&source, function);
    builder.cfg = original_cfg.clone();
    builder.value_types.expressions = original_facts.clone();
    builder.cfg.evaluated_values.shapes[shape_index].ty = shape_type.clone();
    let mut outputs = original_outputs.clone();
    outputs.outputs[0].ty = receiving_type.clone();
    builder.value_types.expressions[resolved_index].outputs = outputs.clone();
    let valid_cfg = builder.cfg.clone();
    let valid_facts = builder.value_types.expressions.clone();
    let mut expected_cfg = original_cfg.clone();
    expected_cfg.evaluated_values.shapes[shape_index].ty = shape_type.clone();
    assert_eq!(valid_cfg, expected_cfg);
    let mut expected_facts = original_facts.clone();
    expected_facts[resolved_index].outputs.outputs[0].ty = receiving_type.clone();
    assert_eq!(valid_facts, expected_facts);
    assert_eq!(builder.cfg.evaluated_values.shapes[shape_index].ty, shape_type);
    assert_eq!(outputs.outputs[0].ty, receiving_type);
    assert_eq!(builder.cfg.evaluated_values.values[value_index], original_fact);
    assert_eq!(builder.cfg.evaluated_values.shapes[shape_index].id, original_shape.id);
    assert_eq!(builder.cfg.points, original_cfg.points);
    assert_eq!(builder.validate_final_output_entry(expression, &sequence.entries[0], &outputs.outputs[0].ty), Ok(()));
    assert_eq!(builder.validate_final_output_sequence(expression, &sequence, &outputs), Ok(()));
    assert_eq!(builder.bind_final_output_values(expression, original_fact.id.point), Ok(sequence.clone()));
    assert_eq!(builder.cfg, valid_cfg);
    assert_eq!(builder.value_types.expressions, valid_facts);
    assert_eq!(sequence, original_sequence);
    let expected_error = ReferenceAnalysisError::MissingContext {
        source_span: occurrence.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    };
    if unequal_types {
        for unequal_type in [
            ScalarType::Array {
                element: Box::new(ScalarType::U8),
                length: 3,
                length_span: ByteSpan::new(55, 56),
                span: ByteSpan::new(52, 57),
            },
            ScalarType::Array {
                element: Box::new(ScalarType::U16),
                length: 2,
                length_span: ByteSpan::new(55, 56),
                span: ByteSpan::new(52, 57),
            },
        ] {
            builder.cfg = valid_cfg.clone();
            builder.value_types.expressions = valid_facts.clone();
            let mut rejected_outputs = outputs.clone();
            rejected_outputs.outputs[0].ty = unequal_type.clone();
            builder.value_types.expressions[resolved_index].outputs = rejected_outputs.clone();
            let rejected_facts = builder.value_types.expressions.clone();
            assert!(!scalar_type_equal(&shape_type, &unequal_type));
            assert_eq!(builder.validate_final_output_entry(expression, &sequence.entries[0], &unequal_type), Err(expected_error.clone()));
            assert_eq!(builder.validate_final_output_sequence(expression, &sequence, &rejected_outputs), Err(expected_error.clone()));
            assert_eq!(builder.bind_final_output_values(expression, original_fact.id.point), Err(expected_error.clone()));
            assert_eq!(builder.cfg, valid_cfg);
            assert_eq!(builder.value_types.expressions, rejected_facts);
            assert_eq!(builder.cfg.evaluated_values.values[value_index], original_fact);
            assert_eq!(builder.cfg.evaluated_values.shapes[shape_index].id, original_shape.id);
            assert_eq!(sequence, original_sequence);
        }
    }
    if corrupt_identity {
        builder.cfg = valid_cfg.clone();
        builder.value_types.expressions = valid_facts.clone();
        let invalid_point = CfgPointId(builder.cfg.points.len());
        assert_ne!(invalid_point, original_fact.id.point);
        builder.cfg.evaluated_values.values[value_index].id.point = invalid_point;
        let rejected_cfg = builder.cfg.clone();
        let mut expected_rejected_cfg = valid_cfg.clone();
        expected_rejected_cfg.evaluated_values.values[value_index].id.point = invalid_point;
        assert_eq!(rejected_cfg, expected_rejected_cfg);
        assert_eq!(builder.cfg.evaluated_values.values[value_index].id.resolved, original_fact.id.resolved);
        assert_eq!(builder.cfg.evaluated_values.values[value_index].shape, original_fact.shape);
        assert_eq!(builder.cfg.evaluated_values.shapes[shape_index].id, original_shape.id);
        assert_eq!(builder.cfg.evaluated_values.shapes[shape_index].ty, shape_type);
        assert_eq!(builder.value_types.expressions[resolved_index].outputs, outputs);
        assert!(scalar_type_equal(&shape_type, &outputs.outputs[0].ty));
        assert_eq!(builder.validate_final_output_entry(expression, &sequence.entries[0], &outputs.outputs[0].ty), Err(expected_error.clone()));
        assert_eq!(builder.validate_final_output_sequence(expression, &sequence, &outputs), Err(expected_error.clone()));
        assert_eq!(builder.bind_final_output_values(expression, original_fact.id.point), Err(expected_error));
        assert_eq!(builder.cfg, rejected_cfg);
        assert_eq!(builder.value_types.expressions, valid_facts);
        assert_eq!(sequence, original_sequence);
    }
    assert_eq!(source, original_source);
    assert_eq!(text, original_text);
    assert_eq!(text, SIMPLE);
    assert_eq!(expression, &original_expression);
    assert_eq!(function.reference_cfg, original_cfg);
    assert_eq!(*validation.program.resolved_expression_outputs.borrow(), original_facts);
    assert_eq!(original_facts[resolved_index].outputs, original_outputs);
    assert_eq!(validation.program, original_program);
}

#[test]
fn return_semantic_array_agreement_distinct_spans_native64() {
    prove_return_semantic_array_agreement(ScalarTargetLayout::NATIVE64, false, false);
}

#[test]
fn return_semantic_array_agreement_distinct_spans_wasm32() {
    prove_return_semantic_array_agreement(ScalarTargetLayout::WASM32, false, false);
}

#[test]
fn return_semantic_array_agreement_rejects_unequal_types_native64() {
    prove_return_semantic_array_agreement(ScalarTargetLayout::NATIVE64, true, false);
}

#[test]
fn return_semantic_array_agreement_rejects_unequal_types_wasm32() {
    prove_return_semantic_array_agreement(ScalarTargetLayout::WASM32, true, false);
}

#[test]
fn return_semantic_array_agreement_preserves_occurrence_identity_native64() {
    prove_return_semantic_array_agreement(ScalarTargetLayout::NATIVE64, false, true);
}

#[test]
fn return_semantic_array_agreement_preserves_occurrence_identity_wasm32() {
    prove_return_semantic_array_agreement(ScalarTargetLayout::WASM32, false, true);
}
