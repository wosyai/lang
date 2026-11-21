use super::*;

#[test]
fn binding_and_assignment_projections_retain_each_nested_call_producer_once() {
    let text = "%%start\nbool(bool) echo = fn(flag) { flag };\n(u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };\nbool(bool) probe = fn(flag) { u8 magnitude, bool valid = pair(echo(flag)); magnitude; valid; magnitude, valid = pair(echo(flag)); magnitude; valid; u8 another, bool ready = pair(echo(flag)); another; ready };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let single = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let ScalarSourceDeclarations {
            source,
            items,
            structs,
            enums,
            diagnostics,
        } = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        assert!(diagnostics.is_empty());
        let order = vec![source.clone()];
        let mut module = ScalarModule::new(source, items, Vec::new());
        module.structs = structs;
        module.enums = enums;
        module.target_layout = target;
        let project = validate_scalar_project(ScalarProject::new(vec![module], order));
        assert_eq!(project.diagnostics, single.diagnostics);
        assert_eq!(project.project.modules[0].items, single.program.items);
        let ScalarItem::Function(echo) = &single.program.items[0] else {
            panic!("echo")
        };
        let ScalarItem::Function(pair) = &single.program.items[1] else {
            panic!("pair")
        };
        let ScalarItem::Function(probe) = &single.program.items[2] else {
            panic!("probe")
        };
        let ScalarItem::Function(project_probe) = &project.project.modules[0].items[2] else {
            panic!("project probe")
        };
        let mut prior_arguments = Vec::new();
        for cfg in [&probe.reference_cfg, &project_probe.reference_cfg] {
            prior_arguments.clear();
            let calls = cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 9);
            for (ordinal, group) in calls.chunks_exact(3).enumerate() {
                let argument = group[0];
                let producer = group[1];
                let projection = group[2];
                let CfgPointKind::Call {
                    expression: argument_expression,
                    output: 0,
                    target: argument_target,
                    argument_points: nested_arguments,
                } = &argument.kind
                else {
                    panic!("argument call")
                };
                assert_eq!(
                    argument_target,
                    &ResolvedCallTarget::ModuleCallable {
                        source: single.program.source.clone(),
                        declaration_span: echo.name_span,
                        concrete: ConcreteCallSelection::Declaration
                    }
                );
                assert_eq!(nested_arguments.len(), 1);
                assert!(nested_arguments[0].0 < argument.id.0);
                assert_eq!(cfg.points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Call { expression, output: 0, .. } if expression == argument_expression)).count(), 1);
                let CfgPointKind::Call {
                    expression,
                    output: 0,
                    target,
                    argument_points,
                } = &producer.kind
                else {
                    panic!("producer")
                };
                assert_eq!(
                    target,
                    &ResolvedCallTarget::ModuleCallable {
                        source: single.program.source.clone(),
                        declaration_span: pair.name_span,
                        concrete: ConcreteCallSelection::Declaration
                    }
                );
                assert_eq!(argument_points, &[argument.id]);
                assert!(!prior_arguments.contains(&argument.id));
                prior_arguments.push(argument.id);
                assert_eq!(
                    projection.kind,
                    CfgPointKind::Call {
                        expression: expression.clone(),
                        output: 1,
                        target: target.clone(),
                        argument_points: argument_points.clone()
                    }
                );
                assert_eq!(projection.source_span, producer.source_span);
                assert_eq!(projection.scope, producer.scope);
                assert_eq!(
                    argument.source_span,
                    SourceSpan::new(
                        single.program.source.clone(),
                        expression_span(argument_expression)
                    )
                );
                assert_eq!(
                    producer.source_span,
                    SourceSpan::new(single.program.source.clone(), expression_span(expression))
                );
                assert!(argument.id.0 < producer.id.0);
                assert!(producer.id.0 < projection.id.0);
                let outer_start = text
                    .match_indices("pair(echo(flag))")
                    .nth(ordinal)
                    .unwrap()
                    .0 as u32;
                assert_eq!(
                    producer.source_span.range,
                    ByteSpan::new(outer_start, outer_start + "pair(echo(flag))".len() as u32)
                );
                assert_eq!(
                    argument.source_span.range,
                    ByteSpan::new(outer_start + 5, outer_start + 5 + "echo(flag)".len() as u32)
                );
                if ordinal == 1 {
                    let writes = cfg
                        .points
                        .iter()
                        .filter_map(|point| match &point.kind {
                            CfgPointKind::Assign { rhs_point, .. }
                                if *rhs_point == producer.id || *rhs_point == projection.id =>
                            {
                                Some((point.id, *rhs_point))
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(writes.len(), 2);
                    assert_eq!(writes[0].1, producer.id);
                    assert_eq!(writes[1].1, projection.id);
                    assert!(projection.id.0 < writes[0].0 .0);
                    assert!(writes[0].0 .0 < writes[1].0 .0);
                } else {
                    let names = if ordinal == 0 {
                        ["magnitude", "valid"]
                    } else {
                        ["another", "ready"]
                    };
                    let installs = cfg
                        .points
                        .iter()
                        .filter_map(|point| match &point.kind {
                            CfgPointKind::Bind { binding } => {
                                let name = &text[binding.declaration_span.start as usize
                                    ..binding.declaration_span.end as usize];
                                names
                                    .iter()
                                    .position(|expected| *expected == name)
                                    .map(|position| (position, point.id))
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(installs.len(), 2);
                    assert_eq!([installs[0].0, installs[1].0], [0, 1]);
                    assert!(producer.id.0 < installs[0].1 .0);
                    assert!(installs[0].1 .0 < projection.id.0);
                    assert!(projection.id.0 < installs[1].1 .0);
                }
            }
        }
    }
}
