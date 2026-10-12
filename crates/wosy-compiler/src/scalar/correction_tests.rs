use super::*;

fn same_returned_entry(text: &str, reference: ScalarType, indexed: bool) {
    let source = SourceIdentity::new(
        "through-support".into(),
        "app".into(),
        "src/main.w".into(),
        "r1".into(),
    );
    let parsed = crate::parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Expression)
        .expect("expression");
    let expression = derive_expression(&node, &source);
    let ScalarExpression::Assignment(outer) = &expression else {
        panic!("outer assignment")
    };
    let ScalarPlace::Dereference { pointer, .. } = reference_root(&outer.targets[0].place) else {
        panic!("pointer operand")
    };
    let ScalarExpression::Assignment(inner) = pointer.as_ref() else {
        panic!("inner assignment")
    };
    let owner = ProducerCfgOwnerId::Module(source.clone());
    let program = ScalarProgram {
        source: source.clone(),
        items: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        target_layout: ScalarTargetLayout::NATIVE64,
        resolved_literals: RefCell::new(Vec::new()),
        resolved_assignment_types: RefCell::new(Vec::new()),
        resolved_expression_outputs: RefCell::new(Vec::new()),
        top_level_reference_cfg: ScalarReferenceCfg::new(owner.clone()),
        terminal_owners: Vec::new(),
        resolved_assignment_outputs: RefCell::new(HashMap::new()),
    };
    let mut diagnostics = Vec::new();
    assert_eq!(
        typecheck::expression_type(
            &expression,
            &BTreeMap::from([("target".into(), reference.clone())]),
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            false
        ),
        ScalarType::U32
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let binding = ReferenceBindingId {
        source: source.clone(),
        function_span: outer.span,
        declaration_span: inner.targets[0].target_span,
        block_span: outer.span,
        kind: ReferenceBindingKind::Declared,
    };
    let assignments = program.resolved_assignment_types.borrow().clone();
    let expressions = program.resolved_expression_outputs.borrow().clone();
    let inner_fact = assignments
        .iter()
        .find(|fact| fact.occurrence == *inner.occurrence)
        .expect("inner type fact");
    let mut builder = ReferenceFlowBuilder {
        rebind_association_count: 0,
        rebind_associations: BTreeMap::new(),
        source: source.clone(),
        function_span: outer.span,
        assignment_context_span: outer.span,
        names: BTreeMap::from([("target".into(), binding.clone())]),
        global_bindings: BTreeMap::new(),
        module_names: HashSet::new(),
        call_sites: Vec::new(),
        origins: HashMap::new(),
        checked: HashSet::from([binding.clone()]),
        binding_types: HashMap::from([(binding.clone(), reference)]),
        structs: Vec::new(),
        enums: Vec::new(),
        generic_scope: owner.clone(),
        generic_declarations: Vec::new(),
        generic_parameters: BTreeSet::new(),
        assignment_outputs: HashMap::from([(
            ResolvedAssignmentOutputId {
                source: source.clone(),
                function_span: outer.span,
                target_span: inner.targets[0].target_span,
                output_index: 0,
            },
            inner_fact.replacements.outputs[0].ty.clone(),
        )]),
        value_types: assignment::SourceValueTypeFacts {
            assignments,
            expressions,
        },
        literal_signatures: Vec::new(),
        dereference_points: HashMap::new(),
        index_points: HashMap::new(),
        array_elements: Box::new(HashMap::new()),
        cfg: ScalarReferenceCfg::new(owner),
        next_scope: 1,
        cursor: None,
    };
    builder.checked_storage(&binding, inner.targets[0].place.clone());
    builder.point(
        CfgPointKind::Entry,
        outer.span,
        ReferenceScopeId(0),
        BTreeSet::new(),
    );
    let selectors = builder
        .assignment_selectors(&outer.targets[0].place, ReferenceScopeId(0))
        .expect("pointer selectors");
    let assignment::EvaluatedAssignmentRoot::Through(entry) = selectors.root else {
        panic!("through root")
    };
    if indexed {
        assert_eq!(selectors.indexes.len(), 1);
        assert_ne!(selectors.indexes[0], entry.value);
        assert!(entry.value.point.0 < selectors.indexes[0].point.0);
        assert_eq!(builder.cursor, Some(selectors.indexes[0].point));
    } else {
        assert!(selectors.indexes.is_empty());
    }
    assert!(builder.dereference_points.is_empty());
    let result = builder
        .cfg
        .evaluated_values
        .results
        .iter()
        .find(|result| result.operation.occurrence == *inner.occurrence)
        .expect("actual saved result");
    assert_eq!(entry.value, result.value);
    assert_eq!(
        entry.value.resolved.output_projection,
        inner.occurrence.output_projection
    );
    assert!(
        matches!(builder.cfg.points[entry.value.point.0].kind, CfgPointKind::AssignmentResult { snapshot_point, output: 0 } if snapshot_point == result.snapshot)
    );
    let facts = builder
        .cfg
        .evaluated_values
        .values
        .iter()
        .filter(|fact| fact.id == entry.value)
        .collect::<Vec<_>>();
    assert_eq!(facts.len(), 1);
    assert_eq!(entry.shape, facts[0].shape);
}

#[test]
fn compositional_assignment_result_support_static_through_nested_operand_same_returned_entry() {
    same_returned_entry(
        "%%start\n(*(target = null) = 9);\n%%end",
        ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Mutable,
            inner: Box::new(ScalarType::U32),
        },
        false,
    );
}

#[test]
fn compositional_assignment_result_support_static_through_pointer_distinct_from_index_entry() {
    same_returned_entry(
        "%%start\n((*(target = null))[0] = 9);\n%%end",
        ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Mutable,
            inner: Box::new(ScalarType::Array {
                element: Box::new(ScalarType::U32),
                length: 2,
                length_span: ByteSpan::new(0, 0),
                span: ByteSpan::new(0, 0),
            }),
        },
        true,
    );
}

fn prove_assignment_semantic_array_producer(layout: ScalarTargetLayout, kind: &str) {
    let source = SourceIdentity::new("assignment-semantic-array-proof".into(), "app".into(), "src/main.w".into(), "r1".into());
    let original_source = source.clone();
    let text = match kind {
        "name" => "%%start\nu8() inspect = fn { u8[2] items = [11, 22]; u8[2] replacement = [33, 44]; u8[2] old = (items = replacement); items[0] = 55; old[0] };\ninspect();\n%%end",
        "call" => "%%start\nu8[2]() make = fn { u8[2] packet = [33, 44]; packet };\nu8() inspect = fn { u8[2] items = [11, 22]; u8[2] old = (items = make()); old[0] };\ninspect();\n%%end",
        "nested" => "%%start\nu8() inspect = fn { u8[2] items = [11, 22]; u8[2] replacement = [33, 44]; u8[2] old = (items = (replacement = items)); old[0] };\ninspect();\n%%end",
        _ => panic!("specified producer kind"),
    }.to_owned();
    let original_text = text.clone();
    let parsed = crate::parse_source(source.clone(), text.clone(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = crate::derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let original_program = validation.program.clone();
    assert_eq!(validation.program.target_layout, layout);
    assert_eq!(validation.program.source, source);
    let functions = validation.program.items.iter().filter_map(|item| match item {
        ScalarItem::Function(function) if function.name == "inspect" => Some(function),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(functions.len(), 1);
    let function = functions[0];
    let bindings = function.body.items.iter().filter_map(|item| match item {
        ScalarBlockItem::LocalBinding(binding) if binding.name == "old" => Some(binding),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(bindings.len(), 1);
    let ScalarExpression::Assignment(assignment) = &bindings[0].value else { panic!("actual outer assignment"); };
    assert_eq!(assignment.targets.len(), 1);
    assert!(matches!(&assignment.targets[0].place, ScalarPlace::Name { name, .. } if name == "items"));
    assert_eq!(assignment.values.len(), 1);
    let rhs_expression = &assignment.values[0];
    let projection = match (kind, rhs_expression) {
        ("name", ScalarExpression::Name { name, .. }) if name == "replacement" => Vec::new(),
        ("call", ScalarExpression::Call { name, occurrence, .. }) if name == "make" => occurrence.output_projection.clone(),
        ("nested", ScalarExpression::Assignment(inner)) => {
            assert_eq!(inner.targets.len(), 1);
            assert!(matches!(&inner.targets[0].place, ScalarPlace::Name { name, .. } if name == "replacement"));
            inner.occurrence.output_projection.clone()
        }
        _ => panic!("specified actual RHS variant"),
    };
    let cfg = &function.reference_cfg;
    let owner = cfg.producer_owner.clone();
    assert!(matches!(&owner, ProducerCfgOwnerId::Function { source: owner_source, .. } if owner_source == &source));
    let evaluations = cfg.evaluated_values.assignment_evaluations.iter().filter(|row| row.operation.occurrence == *assignment.occurrence).collect::<Vec<_>>();
    assert_eq!(evaluations.len(), 1);
    let evaluation = evaluations[0];
    assert_eq!(evaluation.operation.producer, owner);
    let plans = cfg.evaluated_values.assignment_plans.iter().filter(|row| row.operation == evaluation.operation).collect::<Vec<_>>();
    assert_eq!(plans.len(), 1);
    let plan = plans[0];
    assert_eq!(plan.targets.len(), 1);
    assert!(matches!(&plan.targets[0].place, ScalarPlace::Name { name, .. } if name == "items"));
    assert_eq!(plan.replacements.len(), 1);
    assert_eq!(evaluation.rhs.len(), 1);
    assert_eq!(evaluation.destinations.len(), 1);
    assert_eq!(evaluation.previous.len(), 1);
    assert_eq!(evaluation.writes.len(), 1);
    assert_eq!(evaluation.results.len(), 1);
    let rhs = &evaluation.rhs[0];
    let destination = &evaluation.destinations[0];
    let snapshot = &evaluation.previous[0];
    let write = &evaluation.writes[0];
    let result = &evaluation.results[0];
    let replacement = &plan.replacements[0];
    assert_eq!(rhs.expression_ordinal, 0);
    assert_eq!(rhs.producer_output, 0);
    assert_eq!(replacement.expression_ordinal, rhs.expression_ordinal);
    assert_eq!(replacement.producer_output, rhs.producer_output);
    assert_eq!(replacement.destination_ordinal, 0);
    assert_eq!(rhs.value.resolved.producer, owner);
    assert_eq!(rhs.value.resolved.occurrence, SourceSpan::new(source.clone(), expression_span(rhs_expression)));
    assert_eq!(rhs.value.resolved.output_projection, projection);
    assert_eq!(rhs.value.resolved.output, rhs.producer_output);
    assert_eq!(cfg.points[rhs.value.point.0].id, rhs.value.point);
    assert_eq!(cfg.points[rhs.value.point.0].source_span, rhs.value.resolved.occurrence);
    let facts = cfg.evaluated_values.values.iter().filter(|fact| fact.id == rhs.value).collect::<Vec<_>>();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].shape.producer, owner);
    let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == facts[0].shape).collect::<Vec<_>>();
    assert_eq!(shapes.len(), 1);
    assert!(scalar_type_equal(&shapes[0].ty, &replacement.source_type));
    assert_ne!(shapes[0].ty, replacement.source_type);
    let resolved = validation.program.resolved_expression_outputs.borrow();
    let output_rows = resolved.iter().filter(|row| row.source_span == rhs.value.resolved.occurrence).collect::<Vec<_>>();
    assert_eq!(output_rows.len(), 1);
    assert_eq!(output_rows[0].outputs.outputs.len(), 1);
    assert_eq!(replacement.source_type, output_rows[0].outputs.outputs[0].ty);
    assert_eq!(destination.id.operation, evaluation.operation);
    assert_eq!(destination.id.target, 0);
    assert_eq!(destination.source_span, plan.targets[0].source_span);
    assert_eq!(destination.shape.producer, owner);
    let destination_shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == destination.shape).collect::<Vec<_>>();
    assert_eq!(destination_shapes.len(), 1);
    assert!(scalar_type_equal(&destination_shapes[0].ty, &plan.targets[0].ty));
    assert!(matches!(&cfg.points[destination.point.0].kind, CfgPointKind::AssignmentDestination { storage, place } if storage == &destination.storage && place == &plan.targets[0].place));
    assert_eq!(snapshot.destination, destination.id);
    let SnapshotState::Saved(saved) = &snapshot.state else { panic!("observed saved old value"); };
    assert_eq!(saved.point, snapshot.point);
    assert_eq!(saved.resolved.producer, owner);
    assert_eq!(saved.resolved.occurrence, SourceSpan::new(source.clone(), assignment.span));
    assert_eq!(saved.resolved.output_projection, assignment.occurrence.output_projection);
    assert_eq!(saved.resolved.output, 0);
    assert!(matches!(&cfg.points[snapshot.point.0].kind, CfgPointKind::AssignmentSnapshot { destination_point, observed: true, .. } if destination_point == &destination.point));
    assert_eq!(write.destination, destination.id);
    assert_eq!(write.replacement, rhs.value);
    assert_eq!(write.previous, snapshot.point);
    assert_eq!(result.operation, evaluation.operation);
    assert_eq!(result.output, 0);
    assert_eq!(result.snapshot, snapshot.point);
    assert_eq!(result.value.resolved, saved.resolved);
    assert_ne!(result.value.point, saved.point);
    assert!(matches!(&cfg.points[result.value.point.0].kind, CfgPointKind::AssignmentResult { snapshot_point, output: 0 } if snapshot_point == &snapshot.point));
    for value in [&rhs.value, saved, &result.value] {
        let rows = cfg.evaluated_values.values.iter().filter(|fact| fact.id == *value).collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].shape.producer, owner);
        assert_eq!(cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == rows[0].shape).count(), 1);
        assert_eq!(cfg.points.iter().filter(|point| point.id == value.point).count(), 1);
        assert_eq!(cfg.points[value.point.0].id, value.point);
    }
    assert!(rhs.value.point.0 < destination.point.0);
    assert!(destination.point.0 < snapshot.point.0);
    assert!(snapshot.point.0 < write.point.0);
    assert!(write.point.0 < result.value.point.0);
    assert_eq!(validation.program, original_program);
    assert_eq!(source, original_source);
    assert_eq!(text, original_text);
}

#[test]
fn assignment_semantic_array_agreement_name_native64() { prove_assignment_semantic_array_producer(ScalarTargetLayout::NATIVE64, "name"); }
#[test]
fn assignment_semantic_array_agreement_name_wasm32() { prove_assignment_semantic_array_producer(ScalarTargetLayout::WASM32, "name"); }
#[test]
fn assignment_semantic_array_agreement_call_native64() { prove_assignment_semantic_array_producer(ScalarTargetLayout::NATIVE64, "call"); }
#[test]
fn assignment_semantic_array_agreement_call_wasm32() { prove_assignment_semantic_array_producer(ScalarTargetLayout::WASM32, "call"); }
#[test]
fn assignment_semantic_array_agreement_nested_native64() { prove_assignment_semantic_array_producer(ScalarTargetLayout::NATIVE64, "nested"); }
#[test]
fn assignment_semantic_array_agreement_nested_wasm32() { prove_assignment_semantic_array_producer(ScalarTargetLayout::WASM32, "nested"); }
