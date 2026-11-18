use super::*;

fn assert_roles(block: &ScalarBlock) {
    let roles = block.items_with_output_roles().collect::<Vec<_>>();
    assert_eq!(roles.len(), 3);
    for (position, (item, role)) in roles.into_iter().enumerate() {
        assert!(std::ptr::eq(item, &block.items[position]));
        let ScalarBlockItem::Expression(expression) = item else {
            panic!("canonical expression item")
        };
        match role {
            ScalarBlockItemRole::Ordinary => assert_eq!(position, 0),
            ScalarBlockItemRole::FinalOutput { output, metadata } => {
                assert_eq!(output, position - 1);
                assert!(std::ptr::eq(metadata, &block.final_output_values[output]));
                assert!(!std::ptr::eq(expression, &metadata.value));
                assert_eq!(metadata.position, output);
                assert_eq!(metadata.ty, ScalarType::U8);
            }
        }
    }
}

#[test]
fn block_output_roles_borrow_actual_suffix_items_and_preserve_return_point_order() {
    let text = "%%start\nunit(u8) tick = fn(input) { input; };\nu8(u8) first = fn(input) { input };\nu8(u8) second = fn(input) { input };\n(u8, u8)(u8) sequence = fn(input) { tick(input); first(input), second(input) };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let validation = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let program = validation.program;
        let ScalarItem::Function(function) = &program.items[3] else {
            panic!("sequence")
        };
        assert_roles(&function.body);
        let expected = [0, 1, 2].map(|index| {
            let ScalarItem::Function(function) = &program.items[index] else {
                panic!("callee")
            };
            ResolvedCallTarget::ModuleCallable {
                source: program.source.clone(),
                declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Declaration,
            }
        });
        let mut structural_probe = function.body.clone();
        let shared = expression_span(&structural_probe.final_output_values[0].value);
        for item in structural_probe.items.iter_mut().skip(1) {
            let ScalarBlockItem::Expression(ScalarExpression::Call { span, .. }) = item else {
                panic!("final call")
            };
            *span = shared;
        }
        for metadata in &mut structural_probe.final_output_values {
            let ScalarExpression::Call { span, .. } = &mut metadata.value else {
                panic!("final output projection")
            };
            *span = shared;
        }
        assert_roles(&structural_probe);
        let roots = structural_probe
            .items_with_output_roles()
            .filter_map(|(item, role)| match (item, role) {
                (
                    ScalarBlockItem::Expression(expression),
                    ScalarBlockItemRole::FinalOutput { output, metadata },
                ) => Some((expression, output, metadata)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 2);
        assert_eq!(expression_span(roots[0].0), expression_span(roots[1].0));
        assert!(!std::ptr::eq(roots[0].0, roots[1].0));
        assert_eq!((roots[0].1, roots[1].1), (0, 1));
        let before = program.clone();
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let calls = resolve_program_call_sites(&program).unwrap();
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 1);
        drop(stages);
        drop(scope);
        let sites = &calls.functions[&3].sites;
        assert_eq!(sites.len(), 3);
        for (position, (site, target)) in sites.iter().zip(&expected).enumerate() {
            assert_eq!(&site.target, target);
            let ScalarBlockItem::Expression(expression) = &function.body.items[position] else {
                unreachable!()
            };
            assert_eq!(
                site.source_span,
                SourceSpan::new(source(), expression_span(expression))
            );
        }
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let modules = [module];
        let project_calls = resolve_project_call_sites(&modules);
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let standalone = analyze_program_references(&program, &calls);
        let project = analyze_project_references(&modules, &project_calls);
        assert!(standalone.errors.is_empty(), "{:?}", standalone.errors);
        assert!(project.errors.is_empty(), "{:?}", project.errors);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 0);
        drop(stages);
        drop(scope);
        for results in [&standalone.updates, &project.updates[0].1] {
            let (_, result) = results.iter().find(|(index, _)| *index == 3).unwrap();
            let evaluated = result
                .cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                .collect::<Vec<_>>();
            assert_eq!(evaluated.len(), 3);
            for (position, (point, target)) in evaluated.iter().zip(&expected).enumerate() {
                let CfgPointKind::Call {
                    output,
                    target: actual,
                    ..
                } = &point.kind
                else {
                    unreachable!()
                };
                assert_eq!(*output, 0);
                assert_eq!(actual, target);
                assert_eq!(point.source_span, sites[position].source_span);
            }
            let returns = result
                .cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }))
                .collect::<Vec<_>>();
            assert_eq!(returns.len(), 2);
            for (position, point) in returns.iter().enumerate() {
                let CfgPointKind::ReturnOutput {
                    output,
                    value_point,
                    checked,
                } = &point.kind
                else {
                    unreachable!()
                };
                assert_eq!(*output, position);
                assert_eq!(*value_point, evaluated[position + 1].id);
                assert!(!checked);
                assert_eq!(
                    point.source_span,
                    SourceSpan::new(source(), function.body.final_output_values[position].span)
                );
                assert_eq!(point.scope, ReferenceScopeId(0));
            }
            assert!(evaluated[0].id.0 < evaluated[1].id.0 && evaluated[1].id.0 < evaluated[2].id.0);
            assert!(evaluated[2].id.0 < returns[0].id.0 && returns[0].id.0 < returns[1].id.0);
        }
        assert_eq!(program, before);
    }
}
