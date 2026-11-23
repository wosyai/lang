use super::*;
use crate::scalar::typecheck::{tail_conditional_types, TailConditionalTyping};

#[test]
fn checked_tail_conditional_publishes_one_producer_with_ordered_outputs() {
    let source = source();
    let text =
        "%%start\n(u16, bool)() pair = fn { 1; if (true) { 17, true } else { 42, false } };\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let block_node = parsed
        .result
        .canonical_cst()
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Block)
        .expect("function body CST");
    let body = derive_block(&block_node);
    let types = [ScalarType::U16, ScalarType::Bool];
    let function = ScalarFunction {
        name: "pair".into(),
        name_span: body.span,
        signature: ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: types
                    .iter()
                    .map(|ty| ScalarOutput {
                        ty: ty.clone(),
                        span: body.span,
                    })
                    .collect(),
                span: body.span,
            },
            parameters: Vec::new(),
        },
        parameters: Vec::new(),
        parameter_spans: Vec::new(),
        span: body.span,
        body,
        generic_parameters: Vec::new(),
        overload_arms: Vec::new(),
        reference_cfg: ScalarReferenceCfg {
            primary_call_points: HashMap::new(),
            guarded_scalar_allocation: None,
            facts: Vec::new(),
            points: Vec::new(),
            initial_points: Vec::new(),
            allocations: BTreeSet::new(),
            specialized_policies: Vec::new(),
        },
    };
    let ScalarType::Callable { outputs, .. } = &function.signature else {
        unreachable!()
    };
    let tail = function
        .body
        .tail_conditional()
        .expect("raw tail conditional");
    let span = tail.span;
    let owners = ScalarAssignmentOwners::new(&[]);
    let mut inference = ScalarInferenceBuilder::for_validation(&owners);
    let mut diagnostics = Vec::new();
    let actual = tail_conditional_types(
        tail,
        &outputs.outputs,
        &BTreeMap::new(),
        &BTreeSet::new(),
        &BTreeMap::new(),
        TailConditionalTyping::Program(&ScalarProgramTyping {
            source: &source,
            structs: &[],
            enums: &[],
            target_layout: ScalarTargetLayout::NATIVE64,
        }),
        &mut diagnostics,
        false,
        &ScalarOverloadEnvironment::single(&source, &[]).typing(0),
        &mut inference,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(actual, types);

    let (_, mut checked) = inference.finish();
    let calls = resolve_function_call_sites(
        &source,
        &function,
        &CallDeclarationIndex::default(),
        &HashMap::new(),
        true,
        &mut checked,
    )
    .unwrap();
    let structure = &calls.structure;
    let root = structure.root;
    let block = &structure.blocks[root.0];
    assert_eq!(block.items.len(), 2);
    let BlockItemKind::FinalOutput(primary) = block.items[1].kind else {
        panic!("canonical final item")
    };
    let producer = primary.expression;
    assert_eq!(
        block.final_outputs().copied().collect::<Vec<_>>(),
        [
            FinalOutputSlot {
                item: 1,
                output: 0,
                expression: producer,
                span,
            },
            FinalOutputSlot {
                item: 1,
                output: 1,
                expression: producer,
                span,
            },
        ]
    );
    assert_eq!(calls.occurrences.len(), 7);
    assert_eq!(calls.occurrences[producer.0].span, span);
    assert_eq!(calls.conditionals.len(), 1);
    assert_eq!(structure.conditional_output_projections.len(), 1);
    assert_eq!(
        structure.conditional_output_projections[&producer].types,
        types
    );
    for (output, (slot, ty)) in calls.boundary.root_outputs.iter().zip(&types).enumerate() {
        assert_eq!((slot.output, &slot.ty), (output, ty));
    }
    assert_eq!(calls.boundary.root_outputs.len(), types.len());
    assert_eq!(
        structure.edges[&(
            OccurrenceParent::Block(root),
            OccurrenceEdge::FinalOutput { item: 1, output: 0 }
        )],
        OccurrenceNode::Expression(producer),
    );
    assert!(!structure.edges.contains_key(&(
        OccurrenceParent::Block(root),
        OccurrenceEdge::FinalOutput { item: 1, output: 1 },
    )));
    let ConditionalRegion::Value {
        span: region_span,
        condition,
        then_block,
        else_block,
        resume,
    } = calls.conditionals[&producer]
    else {
        panic!("value conditional region")
    };
    assert_eq!((region_span, resume), (span, producer));
    let parent = OccurrenceParent::Expression(producer);
    assert_eq!(
        structure.edges[&(parent, OccurrenceEdge::Condition)],
        OccurrenceNode::Expression(condition)
    );
    assert_ne!(then_block, else_block);
    for (edge, arm) in [
        (OccurrenceEdge::ThenBranch, then_block),
        (OccurrenceEdge::ElseBranch, else_block),
    ] {
        assert_eq!(structure.edges[&(parent, edge)], OccurrenceNode::Block(arm));
        let outputs = structure.blocks[arm.0].final_outputs().collect::<Vec<_>>();
        assert_eq!(outputs.len(), types.len());
        for (output, (slot, ty)) in outputs.into_iter().zip(&types).enumerate() {
            assert_eq!(slot.output, output);
            assert_eq!(&structure.checked_types.expressions[&slot.expression], ty);
        }
    }
}

fn check_detached_tail_conditional(text: &str, check: impl Fn(&ScalarReferenceCfg)) {
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let source = source();
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let node = parsed
            .result
            .canonical_cst()
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::FunctionDecl)
            .expect("function CST");
        let signature = derive_type(
            &node
                .children()
                .find(|n| n.kind() == SyntaxKind::CallableType)
                .unwrap(),
        );
        let body = derive_block(
            &node
                .children()
                .find(|n| n.kind() == SyntaxKind::Block)
                .unwrap(),
        );
        let (parameters, parameter_spans): (Vec<_>, Vec<_>) = node
            .children()
            .find(|n| n.kind() == SyntaxKind::Parameters)
            .unwrap()
            .children_with_tokens()
            .filter_map(|element| match element {
                NodeOrToken::Token(token) if token.kind() == SyntaxKind::Identifier => {
                    Some((token.text().to_owned(), token_span(&token)))
                }
                _ => None,
            })
            .unzip();
        let function = ScalarFunction {
            name: "pair".into(),
            name_span: token_span(&direct_token(&node, SyntaxKind::Identifier).unwrap()),
            signature,
            parameters,
            parameter_spans,
            span: wosy_syntax::byte_span(&node),
            body,
            generic_parameters: Vec::new(),
            overload_arms: Vec::new(),
            reference_cfg: ScalarReferenceCfg {
                primary_call_points: HashMap::new(),
                guarded_scalar_allocation: None,
                facts: Vec::new(),
                points: Vec::new(),
                initial_points: Vec::new(),
                allocations: BTreeSet::new(),
                specialized_policies: Vec::new(),
            },
        };
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &function.signature
        else {
            unreachable!()
        };
        let scope = function
            .parameters
            .iter()
            .cloned()
            .zip(parameters.iter().cloned())
            .collect();
        let tail = function
            .body
            .tail_conditional()
            .expect("raw tail conditional");
        let owners = ScalarAssignmentOwners::new(&[]);
        let mut inference = ScalarInferenceBuilder::for_validation(&owners);
        let mut diagnostics = Vec::new();
        let actual = tail_conditional_types(
            tail,
            &outputs.outputs,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            TailConditionalTyping::Program(&ScalarProgramTyping {
                source: &source,
                structs: &[],
                enums: &[],
                target_layout: target,
            }),
            &mut diagnostics,
            false,
            &ScalarOverloadEnvironment::single(&source, &[]).typing(0),
            &mut inference,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            actual,
            outputs
                .outputs
                .iter()
                .map(|slot| slot.ty.clone())
                .collect::<Vec<_>>(),
        );
        let (resolved_assignment_outputs, checked) = inference.finish();
        let program = ScalarProgram {
            source,
            items: vec![ScalarItem::Function(function)],
            structs: Vec::new(),
            enums: Vec::new(),
            target_layout: target,
            layouts: ScalarTargetLayouts::new(target),
            resolved_assignment_outputs,
        };
        let calls = resolve_program_call_sites_with_occurrences(&program, checked).unwrap();
        let results = analyze_program_references(&program, &calls);
        assert!(
            results.errors.is_empty(),
            "{target:?}: {:?}",
            results.errors
        );
        let [(0, result)] = results.updates.as_slice() else {
            panic!("one detached callable result")
        };
        check(&result.cfg);
    }
}

#[test]
fn detached_query_joins_checked_tail_conditional_outputs_by_position() {
    let text = "%%start\n(*u8, *u8)(bool, *u8, *u8, *u8, *u8) pair = fn(pick, a, b, c, d) { if (pick) { a, b } else { c, d } };\n%%end";
    check_detached_tail_conditional(text, |cfg| {
        let branches = cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Branch { .. }))
            .collect::<Vec<_>>();
        let [branch] = branches.as_slice() else {
            panic!("one branch for the source conditional")
        };
        let [then_entry, else_entry] = branch.successors.as_slice() else {
            panic!("two arm successors")
        };
        let reads = cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Read { .. }))
            .collect::<Vec<_>>();
        let [condition, then_first, then_second, else_first, else_second] = reads.as_slice() else {
            panic!("one condition and two ordered reads per arm")
        };
        assert_eq!(condition.successors, [branch.id]);
        assert_eq!(condition.source_span, branch.source_span);
        for (entry, arm_reads) in [
            (then_entry, [then_first, then_second]),
            (else_entry, [else_first, else_second]),
        ] {
            let entry = &cfg.points[entry.0];
            assert!(matches!(entry.kind, CfgPointKind::ScopeEnter));
            assert_eq!(entry.successors, [arm_reads[0].id]);
            assert!(arm_reads.iter().all(|point| point.scope == entry.scope));
        }
        let returned = cfg
            .points
            .iter()
            .filter_map(|point| match point.kind {
                CfgPointKind::ReturnOutput {
                    output,
                    value_point,
                    checked: true,
                } => Some((output, value_point, point)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(returned.len(), 2);
        assert_ne!(returned[0].1, returned[1].1);
        for (position, (output, value_point, point)) in returned.into_iter().enumerate() {
            assert_eq!(output, position);
            let joined = &cfg.points[value_point.0];
            assert_eq!(
                joined.kind,
                CfgPointKind::ConditionalOutput {
                    then_point: [then_first, then_second][position].id,
                    else_point: [else_first, else_second][position].id,
                },
            );
            assert_eq!(joined.possible_origins, point.possible_origins);
            let origins = point
                .possible_origins
                .as_ref()
                .unwrap()
                .iter()
                .map(|id| {
                    let ReferenceOrigin::BorrowedFrom { parameter, .. } = &cfg.facts[id.0] else {
                        panic!("parameter origin")
                    };
                    *parameter
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(origins, BTreeSet::from([position + 1, position + 3]));
        }
    });
}

#[test]
fn detached_query_links_independent_scalar_if_in_second_projected_slot() {
    let text = "%%start\n(*u8, *u8)(bool, bool, *u8, *u8) pair = fn(pick, inner, sibling, borrowed) { if (pick) { sibling, if (inner) { u8 local = 42; &local } else { borrowed } } else { sibling, borrowed } };\n%%end";
    check_detached_tail_conditional(text, |cfg| {
        let returns = cfg
            .points
            .iter()
            .filter_map(|point| match point.kind {
                CfgPointKind::ReturnOutput {
                    output,
                    value_point,
                    checked: true,
                } => Some((output, value_point, point)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let [(0, first, first_return), (1, second, second_return)] = returns.as_slice() else {
            panic!("two ordered checked outputs");
        };
        let paired_links = |id: CfgPointId| {
            let CfgPointKind::ConditionalOutput {
                then_point,
                else_point,
            } = cfg.points[id.0].kind
            else {
                panic!("expected an explicit conditional output at {id:?}");
            };
            [then_point, else_point]
        };
        let origins = |point: &ReferenceFlowPoint| {
            point
                .possible_origins
                .as_ref()
                .unwrap()
                .iter()
                .map(|origin| &cfg.facts[origin.0])
                .collect::<Vec<_>>()
        };
        for id in paired_links(*first) {
            assert!(matches!(
                origins(&cfg.points[id.0]).as_slice(),
                [ReferenceOrigin::BorrowedFrom { parameter: 2, .. }]
            ));
        }
        assert!(matches!(
            origins(first_return).as_slice(),
            [ReferenceOrigin::BorrowedFrom { parameter: 2, .. }]
        ));
        let [inner, outer_else] = paired_links(*second);
        let [inner_then, inner_else] = paired_links(inner);
        assert!(matches!(
            origins(&cfg.points[inner_then.0]).as_slice(),
            [ReferenceOrigin::Fresh { .. }]
        ));
        for id in [inner_else, outer_else] {
            assert!(matches!(
                origins(&cfg.points[id.0]).as_slice(),
                [ReferenceOrigin::BorrowedFrom { parameter: 3, .. }]
            ));
        }
        assert_eq!(
            second_return.possible_origins,
            cfg.points[inner.0].possible_origins
        );
        assert_eq!(origins(second_return).len(), 2);
    });
}

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

#[test]
fn checked_terminal_descriptor_observes_original_local_backing_or_explicit_absence() {
    use super::aggregate::CheckedProducerBacking;
    let source = source();
    let text = r#"%%start
struct Packet { *?u8 data; u64 length; }
*?u8 captured = null;

(*Packet, bool)(u64, bool) make_backed = fn(count, present) {
	u8[count] bytes;
	if (present) {
		*?u8 data = null;
		unsafe { data = &?bytes[0]; };
		Packet descriptor = { .data = data; .length = count; };
		&descriptor, true
	} else { null, false }
};
(*Packet, bool)(u64, bool) make_empty = fn(count, present) {
	u8[count] bytes;
	u8 observed = bytes[0];
	observed;
	if (present) {
		Packet descriptor = { .data = null; .length = 0; };
		&descriptor, true
	} else { null, false }
};
(*Packet, bool)(u64, bool, bool) make_nullable = fn(count, present, backed) {
	u8[count] bytes;
	if (present) {
		*?u8 data = null;
		if (backed) { unsafe { data = &?bytes[0]; }; };
		Packet descriptor = { .data = data; .length = count; };
		&descriptor, true
	} else { null, false }
};
(*Packet, bool)(u64, bool) make_local_null = fn(count, present) {
	u8[count] bytes;
	u8 observed = bytes[0];
	observed;
	if (present) {
		*?u8 data = null;
		Packet descriptor = { .data = data; .length = count; };
		&descriptor, true
	} else { null, false }
};
bool() unknown_effect = fn { true };
(*Packet, bool)(u64, bool) make_conflicted = fn(count, present) {
	u8[count] bytes;
	if (present) {
		*?u8 data = null;
		unsafe { data = &?bytes[0]; };
		Packet descriptor = { .data = data; .length = count; };
		&descriptor, unknown_effect()
	} else { null, false }
};
(*Packet, bool)(u64, bool) make_nonlocal = fn(count, present) {
	u8[count] bytes;
	if (present) {
		*?u8 data = null;
		unsafe { data = &?bytes[0]; };
		captured = data;
		Packet descriptor = { .data = data; .length = count; };
		&descriptor, true
	} else { null, false }
};
%%end"#;
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let module = &prepared.project().modules[0];
        let mut checked = 0;
        for (item, definition) in module.items.iter().enumerate() {
            let ScalarItem::Function(function) = definition else { continue };
            if function.name == "unknown_effect" {
                continue;
            }
            let mut result = derive_function_reference_result(
                &source, function, &HashSet::from(["captured".to_owned()]), prepared.function_calls(0, item),
                &module.structs, &module.enums, true, &mut Vec::new(),
            );
            assert!(!result.aggregate.checked_producer_authoritative);
            let contracts = ReferenceOutputContracts::default();
            let mut replays = Vec::new();
            let mut cursor = 0;
            transfer_function_output_contracts(
                &mut result, &contracts, &contracts, &mut HashMap::new(), &mut replays, &mut cursor,
            );
            assert_eq!(cursor, replays.len());
            assert!(replays.iter().all(|replay| replay.errors.is_empty()));
            assert!(result.aggregate.checked_producer_authoritative);
            let proof = &result.aggregate.checked_producer_backing;
            assert_eq!(proof.len(), 2, "{function:?}");
            let positive = match function.name.as_str() {
                "make_backed" | "make_nullable" | "make_local_null" => {
                    let allocation = result.cfg.points.iter().find_map(|point| match &point.kind {
                        CfgPointKind::Bind { binding } if result.cfg.allocations.contains(binding) => Some(point.id),
                        _ => None,
                    }).expect("actual typed local runtime array allocation");
                    match function.name.as_str() {
                        "make_backed" => CheckedProducerBacking::FreshWithAllocation(allocation),
                        "make_nullable" => CheckedProducerBacking::FreshWithNullableAllocation(allocation),
                        _ => CheckedProducerBacking::FreshWithExplicitNullAndLiveAllocation(allocation),
                    }
                }
                "make_empty" => CheckedProducerBacking::FreshWithoutBacking,
                "make_conflicted" | "make_nonlocal" => CheckedProducerBacking::Unknown,
                unexpected => panic!("unexpected producer {unexpected}"),
            };
            assert!(proof.values().any(|result| *result == positive), "{proof:?}");
            assert!(proof.values().any(|result| *result == CheckedProducerBacking::NoDescriptor), "{proof:?}");
            checked += 1;
        }
        assert_eq!(checked, 6);
    }
}

#[test]
fn checked_block_observes_the_exact_terminal_before_scope_exit_and_converges() {
    let text = r#"%%start
*u8(*u8) forward = fn(input) { input };
*u8(*u8) frozen = fn(input) { unsafe { input } };
*u8(*u8) relay = fn(input) {
    *u8 result = unsafe { forward(input) };
    result
};
%%end"#;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(declarations, layout);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let (results, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let public = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        for (item, expected_frozen) in [(1, true), (2, false)] {
            let result = &results[item].1;
            let [(wrapper, terminal)] = result.proof_points.checked_block_values.as_slice() else {
                panic!("expected exactly one closed block terminal in body {item}")
            };
            let points = &result.cfg.points;
            assert!(matches!(&points[wrapper.0].kind, CfgPointKind::Evaluate(ScalarExpression::Block(_))));
            assert!(points[terminal.0 + 1..wrapper.0].iter().any(|point| matches!(point.kind, CfgPointKind::ScopeExit)));
            assert_eq!(result.cfg.initial_points[wrapper.0].possible_origins.is_some(), expected_frozen);
            assert_eq!(points[wrapper.0].possible_origins, points[terminal.0].possible_origins);
            assert_eq!(points[wrapper.0].possible_origins.is_some(), expected_frozen);
            let ScalarItem::Function(function) = &public.program.items[item] else {
                panic!("expected checked function")
            };
            let cfg = &function.reference_cfg;
            let observed = cfg.points[wrapper.0].possible_origins.as_ref().expect("proven block value");
            assert_eq!(cfg.points[terminal.0].possible_origins.as_ref(), Some(observed));
            assert_eq!(observed.len(), 1);
            assert!(matches!(&cfg.facts[observed.first().unwrap().0], ReferenceOrigin::BorrowedFrom { parameter: 0, .. }));
        }
    }
}

#[test]
fn checked_ordinary_join_preserves_null_and_missing_provenance_on_separate_arms() {
    let text = r#"%%start
*u8(*u8) forward = fn(input) { input };
*u8(*u8, bool) select = fn(input, flag) {
    *u8 selected = if (flag) { forward(input) } else { null };
    selected
};
%%end"#;
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parse_source(source(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(declarations, layout);
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let calls = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let (mut results, errors) = derive_module_reference_origins(
            &program.source, &program.items, &calls, &program.structs, &program.enums, false,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let result = &mut results[1].1;
        let [(join, [Some(then_point), Some(else_point)])] = result.proof_points.checked_join_values.as_slice() else {
            panic!("expected ordered exact checked arm terminals")
        };
        let (join, then_point, else_point) = (*join, *then_point, *else_point);
        assert!(then_point < else_point && else_point < join);
        let points = &result.cfg.points;
        assert!(matches!(points[join.0].kind, CfgPointKind::Join));
        assert!(matches!(&points[then_point.0].kind,
            CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { .. }, .. }));
        assert!(points[then_point.0].possible_origins.is_none());
        let null = points[else_point.0].possible_origins.as_ref().expect("known null arm");
        assert_eq!(null.len(), 1);
        assert!(matches!(&result.cfg.facts[null.first().unwrap().0], ReferenceOrigin::Null { .. }));
        assert_eq!(points[join.0].possible_origins.as_ref(), Some(null));

        let contracts = ReferenceOutputContracts::default();
        let mut replays = Vec::new();
        transfer_function_output_contracts(
            result, &contracts, &contracts, &mut HashMap::new(), &mut replays, &mut 0,
        );
        let selected_span = text.find("selected =").unwrap() as u32;
        assert!(replays[0].errors.iter().any(|error| matches!(error,
            ReferenceAnalysisError::MissingOrigin {
                binding_id, reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin, ..
            } if binding_id.declaration_span.start == selected_span
        )), "{:?}", replays[0].errors);

        let public = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(public.diagnostics.is_empty(), "{:?}", public.diagnostics);
        let ScalarItem::Function(function) = &public.program.items[1] else {
            panic!("expected checked function")
        };
        let cfg = &function.reference_cfg;
        let known_then = cfg.points[then_point.0].possible_origins.as_ref().expect("converged checked call");
        let known_else = cfg.points[else_point.0].possible_origins.as_ref().expect("preserved null arm");
        assert_eq!(known_then.len(), 1);
        assert_eq!(known_else.len(), 1);
        assert!(matches!(&cfg.facts[known_then.first().unwrap().0], ReferenceOrigin::BorrowedFrom { parameter: 0, .. }));
        assert!(matches!(&cfg.facts[known_else.first().unwrap().0], ReferenceOrigin::Null { .. }));
        assert_eq!(cfg.points[join.0].possible_origins.as_ref(), Some(&known_then.union(known_else).copied().collect()));
    }
}
