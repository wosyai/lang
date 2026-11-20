use super::*;

const TEXT: &str = "%%start
*u8(u8[2], u64) checked = fn(values, index) { &values[index] };
unit(u8[2], u64) raw = fn(values, index) { unsafe { &?values[index]; }; };
*u8(*u8) child = fn(pointer) { &*pointer };
%%end";

fn address(function: &mut ScalarFunction) -> &mut ScalarExpression {
    let ScalarBlockItem::Expression(expression) = &mut function.body.items[0] else {
        panic!("address expression")
    };
    match expression {
        ScalarExpression::Block(body) => {
            let ScalarBlockItem::Expression(address) = &mut body.items[0] else {
                panic!("raw address")
            };
            address
        }
        _ => expression,
    }
}

#[test]
fn callable_named_place_root_projection_preserves_scoped_module_and_missing_facts() {
    let text = "%%start
u8[3] outside = [1, 2, 3];
unit() probe = fn {
    unsafe { u8[3] scratch; &?scratch[0]; };
    unsafe { u8[3] scratch; &?scratch[0]; };
    unsafe { &?outside[0]; };
};
%%end";
    let source = source();
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_scalar_project(
            vec![(
                derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
                Vec::new(),
            )],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let items = &project.project().modules[0].items;
        let [ScalarItem::Binding(global), ScalarItem::Function(function)] = items.as_slice() else {
            panic!("module binding and callable");
        };
        let block = |item| match &function.body.items[item] {
            ScalarBlockItem::Expression(ScalarExpression::Block(block)) => block,
            _ => panic!("unsafe block"),
        };
        let place = |item, address| {
            let ScalarBlockItem::Expression(ScalarExpression::RawAddress { place, .. }) =
                &block(item).items[address]
            else {
                panic!("raw address");
            };
            let ScalarPlace::Index { base, .. } = place else {
                panic!("indexed place");
            };
            (place, base.as_ref())
        };
        let nodes = project
            .bind_callable_body_call_nodes(0, 1, None, &function.body)
            .unwrap();
        let expected_local = |item| {
            let block = block(item);
            let ScalarBlockItem::LocalBinding(allocation) = &block.items[0] else {
                panic!("scratch allocation");
            };
            assert!(allocation.is_allocation);
            NamedPlaceRootIdentity::Lexical(ReferenceBindingId {
                source: source.clone(),
                function_span: function.span,
                declaration_span: allocation.receivers[0].name_span,
                block_span: block.span,
                kind: ReferenceBindingKind::Declared,
            })
        };
        let (indexed, first) = place(0, 1);
        let (_, second) = place(1, 1);
        let (_, module) = place(2, 0);
        assert_ne!(expected_local(0), expected_local(1));
        assert_eq!(nodes.named_place_root(first), Some(&expected_local(0)));
        assert_eq!(nodes.named_place_root(second), Some(&expected_local(1)));
        assert_eq!(
            nodes.named_place_root(module),
            Some(&NamedPlaceRootIdentity::ModuleValue(SourceSpan::new(
                source.clone(),
                global.receivers[0].name_span,
            )))
        );
        assert_eq!(nodes.named_place_root(indexed), None);
        assert_eq!(nodes.named_place_root(&first.clone()), None);

        let mut row = project.function_calls(0, 1).clone();
        let NamedPlaceRootFacts::Prepared(roots) = &mut row.named_place_roots else {
            panic!("prepared place roots");
        };
        roots.retain(|_, identity| *identity != expected_local(0));
        let missing = control::CallableBodyCallNodes::bind(&source, &function.body, &row, None)
            .expect("a missing projection row is not a binding error");
        assert_eq!(missing.named_place_root(first), None);
        assert_eq!(missing.named_place_root(second), Some(&expected_local(1)));
        assert_eq!(
            missing.named_place_root(module),
            nodes.named_place_root(module)
        );
    }
}

#[test]
fn prepared_address_creation_uses_root_and_pointer_facts_on_both_routes_and_layouts() {
    let source = source();
    let parsed = parse_source(source.clone(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let module_names = HashSet::from(["checked".into(), "raw".into(), "child".into()]);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let single = static_calls::PreparedProgram::new(declarations(), target_layout)
            .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let project = prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let rows = [0, 1, 2].map(|item| single.function_calls(item).clone());
        let single = single.finish();
        assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
        let build = analyze_scalar_build(&project);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        let raw = resolve_program_call_sites(&single.program).unwrap();
        for (item, standalone_row) in rows.iter().enumerate() {
            let ScalarItem::Function(single_function) = &single.program.items[item] else {
                panic!("standalone callable")
            };
            let ScalarItem::Function(project_function) = &project.project().modules[0].items[item]
            else {
                panic!("project callable")
            };
            assert_eq!(
                build.reference_cfg(0, item, None),
                Some(&single_function.reference_cfg)
            );
            for (function, row) in [
                (single_function, standalone_row),
                (project_function, project.function_calls(0, item)),
            ] {
                let derive = |f: &ScalarFunction, row: &FunctionCallSites| {
                    derive_function_reference_cfg(&source, f, &module_names, row, &[], &[], true)
                };
                assert_eq!(derive(function, row), derive(function, raw.function(item)));
                let mut mutated = function.clone();
                let expression = address(&mut mutated);
                let (place, span) = match expression {
                    ScalarExpression::CheckedAddress { place, span, .. }
                    | ScalarExpression::RawAddress { place, span } => (place, *span),
                    _ => panic!("address"),
                };
                let address_ordinal = row
                    .occurrences
                    .iter()
                    .enumerate()
                    .find_map(|(index, meta)| {
                        let ordinal = OccurrenceOrdinal(index);
                        (meta.span == span
                            && row.structure.edges.contains_key(&(
                                OccurrenceParent::Expression(ordinal),
                                OccurrenceEdge::ExpressionPlace,
                            )))
                        .then_some(ordinal)
                    })
                    .expect("address ordinal");
                let OccurrenceNode::Place(place_ordinal) = row.structure.edges[&(
                    OccurrenceParent::Expression(address_ordinal),
                    OccurrenceEdge::ExpressionPlace,
                )] else {
                    panic!("address place")
                };
                let mut corrupt = row.clone();
                let (binding, missing_span) = if let ScalarPlace::Dereference { pointer, .. } =
                    place
                {
                    let ScalarExpression::Name { name, span } = pointer.as_mut() else {
                        panic!("pointer name")
                    };
                    *name = "decoy".into();
                    let OccurrenceNode::Expression(pointer_ordinal) = row.structure.edges[&(
                        OccurrenceParent::Place(place_ordinal),
                        OccurrenceEdge::DereferencePointer,
                    )] else {
                        panic!("pointer occurrence")
                    };
                    let PlainNameFacts::Prepared(names) = &mut corrupt.plain_names else {
                        unreachable!()
                    };
                    let Some(PlainNameIdentity::Lexical(binding)) = names.remove(&pointer_ordinal)
                    else {
                        panic!("resolved pointer")
                    };
                    (binding, *span)
                } else {
                    let ScalarPlace::Index { base, .. } = place else {
                        panic!("indexed address")
                    };
                    let ScalarPlace::Name { name, span } = base.as_mut() else {
                        panic!("named root")
                    };
                    *name = "index".into();
                    let OccurrenceNode::Place(root) = row.structure.edges[&(
                        OccurrenceParent::Place(place_ordinal),
                        OccurrenceEdge::PlaceBase,
                    )] else {
                        panic!("root occurrence")
                    };
                    assert!(row.occurrences[root.0].requires_named_place_root);
                    let NamedPlaceRootFacts::Prepared(roots) = &mut corrupt.named_place_roots
                    else {
                        unreachable!()
                    };
                    let Some(NamedPlaceRootIdentity::Lexical(binding)) = roots.remove(&root) else {
                        panic!("resolved lexical root")
                    };
                    (binding, *span)
                };
                assert_eq!(binding.declaration_span, function.parameter_spans[0]);
                let (cfg, errors) = derive(&mutated, row);
                assert!(errors.is_empty(), "{errors:?}");
                let event = cfg
                    .points
                    .iter()
                    .find(|point| {
                        point.source_span.range == span
                            && matches!(
                                point.kind,
                                CfgPointKind::Borrow { .. }
                                    | CfgPointKind::Evaluate(ScalarExpression::RawAddress { .. })
                            )
                    })
                    .unwrap();
                if item != 1 {
                    let CfgPointKind::Borrow { place, loan } = &event.kind else {
                        panic!("borrow event")
                    };
                    assert_eq!(place.binding, binding);
                    assert_eq!(loan.origin_place, *place);
                    assert_eq!(loan.parent.is_some(), item == 2);
                }
                assert_eq!(
                    derive(&mutated, &corrupt).1,
                    [ReferenceAnalysisError::MissingContext {
                        source_span: SourceSpan::new(source.clone(), missing_span),
                        reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    }]
                );

                if item != 0 {
                    let mut builder =
                        ReferenceFlowBuilder::new(&source, function, &module_names, row, &[], &[]);
                    for slot in &row.boundary.parameters {
                        builder.binding_types.insert(
                            ReferenceBindingId {
                                declaration_span: slot.span,
                                ..binding.clone()
                            },
                            slot.ty.clone(),
                        );
                    }
                    builder.checked.insert(binding.clone());
                    let result = builder.expression(
                        address(&mut mutated),
                        ReferenceScopeId(0),
                        address_ordinal,
                    );
                    let first = &builder.cfg.points[0];
                    assert_eq!(first.source_span.range, missing_span);
                    assert_eq!(
                        first.kind,
                        CfgPointKind::Read {
                            binding: binding.clone()
                        }
                    );
                    if item == 2 {
                        assert_eq!(
                            result.unwrap_err(),
                            ReferenceAnalysisError::MissingOrigin {
                                source_span: SourceSpan::new(source.clone(), span),
                                binding_id: binding,
                                reason:
                                    ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
                            }
                        );
                        assert_eq!(builder.cfg.points.len(), 1);
                    } else {
                        assert!(result.is_ok());
                        assert_eq!(builder.cfg.points.len(), 3);
                    }
                }
            }
        }
    }
}
