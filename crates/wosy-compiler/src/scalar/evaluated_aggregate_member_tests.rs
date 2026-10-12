use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct copy Reader { u8() read; }\nstruct copy Bundle { Reader first; Reader second; (u8())[2] items; }\n(u8())() make = fn { fn { 3 } };\nBundle() build = fn { u8() current = fn { 1 }; { .second = { .read = make(); }; .items = [fn { 4 }, fn { 5 }]; .first = { .read = (current = fn { 2 }); }; } };\n%%end";

fn program(layout: ScalarTargetLayout) -> ScalarProgram {
    let source = SourceIdentity::new(
        "literal-member-constructor-proof".into(),
        "app".into(),
        "src/members.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source, SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    validation.program
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let program = program(layout);
    let (items, structs) = if project {
        let source = program.source.clone();
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        let module = validation
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual module");
        (module.items, module.structs)
    } else {
        (program.items, program.structs)
    };
    let mut aggregate_count = 0;
    let mut member_count = 0;
    let mut assignments = 0;
    let mut calls = 0;
    let mut captures = 0;
    let mut nested = 0;
    for item in &items {
        let ScalarItem::Function(function) = item else {
            continue;
        };
        let cfg = &function.reference_cfg;
        let before = cfg.clone();
        for root in &cfg.evaluated_values.values {
            let actual = &cfg.initial_points[root.id.point.0];
            let CfgPointKind::Evaluate(expression) = &actual.kind else {
                continue;
            };
            let children = match expression {
                ScalarExpression::StructLiteral { fields, .. } => {
                    fields.iter().map(|field| &field.value).collect::<Vec<_>>()
                }
                ScalarExpression::ArrayLiteral { elements, .. } => elements.iter().collect(),
                _ => continue,
            };
            let members = literal_aggregate_members(cfg, &structs, &root.id)
                .expect("actual whole literal members");
            assert_eq!(members.len(), children.len());
            aggregate_count += 1;
            member_count += members.len();
            let root_shape = cfg
                .evaluated_values
                .shapes
                .iter()
                .find(|row| row.id == root.shape)
                .expect("root shape");
            for (ordinal, (member, child)) in members.iter().zip(children).enumerate() {
                let source =
                    SourceSpan::new(actual.source_span.source.clone(), expression_span(child));
                assert_eq!(member.value.resolved.occurrence, source);
                assert_eq!(member.value.resolved.producer, cfg.producer_owner);
                assert_eq!(member.value.resolved.output, 0);
                let matches = cfg
                    .evaluated_values
                    .values
                    .iter()
                    .filter(|row| row.id == member.value)
                    .collect::<Vec<_>>();
                let [fact] = matches.as_slice() else {
                    panic!("unique actual member fact");
                };
                match (&root_shape.payload, expression) {
                    (
                        ValuePayloadShape::Struct { structure, fields },
                        ScalarExpression::StructLiteral {
                            fields: literal, ..
                        },
                    ) => {
                        let declaration = structs
                            .iter()
                            .find(|row| row.id == *structure)
                            .expect("actual struct");
                        let field = declaration
                            .fields
                            .iter()
                            .find(|row| row.name == literal[ordinal].name)
                            .expect("declared source field");
                        assert_eq!(
                            member.projection,
                            vec![ValueProjection::Field(field.id.clone())]
                        );
                        assert!(corresponding_shapes(
                            cfg,
                            &fact.shape,
                            &fields
                                .iter()
                                .find(|row| row.field == field.id)
                                .expect("declared field shape")
                                .shape,
                            &source,
                            &mut Vec::new()
                        )
                        .expect("actual declared shape graph"));
                    }
                    (
                        ValuePayloadShape::FixedArray { element, length },
                        ScalarExpression::ArrayLiteral { elements, .. },
                    ) => {
                        assert_eq!(*length, elements.len() as u64);
                        assert_eq!(
                            member.projection,
                            vec![ValueProjection::FixedIndex(ordinal as u64)]
                        );
                        assert!(corresponding_shapes(
                            cfg,
                            &fact.shape,
                            element,
                            &source,
                            &mut Vec::new()
                        )
                        .expect("actual element shape graph"));
                    }
                    _ => panic!("literal root shape"),
                }
                let production = &cfg.initial_points[member.value.point.0];
                assert_eq!(production.source_span, source);
                match child {
                    ScalarExpression::Assignment(assignment) => {
                        assignments += 1;
                        let evaluation = cfg
                            .evaluated_values
                            .assignment_evaluations
                            .iter()
                            .find(|row| row.operation.occurrence == *assignment.occurrence)
                            .expect("actual saved operation");
                        let [result] = evaluation.results.as_slice() else {
                            panic!("single result");
                        };
                        assert_eq!(result.value, member.value);
                        assert_eq!(
                            member.value.resolved.output_projection,
                            assignment.occurrence.output_projection
                        );
                        assert!(
                            matches!(production.kind, CfgPointKind::AssignmentResult { snapshot_point, output: 0 } if snapshot_point == result.snapshot)
                        );
                        assert_eq!(
                            cfg.evaluated_values
                                .values
                                .iter()
                                .filter(|row| row.id.resolved == member.value.resolved)
                                .count(),
                            2
                        );
                    }
                    ScalarExpression::Call { occurrence, .. } => {
                        calls += 1;
                        assert_eq!(
                            member.value.resolved.output_projection,
                            occurrence.output_projection
                        );
                        let registered = cfg
                            .evaluated_values
                            .productions
                            .iter()
                            .find(|row| {
                                row.occurrence == occurrence.source_span
                                    && row.output_projection == occurrence.output_projection
                            })
                            .expect("actual Call production");
                        assert_eq!(registered.outputs, vec![member.value.clone()]);
                        assert!(matches!(
                            production.kind,
                            CfgPointKind::Call { output: 0, .. }
                        ));
                    }
                    ScalarExpression::FunctionLiteral { .. } => {
                        captures += 1;
                        assert!(matches!(production.kind, CfgPointKind::Capture { .. }));
                        assert!(member.value.resolved.output_projection.is_empty());
                    }
                    ScalarExpression::StructLiteral { .. }
                    | ScalarExpression::ArrayLiteral { .. } => {
                        nested += 1;
                        assert!(
                            matches!(&production.kind, CfgPointKind::Evaluate(actual) if actual == child)
                        );
                        assert!(!literal_aggregate_members(cfg, &structs, &member.value)
                            .expect("nested own direct members")
                            .is_empty());
                        assert_eq!(member.projection.len(), 1);
                    }
                    _ => panic!("actual proof member kind"),
                }
            }
        }
        assert_eq!(cfg, &before);
    }
    assert_eq!(
        (
            aggregate_count,
            member_count,
            assignments,
            calls,
            captures,
            nested
        ),
        (4, 7, 1, 1, 2, 3)
    );
}

fn negative(kind: usize) {
    for layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let program = program(layout);
        let function = program
            .items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "build" => Some(function),
                _ => None,
            })
            .expect("actual aggregate-producing function");
        let cfg = &function.reference_cfg;
        let root = cfg.evaluated_values.values.iter().find(|row| {
            matches!(&cfg.initial_points[row.id.point.0].kind, CfgPointKind::Evaluate(ScalarExpression::StructLiteral { fields, .. }) if fields.len() == 3)
        }).expect("actual Bundle root");
        let members = literal_aggregate_members(cfg, &program.structs, &root.id)
            .expect("actual source inputs");
        let child = &members[0].value;
        let mut malformed = cfg.clone();
        let child_index = malformed
            .evaluated_values
            .values
            .iter()
            .position(|row| row.id == *child)
            .expect("actual child index");
        match kind {
            0 => {
                let foreign = program
                    .items
                    .iter()
                    .find_map(|item| match item {
                        ScalarItem::Function(function)
                            if function.reference_cfg.producer_owner != cfg.producer_owner =>
                        {
                            Some(function.reference_cfg.producer_owner.clone())
                        }
                        _ => None,
                    })
                    .expect("actual foreign owner");
                malformed.evaluated_values.values[child_index]
                    .id
                    .resolved
                    .producer = foreign;
            }
            1 => {
                let duplicate = malformed.evaluated_values.values[child_index].clone();
                malformed.evaluated_values.values.push(duplicate);
            }
            2 => {
                malformed.evaluated_values.values[child_index].id.point = members[1].value.point;
            }
            3 => {
                let shape_index = malformed
                    .evaluated_values
                    .shapes
                    .iter()
                    .position(|row| row.id == root.shape)
                    .expect("actual root shape");
                let ValuePayloadShape::Struct { fields, .. } =
                    &mut malformed.evaluated_values.shapes[shape_index].payload
                else {
                    panic!("actual struct shape");
                };
                let ValueProjection::Field(field) = &members[0].projection[0] else {
                    panic!("actual field");
                };
                let registered = fields
                    .iter_mut()
                    .find(|row| row.field == *field)
                    .expect("actual field shape");
                registered.shape = malformed
                    .evaluated_values
                    .values
                    .iter()
                    .find(|row| row.id == members[1].value)
                    .expect("actual array member")
                    .shape
                    .clone();
            }
            _ => panic!("negative identity"),
        }
        let before = malformed.clone();
        assert_eq!(
            literal_aggregate_members(&malformed, &program.structs, &root.id),
            Err(invalid(&child.resolved.occurrence))
        );
        assert_eq!(malformed, before);
    }
}

#[test]
fn literal_members_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn literal_members_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn literal_members_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn literal_members_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
#[test]
fn literal_members_foreign_child() {
    negative(0);
}
#[test]
fn literal_members_duplicate_child() {
    negative(1);
}
#[test]
fn literal_members_wrong_child_point() {
    negative(2);
}
#[test]
fn literal_members_wrong_field_shape() {
    negative(3);
}
