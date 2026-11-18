use super::*;

#[test]
fn binding_output_sources_borrow_canonical_roots_and_project_precise_primary_ordinals() {
    let text = "%%start\nu8(u8) first = fn(input) { input };\nu8(u8) second = fn(input) { input };\n(u8, u8, u8)(u8) triple = fn(input) { input, input, input };\nunit(u8) probe = fn(input) { u8 left, u8 right = first(input) + input, second(input) + input; left; right; u8 a, u8 b, u8 c = triple(input); a; b; c; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let validation = derive_scalar_program_with_layout(&parsed.result, target);
        assert!(
            validation.diagnostics.is_empty(),
            "{:?}",
            validation.diagnostics
        );
        let mut program = validation.program;
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
        let ScalarItem::Function(probe) = &mut program.items[3] else {
            panic!("probe")
        };
        let mut bindings = probe.body.items.iter_mut().filter_map(|item| match item {
            ScalarBlockItem::LocalBinding(binding) => Some(binding),
            _ => None,
        });
        let independent = bindings.next().unwrap();
        assert_eq!(
            independent.output_origin,
            ScalarBindingOutputOrigin::IndependentExpressions
        );
        assert_eq!(independent.output_values.len(), 2);
        assert_eq!(independent.receivers.len(), 2);
        let shared = expression_span(&independent.value);
        let ScalarExpression::Binary { span, .. } = &mut independent.output_values[1].value else {
            panic!("second independent root")
        };
        *span = shared;
        for position in 0..2 {
            let ScalarBindingOutput::Evaluate(value) = independent.output_source(position) else {
                panic!("independent evaluation")
            };
            let canonical = if position == 0 {
                &independent.value
            } else {
                &independent.output_values[position].value
            };
            assert!(std::ptr::eq(value, canonical));
            assert_eq!(expression_span(value), shared);
        }
        let ScalarBindingOutput::Evaluate(primary) = independent.output_source(0) else {
            unreachable!()
        };
        let ScalarBindingOutput::Evaluate(secondary) = independent.output_source(1) else {
            unreachable!()
        };
        assert!(!std::ptr::eq(primary, secondary));
        assert!(!std::ptr::eq(primary, &independent.output_values[0].value));
        let single = bindings.next().unwrap();
        assert_eq!(
            single.output_origin,
            ScalarBindingOutputOrigin::SingleExpression
        );
        assert_eq!(single.receivers.len(), 3);
        let ScalarBindingOutput::Evaluate(primary) = single.output_source(0) else {
            panic!("primary evaluation")
        };
        assert!(std::ptr::eq(primary, &single.value));
        for position in 1..3 {
            let ScalarBindingOutput::ProjectPrimary { output } = single.output_source(position)
            else {
                panic!("primary projection")
            };
            assert_eq!(output, position);
        }
        assert!(bindings.next().is_none());
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
        for (site, expected) in sites.iter().zip(&expected) {
            assert_eq!(&site.target, expected);
        }
        let scope = layout::observations::Scope::new();
        let stages = stage_observations::Scope::new();
        let result = analyze_program_references(&program, &calls);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(scope.counts(), layout::observations::Counts::default());
        assert_eq!(stages.counts().call_sites, 0);
        drop(stages);
        drop(scope);
        let (_, probe) = result
            .updates
            .iter()
            .find(|(index, _)| *index == 3)
            .unwrap();
        let calls = probe
            .cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Call { output, target, .. } => Some((*output, target)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0], (0, &expected[0]));
        assert_eq!(calls[1], (0, &expected[1]));
        for (position, call) in calls[2..].iter().enumerate() {
            assert_eq!(*call, (position, &expected[2]));
        }
        assert_eq!(program, before);
    }
}
