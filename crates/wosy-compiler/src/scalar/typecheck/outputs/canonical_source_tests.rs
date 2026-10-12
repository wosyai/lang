use super::*;
use crate::parse_source;

const NESTED: &str = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end";
const INDEPENDENT: &str = "%%start\n(i32, bool)() pair = fn { if (true) { 1 } else { 2 }, if (false) { true } else { false } };\n%%end";
const SINGLE: &str = "%%start\ni32() pair = fn { if (true) { 1 } else { 2 } };\n%%end";
const FIXED: &str = "%%start\n(u8[], bool)() pair = fn { u8[2] left = [4, 5]; u8[2] right = [6, 7]; if (true) { left, true } else { right, false } };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    for (family, text) in [("nested", NESTED), ("independent", INDEPENDENT), ("single", SINGLE), ("fixed", FIXED)] {
        let source = SourceIdentity::new("r1c-canonical-source-proof".into(), "app".into(), format!("src/{family}.w"), "r1".into());
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        let validation = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        let (items, expressions) = if project {
            let validation = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
            assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
            let module = &validation.project.modules[0];
            let expressions = module.resolved_expression_outputs.borrow().clone();
            (module.items.clone(), expressions)
        } else {
            (validation.program.items.clone(), validation.program.resolved_expression_outputs.borrow().clone())
        };
        let function = items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "pair" => Some(function),
            _ => None,
        }).expect("actual parsed conditional producer");
        let retained = function.body.clone();
        let bytes = serde_json::to_vec(&retained).expect("original typed AST bytes");
        let facts = assignment::SourceValueTypeFacts { assignments: Vec::new(), expressions: expressions.clone() };
        let ScalarType::Callable { outputs: receiving, .. } = &function.signature else { panic!("actual receiving signature"); };
        let group = group_conditional_final_outputs(&source, &function.body).expect("canonical inverse");
        if family == "nested" || family == "fixed" {
            let group = group.expect("real projected whole-source conditional");
            let span = SourceSpan::new(source.clone(), expression_span(&group.expression));
            let rows = expressions.iter().filter(|fact| fact.source_span == span).collect::<Vec<_>>();
            let [row] = rows.as_slice() else { panic!("one complete canonical span fact"); };
            assert_eq!(row.outputs.outputs.len(), 2);
            assert_eq!(facts.selected_output(&span, 1), Ok(&ScalarType::Bool));
            if family == "nested" {
                assert_eq!(facts.selected_output(&span, 0), Ok(&ScalarType::I32));
                let ScalarExpression::If { then_branch, .. } = &group.expression else { panic!("whole outer If"); };
                let inner = group_conditional_final_outputs(&source, then_branch).expect("inner inverse").expect("whole nested source");
                let inner_span = SourceSpan::new(source.clone(), expression_span(&inner.expression));
                assert_ne!(inner_span, span);
                assert_eq!(facts.selected_output(&inner_span, 0), Ok(&ScalarType::I32));
                assert_eq!(facts.selected_output(&inner_span, 1), Ok(&ScalarType::Bool));
                let rows = expressions.iter().filter(|fact| fact.source_span == inner_span).collect::<Vec<_>>();
                let [row] = rows.as_slice() else { panic!("one complete original inner sequence"); };
                assert_eq!(row.outputs.outputs.len(), 2);
            } else {
                assert!(matches!(facts.selected_output(&span, 0), Ok(ScalarType::Array { .. })));
                assert!(matches!(receiving.outputs[0].ty, ScalarType::RuntimeArray { .. }));
                assert_eq!(receiving.outputs[1].ty, ScalarType::Bool);
                let shape = function.reference_cfg.evaluated_values.shapes.iter().find(|shape| matches!(shape.ty, ScalarType::Array { .. })).expect("actual fixed source shape");
                assert_eq!(facts.selected_output(&span, 0), Ok(&shape.ty));
            }
            let table = RefCell::new(expressions.clone());
            let result = block_outputs(&table, &source, &function.body, &ScalarType::Error).expect("finish consumes complete source once");
            assert_eq!(result.outputs, row.outputs.outputs);
            assert_eq!(*table.borrow(), expressions);
            let missing = RefCell::new(expressions.iter().filter(|fact| fact.source_span != span).cloned().collect::<Vec<_>>());
            let error = block_outputs(&missing, &source, &function.body, &ScalarType::I32).expect_err("missing original whole-source fact retains qualified correspondence error");
            assert!(matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == span));
            let mut foreign_facts = expressions.clone();
            let fact = foreign_facts.iter_mut().find(|fact| fact.source_span == span).expect("actual canonical source row");
            fact.source_span.source = SourceIdentity::new("foreign".into(), "app".into(), "src/foreign.w".into(), "r1".into());
            let foreign_facts = RefCell::new(foreign_facts);
            let error = block_outputs(&foreign_facts, &source, &function.body, &ScalarType::I32).expect_err("foreign span-qualified type row supplies zero local source incidence");
            assert!(matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == span));
            for malformed in 0..3 {
                let mut block = retained.clone();
                let expected_span = match malformed {
                    0 => {
                        let ScalarExpression::If { then_branch, .. } = &mut block.final_output_values[0].value else { panic!("actual projected arm"); };
                        then_branch.occurrence.output_projection.clear();
                        SourceSpan::new(source.clone(), then_branch.span)
                    }
                    1 => {
                        block.final_output_values[1].position = 0;
                        span.clone()
                    }
                    2 => {
                        let ScalarExpression::If { then_branch, .. } = &mut block.final_output_values[0].value else { panic!("actual projected arm"); };
                        then_branch.occurrence.source_span.source = SourceIdentity::new("foreign".into(), "app".into(), "src/foreign.w".into(), "r1".into());
                        SourceSpan::new(source.clone(), then_branch.span)
                    }
                    _ => unreachable!(),
                };
                let error = block_outputs(&table, &source, &block, &ScalarType::I32).expect_err("malformed canonical source rejected by active finish reader");
                assert!(matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span == expected_span));
                assert_eq!(*table.borrow(), expressions);
            }
        } else {
            assert!(group.is_none());
            for (position, output) in function.body.final_output_values.iter().enumerate() {
                let span = SourceSpan::new(source.clone(), expression_span(&output.value));
                assert_eq!(facts.selected_output(&span, 0), Ok(&receiving.outputs[position].ty));
                assert!(facts.selected_output(&span, 1).is_err());
            }
        }
        assert_eq!(function.body, retained);
        assert_eq!(serde_json::to_vec(&function.body).expect("retained typed AST bytes"), bytes);
    }
}

#[test] fn r1c_canonical_source_types_program_native64() { prove(ScalarTargetLayout::NATIVE64, false); }
#[test] fn r1c_canonical_source_types_program_wasm32() { prove(ScalarTargetLayout::WASM32, false); }
#[test] fn r1c_canonical_source_types_project_native64() { prove(ScalarTargetLayout::NATIVE64, true); }
#[test] fn r1c_canonical_source_types_project_wasm32() { prove(ScalarTargetLayout::WASM32, true); }
