use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct copy Pair { u8 left; u8 right; }\nstruct copy Packet { Pair pair; u8[2] values; }\nPacket() build = fn { Packet stored = { .pair = { .left = 1; .right = 2; }; .values = [3, 4]; }; stored; Pair target = { .left = 5; .right = 6; }; target = { .left = 7; .right = 8; }; target; { .pair = { .left = 9; .right = 10; }; .values = [11, 12]; } };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "aggregate-root-registration-proof".into(),
        "app".into(),
        "src/roots.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let (items, structs) = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
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
        (validation.program.items, validation.program.structs)
    };
    let functions = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [function] = functions.as_slice() else {
        panic!("single actual function");
    };
    let cfg = &function.reference_cfg;
    let before = cfg.clone();
    let mut roots = Vec::new();
    let mut member_count = 0;
    let mut nested = Vec::new();
    for event in &cfg.initial_points {
        let CfgPointKind::Evaluate(expression) = &event.kind else {
            continue;
        };
        let children = match expression {
            ScalarExpression::StructLiteral { fields, .. } => {
                fields.iter().map(|field| &field.value).collect::<Vec<_>>()
            }
            ScalarExpression::ArrayLiteral { elements, .. } => elements.iter().collect(),
            _ => continue,
        };
        let values = cfg
            .evaluated_values
            .values
            .iter()
            .filter(|row| {
                row.id.resolved.occurrence == event.source_span && row.id.resolved.output == 0
            })
            .collect::<Vec<_>>();
        let [root] = values.as_slice() else {
            panic!("unique actual literal root: {values:#?}");
        };
        assert_eq!(root.id.point, event.id);
        assert_eq!(root.id.resolved.producer, cfg.producer_owner);
        assert!(root.id.resolved.output_projection.is_empty());
        assert_eq!(cfg.initial_points[root.id.point.0], *event);
        assert_eq!(cfg.initial_points.iter().filter(|row| {
            row.source_span == event.source_span
                && matches!(&row.kind, CfgPointKind::Evaluate(actual) if actual == expression)
        }).count(), 1);
        let root_shape = shape(cfg, &root.shape, &event.source_span).expect("actual root shape");
        match (expression, &root_shape.payload) {
            (
                ScalarExpression::StructLiteral { fields, .. },
                ValuePayloadShape::Struct { fields: shapes, .. },
            ) => {
                assert_eq!(fields.len(), shapes.len());
            }
            (
                ScalarExpression::ArrayLiteral { elements, .. },
                ValuePayloadShape::FixedArray { length, .. },
            ) => {
                assert_eq!(elements.len() as u64, *length);
            }
            _ => panic!("actual literal payload shape"),
        }
        let members = literal_aggregate_members(cfg, &structs, &root.id)
            .expect("actual registered root members");
        assert_eq!(members.len(), children.len());
        member_count += members.len();
        for (member, child) in members.iter().zip(children) {
            assert_eq!(
                member.value.resolved.occurrence,
                SourceSpan::new(event.source_span.source.clone(), expression_span(child))
            );
            let child_fact = fact(cfg, &member.value).expect("actual unique child fact");
            assert_eq!(child_fact.id, member.value);
            if matches!(
                child,
                ScalarExpression::StructLiteral { .. } | ScalarExpression::ArrayLiteral { .. }
            ) {
                assert!(matches!(&cfg.initial_points[member.value.point.0].kind,
                    CfgPointKind::Evaluate(actual) if actual == child));
                nested.push(member.value.clone());
            }
        }
        roots.push(root.id.clone());
    }
    assert_eq!((roots.len(), member_count, nested.len()), (8, 16, 4));
    for member in &nested {
        assert_eq!(roots.iter().filter(|root| *root == member).count(), 1);
    }
    let mut binding_roots = 0;
    for binding in &cfg.binding_points {
        if binding.role != ScalarBindingPointRole::Bind {
            continue;
        }
        let ScalarBindingOutputSource::Direct { occurrence, output_projection, output } = &binding.source else {
            continue;
        };
        let matches = roots
            .iter()
            .filter(|root| root.resolved.occurrence == *occurrence)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            continue;
        }
        let [root] = matches.as_slice() else {
            panic!("single actual binding root");
        };
        assert_eq!(*output, 0);
        assert_eq!(*output_projection, root.resolved.output_projection);
        let inputs = cfg.evaluated_values.binding_inputs.iter().filter(|input| input.consumer == binding.point).collect::<Vec<_>>();
        let [input] = inputs.as_slice() else { panic!("actual Direct aggregate singleton"); };
        assert_eq!(input.source, **root);
        assert_eq!(input.producer, cfg.producer_owner);
        assert_eq!(input.occurrence, binding.occurrence);
        assert_eq!(input.output, binding.output);
        assert!(matches!(
            &cfg.initial_points[binding.point.0].kind,
            CfgPointKind::Bind { .. }
        ));
        assert_eq!(
            cfg.initial_points
                .iter()
                .filter(|point| {
                    point.id == root.point && point.successors.contains(&binding.point)
                })
                .count(),
            1
        );
        binding_roots += 1;
    }
    let mut rhs_roots = 0;
    for evaluation in &cfg.evaluated_values.assignment_evaluations {
        for rhs in &evaluation.rhs {
            let matches = roots
                .iter()
                .filter(|root| **root == rhs.value)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let [root] = matches.as_slice() else {
                panic!("unique reused RHS root");
            };
            assert_eq!(rhs.value, **root);
            assert_eq!((rhs.expression_ordinal, rhs.producer_output), (0, 0));
            rhs_roots += 1;
        }
    }
    let mut final_roots = 0;
    for input in &cfg.evaluated_values.final_inputs {
        let matches = roots
            .iter()
            .filter(|root| **root == input.source)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            continue;
        }
        let [root] = matches.as_slice() else {
            panic!("unique reused final root");
        };
        assert_eq!(input.source, **root);
        assert_eq!(input.consumer.producer, cfg.producer_owner);
        assert!(matches!(&cfg.initial_points[input.consumer.point.0].kind,
            CfgPointKind::ReturnOutput { output: 0, value_point, .. } if *value_point == root.point));
        assert!(scalar_type_equal(
            &shape(
                cfg,
                &fact(cfg, root).expect("final root fact").shape,
                &root.resolved.occurrence
            )
            .expect("final root shape")
            .ty,
            &input.receiving_type
        ));
        final_roots += 1;
    }
    assert_eq!((binding_roots, rhs_roots, final_roots), (2, 1, 1));
    assert_eq!(cfg, &before);
}

#[test]
fn aggregate_roots_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn aggregate_roots_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn aggregate_roots_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn aggregate_roots_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
