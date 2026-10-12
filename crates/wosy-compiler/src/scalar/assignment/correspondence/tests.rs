use super::*;

struct ThroughSupport {
    cfg: ScalarReferenceCfg,
    plan: ResolvedAssignmentPlan,
    evaluation: ScalarAssignmentEvaluation,
    outputs: Vec<ResolvedExpressionOutputFact>,
}

fn support_source(path: &str) -> SourceIdentity {
    SourceIdentity::new(
        "through-support".into(),
        "app".into(),
        path.into(),
        "r1".into(),
    )
}

fn support_point(
    id: usize,
    kind: CfgPointKind,
    source_span: SourceSpan,
    successors: Vec<CfgPointId>,
) -> ReferenceFlowPoint {
    ReferenceFlowPoint {
        id: CfgPointId(id),
        kind,
        source_span,
        scope: ReferenceScopeId(0),
        possible_origins: None,
        possible_environments: BTreeSet::new(),
        possible_projected_environments: Vec::new(),
        possible_callee_environments: BTreeSet::new(),
        possible_backings: Vec::new(),
        possible_projected_backings: Vec::new(),
        backing_argument_paths: Vec::new(),
        global_callable_call_paths: Vec::new(),
        global_callable_output_effects: Vec::new(),
        call_import_paths: Vec::new(),
        successors,
    }
}

fn support(source: SourceIdentity) -> ThroughSupport {
    let parsed = crate::parse_source(
        source.clone(),
        "%%start\n(*(target = replacement) = 9);\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Expression)
        .expect("expression");
    let ScalarExpression::Assignment(outer) = derive_expression(&node, &source) else {
        panic!("outer assignment")
    };
    let ScalarPlace::Dereference {
        pointer: operand, ..
    } = &outer.targets[0].place
    else {
        panic!("pointer")
    };
    let ScalarExpression::Assignment(inner) = operand.as_ref() else {
        panic!("inner assignment")
    };
    let producer = ProducerCfgOwnerId::Module(source.clone());
    let operation = AssignmentOperationId {
        producer: producer.clone(),
        occurrence: outer.occurrence.as_ref().clone(),
    };
    let inner_operation = AssignmentOperationId {
        producer: producer.clone(),
        occurrence: inner.occurrence.as_ref().clone(),
    };
    let mut cfg = ScalarReferenceCfg::new(producer.clone());
    let reference = ScalarType::CheckedReference {
        mutability: ScalarReferenceMutability::Mutable,
        inner: Box::new(ScalarType::U32),
    };
    let shape = intern_value_shape(
        &mut cfg.evaluated_values.shapes,
        &producer,
        &ScalarType::U32,
        &[],
        &[], &producer, &[],
    )
    .expect("referent shape");
    let pointer_shape = intern_value_shape(
        &mut cfg.evaluated_values.shapes,
        &producer,
        &reference,
        &[],
        &[], &producer, &[],
    )
    .expect("pointer shape");
    let pointer = EvaluatedValueId {
        resolved: ResolvedValueOutputId {
            producer: producer.clone(),
            occurrence: SourceSpan::new(source.clone(), inner.span),
            output_projection: inner.occurrence.output_projection.clone(),
            output: 0,
        },
        point: CfgPointId(3),
    };
    let target_source = SourceSpan::new(source.clone(), outer.targets[0].target_span);
    let binding = ReferenceBindingId {
        source: source.clone(),
        function_span: outer.span,
        declaration_span: inner.targets[0].target_span,
        block_span: outer.span,
        kind: ReferenceBindingKind::Declared,
    };
    let place = ReferencePlaceId {
        source: source.clone(),
        function_span: outer.span,
        declaration_span: binding.declaration_span,
        block_span: outer.span,
        binding: binding.clone(),
        projections: Vec::new(),
        place: inner.targets[0].place.clone(),
    };
    let holder = ReferenceBindingId {
        kind: ReferenceBindingKind::AssignmentPrevious {
            snapshot: CfgPointId(2),
            output: 0,
        },
        ..binding
    };
    let storage = EvaluatedDestinationStorage::Through {
        pointer: pointer.clone(),
        cases: Vec::new(),
    };
    cfg.points = vec![
        support_point(
            0,
            CfgPointKind::Entry,
            SourceSpan::new(source.clone(), outer.span),
            vec![CfgPointId(1)],
        ),
        support_point(
            1,
            CfgPointKind::AssignmentDestination {
                storage: EvaluatedDestinationStorage::Direct(place),
                place: inner.targets[0].place.clone(),
            },
            SourceSpan::new(source.clone(), inner.targets[0].target_span),
            vec![CfgPointId(2)],
        ),
        support_point(
            2,
            CfgPointKind::AssignmentSnapshot {
                destination_point: CfgPointId(1),
                holder,
                observed: true,
            },
            SourceSpan::new(source.clone(), inner.targets[0].target_span),
            vec![CfgPointId(3)],
        ),
        support_point(
            3,
            CfgPointKind::AssignmentResult {
                snapshot_point: CfgPointId(2),
                output: 0,
            },
            pointer.resolved.occurrence.clone(),
            vec![CfgPointId(4)],
        ),
        support_point(
            4,
            CfgPointKind::AssignmentDestination {
                storage: storage.clone(),
                place: outer.targets[0].place.clone(),
            },
            target_source.clone(),
            Vec::new(),
        ),
    ];
    cfg.initial_points = cfg.points.clone();
    cfg.evaluated_values.values.push(EvaluatedValueFact {
        id: pointer.clone(),
        shape: pointer_shape,
        cases: Vec::new(),
    });
    cfg.evaluated_values.results.push(AssignmentResult {
        operation: inner_operation,
        output: 0,
        snapshot: CfgPointId(2),
        value: pointer.clone(),
        consumers: Vec::new(),
    });
    let plan = ResolvedAssignmentPlan {
        operation: operation.clone(),
        targets: vec![ResolvedAssignmentTarget {
            ordinal: 0,
            source_span: target_source.clone(),
            place: outer.targets[0].place.clone(),
            ty: ScalarType::U32,
            storage: ResolvedAssignmentStorage::Through {
                pointer: pointer.resolved.clone(),
            },
        }],
        replacements: Vec::new(),
        previous_outputs: Vec::new(),
        consumption: AssignmentConsumption::Discarded,
    };
    let evaluation = ScalarAssignmentEvaluation {
        operation: operation.clone(),
        rhs: Vec::new(),
        previous: Vec::new(),
        writes: Vec::new(),
        results: Vec::new(),
        destinations: vec![EvaluatedAssignmentDestination {
            id: AssignmentDestinationId {
                operation,
                target: 0,
            },
            point: CfgPointId(4),
            source_span: target_source,
            shape,
            storage,
            selector_inputs: Vec::new(),
        }],
    };
    let outputs = vec![ResolvedExpressionOutputFact {
        source_span: pointer.resolved.occurrence.clone(),
        outputs: ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty: reference,
                span: inner.span,
            }],
            span: inner.span,
        },
    }];
    ThroughSupport {
        cfg,
        plan,
        evaluation,
        outputs,
    }
}

fn check(support: &ThroughSupport) -> Result<(), AssignmentCorrespondenceError> {
    validate_assignment_destinations(
        &support.cfg,
        &support.plan,
        &support.evaluation,
        &support.outputs,
        &[],
    )
}

fn assert_invariant(support: &ThroughSupport, invariant: AssignmentCorrespondenceInvariant) {
    assert_eq!(
        check(support),
        Err(AssignmentCorrespondenceError {
            source_span: support.plan.targets[0].source_span.clone(),
            invariant
        })
    );
}

#[test]
fn compositional_assignment_result_support_static_through_serde_initial() {
    let mut support = support(support_source("src/main.w"));
    assert_eq!(check(&support), Ok(()));
    support
        .cfg
        .evaluated_values
        .assignment_plans
        .push(support.plan.clone());
    support
        .cfg
        .evaluated_values
        .assignment_evaluations
        .push(support.evaluation.clone());
    let bytes = serde_json::to_vec(&support.cfg).expect("serialize CFG");
    let decoded: ScalarReferenceCfg = serde_json::from_slice(&bytes).expect("deserialize CFG");
    assert_eq!(decoded, support.cfg);
    support.cfg = decoded;
    support.cfg.points = support.cfg.initial_points.clone();
    assert_eq!(check(&support), Ok(()));
}

#[test]
fn compositional_assignment_result_support_static_through_equal_offsets_distinct_sources() {
    let left = support(support_source("src/left.w"));
    let right = support(support_source("src/right.w"));
    let ResolvedAssignmentStorage::Through {
        pointer: left_pointer,
    } = &left.plan.targets[0].storage
    else {
        panic!("left pointer")
    };
    let ResolvedAssignmentStorage::Through {
        pointer: right_pointer,
    } = &right.plan.targets[0].storage
    else {
        panic!("right pointer")
    };
    assert_eq!(
        left_pointer.occurrence.range,
        right_pointer.occurrence.range
    );
    assert_ne!(left_pointer, right_pointer);
    assert_eq!(check(&left), Ok(()));
    assert_eq!(check(&right), Ok(()));
}

#[test]
fn compositional_assignment_result_support_static_through_remap() {
    let mut support = support(support_source("src/main.w"));
    let producer = support.cfg.producer_owner.clone();
    let imported = ProducerCfgOwnerId::Module(support_source("src/import.w"));
    let remap = ValueSchemaRemap {
        producers: vec![(producer.clone(), imported.clone())],
        points: (0..5)
            .map(|point| ValuePointRemap {
                producer: producer.clone(),
                point: CfgPointId(point),
                imported: CfgPointId(point),
            })
            .collect(),
        shapes: support
            .cfg
            .evaluated_values
            .shapes
            .iter()
            .map(|shape| {
                (
                    shape.id.clone(),
                    ValueShapeId {
                        producer: imported.clone(),
                        index: shape.id.index,
                    },
                )
            })
            .collect(),
        imports: Vec::new(),
    };
    let original = support.plan.clone();
    support.plan = remap
        .assignment_plan(&support.plan)
        .expect("static Through remap needs no place or loan map");
    support.evaluation = remap
        .assignment_evaluation(&support.evaluation)
        .expect("evaluation remap");
    let context = SourceCapturePolicyContext { structs: Vec::new(), enums: Vec::new(), generic_scope: producer.clone(), generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() };
    let sources = capture_table_sources(&support.cfg, &context).expect("original support constructor closure");
    let forest = capture_root_construction_forest(&support.cfg, &context).expect("actual original support construction forest");
    let mapped_graph = remap
        .tables(&sources, &forest)
        .expect("value tables remap");
    let roots = mapped_graph.iter().filter(|row| row.source == producer).collect::<Vec<_>>();
    let [root] = roots.as_slice() else { panic!("unique original support root"); };
    support.cfg.evaluated_values = root.tables.clone();
    support.cfg.source_capture_inventory = root.capture_inventory.clone();
    assert_eq!(root.children.len(), sources[0].children.len());
    support.cfg.producer_owner = imported.clone();
    if let CfgPointKind::AssignmentDestination { storage, .. } = &mut support.cfg.points[4].kind {
        *storage = support.evaluation.destinations[0].storage.clone();
    }
    assert_eq!(support.plan.targets[0].place, original.targets[0].place);
    assert_eq!(
        support.plan.targets[0].source_span,
        original.targets[0].source_span
    );
    let ResolvedAssignmentStorage::Through { pointer } = &support.plan.targets[0].storage else {
        panic!("static pointer")
    };
    assert_eq!(pointer.producer, imported);
    assert_eq!(check(&support), Ok(()));
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_pointer() {
    let mut support = support(support_source("src/main.w"));
    let ResolvedAssignmentStorage::Through { pointer } = &mut support.plan.targets[0].storage
    else {
        panic!("pointer")
    };
    pointer.output = 1;
    assert_invariant(&support, AssignmentCorrespondenceInvariant::PointerIdentity);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_projection() {
    let mut support = support(support_source("src/main.w"));
    let ResolvedAssignmentStorage::Through { pointer } = &mut support.plan.targets[0].storage
    else {
        panic!("pointer")
    };
    pointer.output_projection.push(8);
    let resolved = pointer.clone();
    let EvaluatedDestinationStorage::Through { pointer, .. } =
        &mut support.evaluation.destinations[0].storage
    else {
        panic!("evaluated pointer")
    };
    pointer.resolved = resolved;
    if let CfgPointKind::AssignmentDestination { storage, .. } = &mut support.cfg.points[4].kind {
        *storage = support.evaluation.destinations[0].storage.clone();
    }
    assert_invariant(&support, AssignmentCorrespondenceInvariant::OperandIdentity);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_storage_role() {
    let mut support = support(support_source("src/main.w"));
    let CfgPointKind::AssignmentDestination {
        storage: EvaluatedDestinationStorage::Direct(place),
        ..
    } = &support.cfg.points[1].kind
    else {
        panic!("direct place")
    };
    support.plan.targets[0].storage = ResolvedAssignmentStorage::Existing(place.clone());
    assert_invariant(&support, AssignmentCorrespondenceInvariant::StorageRole);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_ordinal() {
    let mut support = support(support_source("src/main.w"));
    support.evaluation.destinations[0].id.target = 1;
    assert_invariant(&support, AssignmentCorrespondenceInvariant::Ordinal);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_operand_type() {
    let mut support = support(support_source("src/main.w"));
    support.outputs[0].outputs.outputs[0].ty = ScalarType::RawPointer(Box::new(ScalarType::U64));
    assert_invariant(&support, AssignmentCorrespondenceInvariant::OperandType);
}

#[test]
fn compositional_assignment_result_support_static_through_disconnected_operand() {
    let mut support = support(support_source("src/main.w"));
    support.cfg.points[0].successors.push(CfgPointId(4));
    assert_invariant(&support, AssignmentCorrespondenceInvariant::Connection);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_source() {
    let mut support = support(support_source("src/main.w"));
    support.evaluation.destinations[0].source_span.source = support_source("src/other.w");
    assert_invariant(&support, AssignmentCorrespondenceInvariant::Source);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_shape() {
    let mut support = support(support_source("src/main.w"));
    support.evaluation.destinations[0].shape.index = 100;
    assert_invariant(&support, AssignmentCorrespondenceInvariant::Shape);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_operand_output() {
    let mut support = support(support_source("src/main.w"));
    let ResolvedAssignmentStorage::Through { pointer } = &mut support.plan.targets[0].storage
    else {
        panic!("pointer")
    };
    pointer.output = 1;
    let resolved = pointer.clone();
    let EvaluatedDestinationStorage::Through { pointer, .. } =
        &mut support.evaluation.destinations[0].storage
    else {
        panic!("evaluated pointer")
    };
    pointer.resolved = resolved;
    if let CfgPointKind::AssignmentDestination { storage, .. } = &mut support.cfg.points[4].kind {
        *storage = support.evaluation.destinations[0].storage.clone();
    }
    assert_invariant(&support, AssignmentCorrespondenceInvariant::OperandOutput);
}

#[test]
fn compositional_assignment_result_support_static_through_malformed_saved_result_point() {
    let mut support = support(support_source("src/main.w"));
    let EvaluatedDestinationStorage::Through { pointer, .. } =
        &mut support.evaluation.destinations[0].storage
    else {
        panic!("pointer")
    };
    pointer.point = CfgPointId(2);
    support.cfg.evaluated_values.values[0].id = pointer.clone();
    if let CfgPointKind::AssignmentDestination { storage, .. } = &mut support.cfg.points[4].kind {
        *storage = support.evaluation.destinations[0].storage.clone();
    }
    assert_invariant(&support, AssignmentCorrespondenceInvariant::ProducerPoint);
}

fn prove_assignment_semantic_array_boundary(layout: ScalarTargetLayout, unequal_types: bool, corrupt_identity: bool) {
    assert!(layout == ScalarTargetLayout::NATIVE64 || layout == ScalarTargetLayout::WASM32);
    let source = support_source("src/main.w");
    let original_source = source.clone();
    let original = support(source.clone());
    let original_state = (original.cfg.clone(), original.plan.clone(), original.evaluation.clone(), original.outputs.clone());
    assert_eq!(check(&original), Ok(()));
    let clone_state = |state: &ThroughSupport| ThroughSupport {
        cfg: state.cfg.clone(), plan: state.plan.clone(), evaluation: state.evaluation.clone(), outputs: state.outputs.clone(),
    };
    let image = |state: &ThroughSupport| (state.cfg.clone(), state.plan.clone(), state.evaluation.clone(), state.outputs.clone());
    let a = ScalarType::Array { element: Box::new(ScalarType::U8), length: 2, length_span: ByteSpan::new(31, 32), span: ByteSpan::new(28, 33) };
    let b = ScalarType::Array { element: Box::new(ScalarType::U8), length: 2, length_span: ByteSpan::new(55, 56), span: ByteSpan::new(52, 57) };
    assert_ne!(a, b);
    assert!(scalar_type_equal(&a, &b));
    let reference = |ty: ScalarType| ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner: Box::new(ty) };
    let destination_indices = original.cfg.evaluated_values.shapes.iter().enumerate().filter(|(_, row)| row.id == original.evaluation.destinations[0].shape).map(|(index, _)| index).collect::<Vec<_>>();
    assert_eq!(destination_indices.len(), 1);
    let pointer = match &original.evaluation.destinations[0].storage {
        EvaluatedDestinationStorage::Through { pointer, .. } => pointer.clone(),
        _ => panic!("original Through destination"),
    };
    let value_indices = original.cfg.evaluated_values.values.iter().enumerate().filter(|(_, row)| row.id == pointer).map(|(index, _)| index).collect::<Vec<_>>();
    assert_eq!(value_indices.len(), 1);
    let pointer_indices = original.cfg.evaluated_values.shapes.iter().enumerate().filter(|(_, row)| row.id == original.cfg.evaluated_values.values[value_indices[0]].shape).map(|(index, _)| index).collect::<Vec<_>>();
    assert_eq!(pointer_indices.len(), 1);
    let destination_index = destination_indices[0];
    let pointer_index = pointer_indices[0];
    assert_ne!(destination_index, pointer_index);
    for site in [154, 245, 265] {
        let (destination_ty, output_ty, pointer_ty) = match site {
            154 => (a.clone(), reference(b.clone()), reference(b.clone())),
            245 => (b.clone(), reference(a.clone()), reference(a.clone())),
            265 => (b.clone(), reference(b.clone()), reference(a.clone())),
            _ => panic!("specified receiving site"),
        };
        let mut positive = clone_state(&original);
        positive.cfg.evaluated_values.shapes[destination_index].ty = destination_ty.clone();
        positive.plan.targets[0].ty = b.clone();
        positive.outputs[0].outputs.outputs[0].ty = output_ty.clone();
        positive.cfg.evaluated_values.shapes[pointer_index].ty = pointer_ty.clone();
        let mut expected_cfg = original.cfg.clone();
        expected_cfg.evaluated_values.shapes[destination_index].ty = destination_ty;
        expected_cfg.evaluated_values.shapes[pointer_index].ty = pointer_ty;
        let mut expected_plan = original.plan.clone();
        expected_plan.targets[0].ty = b.clone();
        let mut expected_outputs = original.outputs.clone();
        expected_outputs[0].outputs.outputs[0].ty = output_ty;
        let expected_positive = (expected_cfg, expected_plan, original.evaluation.clone(), expected_outputs);
        assert_eq!(image(&positive), expected_positive);
        assert_eq!(check(&positive), Ok(()));
        assert_eq!(image(&positive), expected_positive);
        let compared = match site {
            154 => (positive.cfg.evaluated_values.shapes[destination_index].ty.clone(), positive.plan.targets[0].ty.clone()),
            245 => (projected_type(&positive.plan.targets[0].place, &positive.outputs[0].outputs.outputs[0].ty, &[]).expect("actual projected type"), positive.plan.targets[0].ty.clone()),
            265 => (positive.cfg.evaluated_values.shapes[pointer_index].ty.clone(), positive.outputs[0].outputs.outputs[0].ty.clone()),
            _ => panic!("specified receiving site"),
        };
        assert_ne!(compared.0, compared.1);
        assert!(scalar_type_equal(&compared.0, &compared.1));
        if unequal_types {
            for unequal in [
                ScalarType::Array { element: Box::new(ScalarType::U8), length: 3, length_span: ByteSpan::new(55, 56), span: ByteSpan::new(52, 57) },
                ScalarType::Array { element: Box::new(ScalarType::U16), length: 2, length_span: ByteSpan::new(55, 56), span: ByteSpan::new(52, 57) },
            ] {
                assert!(!scalar_type_equal(&a, &unequal));
                assert!(!scalar_type_equal(&b, &unequal));
                let mut rejected = clone_state(&positive);
                let mut expected = image(&positive);
                let invariant = match site {
                    154 => {
                        rejected.plan.targets[0].ty = unequal.clone();
                        expected.1.targets[0].ty = unequal;
                        AssignmentCorrespondenceInvariant::Shape
                    }
                    245 => {
                        let ty = reference(unequal);
                        rejected.outputs[0].outputs.outputs[0].ty = ty.clone();
                        rejected.cfg.evaluated_values.shapes[pointer_index].ty = ty.clone();
                        expected.3[0].outputs.outputs[0].ty = ty.clone();
                        expected.0.evaluated_values.shapes[pointer_index].ty = ty;
                        AssignmentCorrespondenceInvariant::OperandType
                    }
                    265 => {
                        let ty = reference(unequal);
                        rejected.cfg.evaluated_values.shapes[pointer_index].ty = ty.clone();
                        expected.0.evaluated_values.shapes[pointer_index].ty = ty;
                        AssignmentCorrespondenceInvariant::OperandType
                    }
                    _ => panic!("specified receiving site"),
                };
                assert_eq!(image(&rejected), expected);
                assert_invariant(&rejected, invariant);
                assert_eq!(image(&rejected), expected);
            }
            let mut rejected = clone_state(&positive);
            let mut expected = image(&positive);
            rejected.outputs[0].outputs.outputs[0].ty = ScalarType::U8;
            expected.3[0].outputs.outputs[0].ty = ScalarType::U8;
            assert_eq!(projected_type(&rejected.plan.targets[0].place, &rejected.outputs[0].outputs.outputs[0].ty, &[]), None);
            assert_eq!(image(&rejected), expected);
            assert_invariant(&rejected, AssignmentCorrespondenceInvariant::OperandType);
            assert_eq!(image(&rejected), expected);
        }
        if corrupt_identity {
            for mutation in [0, 1, 2, 3] {
                let mut rejected = clone_state(&positive);
                let mut expected = image(&positive);
                let invariant = match mutation {
                    0 => {
                        rejected.evaluation.destinations[0].source_span.source = support_source("src/other.w");
                        expected.2.destinations[0].source_span.source = support_source("src/other.w");
                        AssignmentCorrespondenceInvariant::Source
                    }
                    1 => {
                        rejected.evaluation.destinations[0].shape.index = 100;
                        expected.2.destinations[0].shape.index = 100;
                        AssignmentCorrespondenceInvariant::Shape
                    }
                    2 => {
                        let ResolvedAssignmentStorage::Through { pointer } = &mut rejected.plan.targets[0].storage else { panic!("resolved Through pointer"); };
                        pointer.output = 1;
                        let ResolvedAssignmentStorage::Through { pointer } = &mut expected.1.targets[0].storage else { panic!("expected Through pointer"); };
                        pointer.output = 1;
                        AssignmentCorrespondenceInvariant::PointerIdentity
                    }
                    3 => {
                        let invalid = CfgPointId(rejected.cfg.points.len());
                        let EvaluatedDestinationStorage::Through { pointer, .. } = &mut rejected.evaluation.destinations[0].storage else { panic!("evaluated Through pointer"); };
                        pointer.point = invalid;
                        rejected.cfg.evaluated_values.values[value_indices[0]].id = pointer.clone();
                        let CfgPointKind::AssignmentDestination { storage, .. } = &mut rejected.cfg.points[rejected.evaluation.destinations[0].point.0].kind else { panic!("destination event"); };
                        *storage = rejected.evaluation.destinations[0].storage.clone();
                        let EvaluatedDestinationStorage::Through { pointer, .. } = &mut expected.2.destinations[0].storage else { panic!("expected evaluated pointer"); };
                        pointer.point = invalid;
                        expected.0.evaluated_values.values[value_indices[0]].id = pointer.clone();
                        let CfgPointKind::AssignmentDestination { storage, .. } = &mut expected.0.points[expected.2.destinations[0].point.0].kind else { panic!("expected destination event"); };
                        *storage = expected.2.destinations[0].storage.clone();
                        AssignmentCorrespondenceInvariant::ProducerPoint
                    }
                    _ => panic!("specified identity mutation"),
                };
                assert_eq!(image(&rejected), expected);
                assert_eq!(rejected.outputs, positive.outputs);
                assert_eq!(rejected.plan.targets[0].ty, positive.plan.targets[0].ty);
                assert_eq!(rejected.cfg.evaluated_values.shapes, positive.cfg.evaluated_values.shapes);
                assert_invariant(&rejected, invariant);
                assert_eq!(image(&rejected), expected);
            }
        }
        assert_eq!(image(&positive), expected_positive);
        assert_eq!(image(&original), original_state);
    }
    assert_eq!(image(&original), original_state);
    assert_eq!(source, original_source);
}

#[test]
fn assignment_semantic_array_agreement_destination_spans_native64() { prove_assignment_semantic_array_boundary(ScalarTargetLayout::NATIVE64, false, false); }
#[test]
fn assignment_semantic_array_agreement_destination_spans_wasm32() { prove_assignment_semantic_array_boundary(ScalarTargetLayout::WASM32, false, false); }
#[test]
fn assignment_semantic_array_agreement_rejects_unequal_types_native64() { prove_assignment_semantic_array_boundary(ScalarTargetLayout::NATIVE64, true, false); }
#[test]
fn assignment_semantic_array_agreement_rejects_unequal_types_wasm32() { prove_assignment_semantic_array_boundary(ScalarTargetLayout::WASM32, true, false); }
#[test]
fn assignment_semantic_array_agreement_preserves_identity_native64() { prove_assignment_semantic_array_boundary(ScalarTargetLayout::NATIVE64, false, true); }
#[test]
fn assignment_semantic_array_agreement_preserves_identity_wasm32() { prove_assignment_semantic_array_boundary(ScalarTargetLayout::WASM32, false, true); }
