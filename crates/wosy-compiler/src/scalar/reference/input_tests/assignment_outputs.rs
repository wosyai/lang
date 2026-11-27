use super::*;

#[test]
fn assignment_rhs_view_borrows_independent_roots_and_reuses_ordered_call_projections() {
    let text = "%%start\nbool(bool) echo = fn(flag) { flag };\nu8(u8) number = fn(value) { value };\n(u8, bool, u8)(bool) triple = fn(flag) { u8 small = 1; u8 large = 2; small, flag, large };\nunit(bool) probe = fn(flag) { u8 first = 0; bool valid = false; u8 last = 0; first; valid; last; first, valid, last = triple(echo(flag)); first; valid; last; first, last = first, number(last); first; last; };\n%%end";
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
        let ScalarItem::Function(probe) = &program.items[3] else {
            panic!("probe")
        };
        let assignments = probe
            .body
            .items
            .iter()
            .filter_map(|item| match item {
                ScalarBlockItem::Assignment(assignment) => Some(assignment),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(assignments.len(), 2);
        let projected = assignments[0];
        let rhs = projected.rhs();
        let roots = rhs.evaluations().collect::<Vec<_>>();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].0, 0);
        assert!(std::ptr::eq(roots[0].1, &projected.values[0]));
        assert_eq!(rhs.projected_outputs().collect::<Vec<_>>(), vec![1, 2]);
        let independent = assignments[1];
        let rhs = independent.rhs();
        let roots = rhs.evaluations().collect::<Vec<_>>();
        assert_eq!(roots.len(), 2);
        assert!(matches!(roots[1].1, ScalarExpression::Call { .. }));
        for (position, value) in roots {
            assert!(std::ptr::eq(value, &independent.values[position]));
        }
        assert!(rhs.projected_outputs().next().is_none());
        let mut structural_probe = independent.clone();
        let shared = expression_span(&structural_probe.values[0]);
        let ScalarExpression::Call { span, .. } = &mut structural_probe.values[1] else {
            panic!("independent call")
        };
        *span = shared;
        let rhs = structural_probe.rhs();
        let roots = rhs.evaluations().collect::<Vec<_>>();
        assert_eq!((roots[0].0, roots[1].0), (0, 1));
        assert_eq!(expression_span(roots[0].1), expression_span(roots[1].1));
        assert!(!std::ptr::eq(roots[0].1, roots[1].1));
        for (position, value) in roots {
            assert!(std::ptr::eq(value, &structural_probe.values[position]));
        }
        assert!(rhs.projected_outputs().next().is_none());
        let before = program.clone();
        let module = ScalarModule::from_program(program.clone(), Vec::new());
        let modules = [module];
        let calls = resolve_program_call_sites(&program).unwrap();
        let project_calls = resolve_project_call_sites(&modules);
        assert_eq!(calls.functions[&3].sites.len(), 3);
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
            let (_, probe) = results.iter().find(|(index, _)| *index == 3).unwrap();
            let cfg = &probe.cfg;
            let call_points = cfg
                .points
                .iter()
                .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
                .collect::<Vec<_>>();
            assert_eq!(call_points.len(), 5);
            let CfgPointKind::Call {
                output: 0,
                target: selected,
                argument_points,
                ..
            } = &call_points[1].kind
            else {
                panic!("primary triple call")
            };
            let ScalarItem::Function(triple) = &program.items[2] else {
                unreachable!()
            };
            assert_eq!(
                selected,
                &ResolvedCallTarget::ModuleCallable {
                    source: source(),
                    declaration_span: triple.name_span,
                    concrete: ConcreteCallSelection::Declaration
                }
            );
            assert_eq!(argument_points, &[call_points[0].id]);
            for (position, point) in call_points[1..4].iter().enumerate() {
                let CfgPointKind::Call {
                    output,
                    target,
                    argument_points: arguments,
                    ..
                } = &point.kind
                else {
                    unreachable!()
                };
                assert_eq!(*output, position);
                assert_eq!(target, selected);
                assert_eq!(arguments, argument_points);
                assert_eq!(point.source_span, call_points[1].source_span);
            }
            let writes = cfg
                .points
                .iter()
                .filter_map(|point| match &point.kind {
                    CfgPointKind::Assign { rhs_point, .. } => Some((point.id, *rhs_point)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(writes.len(), 5);
            assert_eq!(
                writes
                    .iter()
                    .take(3)
                    .map(|(_, rhs)| *rhs)
                    .collect::<Vec<_>>(),
                call_points[1..4]
                    .iter()
                    .map(|point| point.id)
                    .collect::<Vec<_>>()
            );
            assert_eq!(writes[4].1, call_points[4].id);
            let captures = cfg
                .points
                .iter()
                .filter_map(|point| match &point.kind {
                    CfgPointKind::CaptureAssignmentPrevious { writes } => Some((point.id, writes)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(captures.len(), 2);
            assert_eq!(
                captures[0].1,
                &writes[..3].iter().map(|(id, _)| *id).collect::<Vec<_>>()
            );
            assert_eq!(
                captures[1].1,
                &writes[3..].iter().map(|(id, _)| *id).collect::<Vec<_>>()
            );
            assert!(call_points[3].id.0 < captures[0].0 .0 && captures[0].0 .0 < writes[0].0 .0);
            assert!(call_points[4].id.0 < captures[1].0 .0 && captures[1].0 .0 < writes[3].0 .0);
            assert!(writes.windows(2).all(|pair| pair[0].0 .0 < pair[1].0 .0));
        }
        assert_eq!(program, before);
    }
}
