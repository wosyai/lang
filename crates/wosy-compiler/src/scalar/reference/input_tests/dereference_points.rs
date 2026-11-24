use super::*;

#[test]
fn prepared_equal_span_dereference_places_retain_distinct_query_pointer_points() {
    let text = "%%start\nunit(*u8, *u8) probe = fn(left, right) { *left; *right; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let public =
            derive_scalar_program_from_cst_with_layout(parsed.result.canonical_cst(), target);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        let mut declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let ScalarItem::Function(function) = &mut declarations.items[0] else {
            panic!("probe")
        };
        let ScalarBlockItem::Expression(ScalarExpression::Dereference { place, .. }) =
            &function.body.items[0]
        else {
            panic!("first dereference")
        };
        let shared = scalar_place_target_span(place);
        let ScalarBlockItem::Expression(ScalarExpression::Dereference { place, .. }) =
            &mut function.body.items[1]
        else {
            panic!("second dereference")
        };
        let ScalarPlace::Dereference { span, .. } = place else {
            panic!("dereference place")
        };
        *span = shared;
        let typed = typecheck_scalar_program(declarations, target);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites(&program).unwrap();
        let ScalarItem::Function(function) = &program.items[0] else {
            unreachable!()
        };
        let row = calls.function(0);
        let root = OccurrenceParent::Block(row.structure.root);
        let module_names = HashSet::from(["probe".to_owned()]);
        let mut builder = ReferenceFlowBuilder::new(
            &program.source,
            function,
            &module_names,
            row,
            &program.structs,
            &program.enums,
        );
        let ScalarType::Callable { parameters, .. } = &function.signature else {
            unreachable!()
        };
        for ((name, span), ty) in function
            .parameters
            .iter()
            .zip(&function.parameter_spans)
            .zip(parameters)
        {
            let binding = ReferenceBindingId {
                source: program.source.clone(),
                function_span: function.span,
                declaration_span: *span,
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            };
            builder.names.insert(name.clone(), binding.clone());
            builder.binding_types.insert(binding.clone(), ty.clone());
            builder.checked.insert(binding);
        }
        let mut retained = Vec::new();
        for (item, canonical) in function.body.items.iter().enumerate() {
            let ScalarBlockItem::Expression(expression) = canonical else {
                panic!("canonical read")
            };
            let OccurrenceNode::Expression(ordinal) =
                row.structure.edges[&(root, OccurrenceEdge::ExpressionItem { item })]
            else {
                panic!("expression ordinal")
            };
            let OccurrenceNode::Place(place_ordinal) = row.structure.edges[&(
                OccurrenceParent::Expression(ordinal),
                OccurrenceEdge::ExpressionPlace,
            )] else {
                panic!("place ordinal")
            };
            assert_eq!(row.occurrences[place_ordinal.0].span, shared);
            builder
                .expression(expression, ReferenceScopeId(0), ordinal)
                .unwrap();
            let pointer = builder.dereference_points[&place_ordinal];
            let CfgPointKind::Read { binding } = &builder.cfg.points[pointer.0].kind else {
                panic!("retained pointer evaluation")
            };
            assert_eq!(binding.declaration_span, function.parameter_spans[item]);
            let read = builder.cfg.points.last().unwrap();
            let CfgPointKind::ReadPlace { pointer_point, .. } = &read.kind else {
                panic!("dereferenced read")
            };
            assert_eq!(*pointer_point, Some(pointer));
            retained.push((place_ordinal, pointer));
        }
        assert_eq!(retained.len(), 2);
        assert_ne!(retained[0].0, retained[1].0);
        assert_ne!(retained[0].1, retained[1].1);
        assert_eq!(builder.dereference_points.len(), 2);
        for (ordinal, pointer) in &retained {
            assert_eq!(builder.dereference_points[ordinal], *pointer);
        }
        let before = program.clone();
        let stages = stage_observations::Scope::new();
        let results = analyze_program_references(&program, &calls);
        assert_eq!(stages.counts().call_sites, 0);
        drop(stages);
        let mut installed = before.clone();
        let errors = results.install(&mut installed);
        assert!(errors.is_empty(), "{errors:?}");
        let ScalarItem::Function(probe) = &installed.items[0] else {
            unreachable!()
        };
        let reads = probe
            .reference_cfg
            .points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::ReadPlace {
                    pointer_point: Some(pointer),
                    place: ScalarPlace::Dereference { span, .. },
                    ..
                } => Some((*span, *pointer)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].0, shared);
        assert_eq!(reads[1].0, shared);
        assert_ne!(reads[0].1, reads[1].1);
        assert_eq!(program, before);
    }
}
