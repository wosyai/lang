use super::*;
use crate::parse_source;

const NESTED: &str = "%%start\n(i32, bool)() pair = fn { if (true) { if (true) { 1, true } else { 2, false } } else { 3, true } };\n%%end";
const EFFECTS: &str = "%%start\ni32() tick = fn { 7 };\n(i32, bool)() pair = fn { if (true) { i32 value = tick(); i32() read = fn { value }; i32[2] values = [1, 2]; i32 index = 0; values[index] = tick(); values[index]; while (false) { value = tick(); }; read(); value, true } else { i32 value = tick(); i32() read = fn { value }; read(); value, false } };\n%%end";
const INDEPENDENT: &str = "%%start\n(i32, bool)() pair = fn { if (true) { 1 } else { 2 }, if (false) { true } else { false } };\n%%end";
const SINGLE: &str = "%%start\ni32() pair = fn { if (true) { 1 } else { 2 } };\n%%end";

fn parsed_body(
    text: &str,
    family: &str,
    layout: ScalarTargetLayout,
    project: bool,
) -> (SourceIdentity, ScalarBlock) {
    let source = SourceIdentity::new(
        "conditional-group-proof".into(),
        "app".into(),
        format!("src/{family}.w"),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let items = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source.clone()],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        validation.project.modules[0].items.clone()
    } else {
        validation.program.items
    };
    let body = items
        .iter()
        .find_map(|item| match item {
            ScalarItem::Function(function) if function.name == "pair" => {
                Some(function.body.clone())
            }
            _ => None,
        })
        .expect("source-derived pair body");
    (source, body)
}

fn synchronize_final_items(block: &mut ScalarBlock) {
    let start = block.items.len() - block.final_output_values.len();
    for (position, output) in block.final_output_values.iter().enumerate() {
        block.items[start + position] = ScalarBlockItem::Expression(output.value.clone());
    }
    block.expressions = block
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarBlockItem::Expression(expression) => Some(expression.clone()),
            _ => None,
        })
        .collect();
}

fn negative(source: &SourceIdentity, block: &ScalarBlock) {
    let retained = block.clone();
    let error =
        group_conditional_final_outputs(source, block).expect_err("malformed projected source");
    let ReferenceAnalysisError::MissingContext {
        source_span,
        reason,
    } = error
    else {
        panic!("qualified correspondence error")
    };
    assert_eq!(source_span.source, *source);
    assert_eq!(
        reason,
        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence
    );
    assert_eq!(*block, retained);
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    for (family, text) in [("nested", NESTED), ("effects", EFFECTS)] {
        let (source, block) = parsed_body(text, family, layout, project);
        let retained = block.clone();
        let group = group_conditional_final_outputs(&source, &block)
            .expect("verified expansion")
            .expect("projected conditional");
        assert_eq!(block, retained);
        let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } = &group.expression
        else {
            panic!("whole source conditional")
        };
        assert_eq!(then_branch.final_output_values.len(), 2);
        assert_eq!(else_branch.final_output_values.len(), 2);
        assert_eq!(
            then_branch.occurrence.output_projection,
            block.occurrence.output_projection
        );
        assert_eq!(
            else_branch.occurrence.output_projection,
            block.occurrence.output_projection
        );
        for (position, selected) in block.final_output_values.iter().enumerate() {
            let ScalarExpression::If {
                condition: selected_condition,
                then_branch: selected_then,
                else_branch: selected_else,
                span: selected_span,
            } = &selected.value
            else {
                panic!("canonical selected conditional")
            };
            assert_eq!(condition, selected_condition);
            assert_eq!(span, selected_span);
            for (whole, selected) in [(then_branch, selected_then), (else_branch, selected_else)] {
                assert_eq!(whole.final_output_values[position].position, position);
                assert_eq!(
                    whole.final_output_values[position].ty,
                    selected.final_output_values[0].ty
                );
                assert_eq!(
                    whole.final_output_values[position].span,
                    selected.final_output_values[0].span
                );
                assert_eq!(whole.items.len() - 2, selected.items.len() - 1);
                assert!(group
                    .blocks
                    .iter()
                    .any(|row| row.selected == *selected.occurrence
                        && row.source == *whole.occurrence));
            }
        }
        assert_eq!(then_branch.final_output_values[0].ty, ScalarType::I32);
        assert_eq!(then_branch.final_output_values[1].ty, ScalarType::Bool);
        for row in &group.blocks {
            assert_eq!(row.selected.source_span.source, source);
            assert_eq!(row.selected.source_span, row.source.source_span);
            assert_eq!(
                &row.selected.output_projection[1..],
                row.source.output_projection.as_slice()
            );
            assert!(row.selected.output_projection[0] < 2);
        }
        let mut roundtrip = block.clone();
        roundtrip.final_output_values = vec![ScalarOutputValue {
            position: 0,
            ty: ScalarType::Error,
            span: *span,
            value: group.expression.clone(),
        }];
        roundtrip
            .items
            .truncate(roundtrip.items.len() - block.final_output_values.len());
        roundtrip
            .items
            .push(ScalarBlockItem::Expression(group.expression.clone()));
        roundtrip
            .terminated_items
            .truncate(roundtrip.items.len() - 1);
        roundtrip.terminated_items.push(false);
        synchronize_final_items(&mut roundtrip);
        let outputs = block
            .final_output_values
            .iter()
            .map(|output| ScalarOutput {
                ty: output.ty.clone(),
                span: output.span,
            })
            .collect::<Vec<_>>();
        reference::expand_conditional_final_outputs(&mut roundtrip, &outputs);
        assert_eq!(roundtrip.items, block.items);
        assert_eq!(roundtrip.expressions, block.expressions);
        assert_eq!(roundtrip.terminated_items, block.terminated_items);
        assert_eq!(roundtrip.occurrence, block.occurrence);
        for (actual, expected) in roundtrip
            .final_output_values
            .iter()
            .zip(&block.final_output_values)
        {
            assert_eq!(actual.position, expected.position);
            assert_eq!(actual.span, expected.span);
            assert_eq!(actual.value, expected.value);
        }
        if family == "nested" {
            assert!(group
                .blocks
                .iter()
                .any(|row| row.selected.output_projection == [0, 0]
                    && row.source.output_projection == [0]));
            assert!(group
                .blocks
                .iter()
                .any(|row| row.selected.output_projection == [1, 1]
                    && row.source.output_projection == [1]));
            let nested = group_conditional_final_outputs(&source, then_branch)
                .expect("same nested authority")
                .expect("nested projected group");
            assert!(nested
                .blocks
                .iter()
                .all(|row| row.source.output_projection.is_empty()));
        } else {
            let prefix = &then_branch.items[..then_branch.items.len() - 2];
            assert_eq!(
                prefix
                    .iter()
                    .filter(|item| matches!(item, ScalarBlockItem::While(_)))
                    .count(),
                1
            );
            assert_eq!(
                prefix
                    .iter()
                    .filter(|item| matches!(item, ScalarBlockItem::Assignment(_)))
                    .count(),
                1
            );
            assert_eq!(prefix.iter().filter(|item| matches!(item, ScalarBlockItem::LocalBinding(binding) if binding.name == "value")).count(), 1);
            assert!(group
                .blocks
                .iter()
                .any(|row| row.source.output_projection.is_empty()
                    && row.source.source_span.range != then_branch.span
                    && row.source.source_span.range != else_branch.span));
        }
        let mut receiving_types = block.clone();
        receiving_types.final_output_values[0].ty = ScalarType::U64;
        assert_eq!(
            group_conditional_final_outputs(&source, &receiving_types)
                .unwrap()
                .unwrap(),
            group
        );

        let mut missing = block.clone();
        let ScalarExpression::If { then_branch, .. } = &mut missing.final_output_values[1].value
        else {
            unreachable!()
        };
        then_branch.occurrence.output_projection.clear();
        synchronize_final_items(&mut missing);
        negative(&source, &missing);
        let mut missing_output = block.clone();
        missing_output.final_output_values.pop();
        missing_output.items.pop();
        missing_output.terminated_items.pop();
        synchronize_final_items(&mut missing_output);
        negative(&source, &missing_output);
        let mut duplicate = block.clone();
        duplicate.final_output_values[1] = duplicate.final_output_values[0].clone();
        synchronize_final_items(&mut duplicate);
        negative(&source, &duplicate);
        let mut foreign = block.clone();
        let ScalarExpression::If { else_branch, .. } = &mut foreign.final_output_values[1].value
        else {
            unreachable!()
        };
        else_branch.occurrence.source_span.source = SourceIdentity::new(
            "foreign".into(),
            "app".into(),
            "src/other.w".into(),
            "r1".into(),
        );
        synchronize_final_items(&mut foreign);
        negative(&source, &foreign);
        let mut conflicting = block.clone();
        let ScalarExpression::If { then_branch, .. } =
            &mut conflicting.final_output_values[1].value
        else {
            unreachable!()
        };
        then_branch.unsafe_context = !then_branch.unsafe_context;
        synchronize_final_items(&mut conflicting);
        negative(&source, &conflicting);
        if family == "effects" {
            let mut prefix = block.clone();
            let ScalarExpression::If { then_branch, .. } = &mut prefix.final_output_values[1].value
            else {
                unreachable!()
            };
            then_branch.terminated_items[0] = !then_branch.terminated_items[0];
            synchronize_final_items(&mut prefix);
            negative(&source, &prefix);
        }
    }
    for (family, text) in [("independent", INDEPENDENT), ("single", SINGLE)] {
        let (source, block) = parsed_body(text, family, layout, project);
        assert_eq!(
            group_conditional_final_outputs(&source, &block).unwrap(),
            None
        );
    }
}

#[test]
fn conditional_output_group_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn conditional_output_group_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}

#[test]
fn conditional_output_group_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn conditional_output_group_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
