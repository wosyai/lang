use super::*;

fn source(path: &str, revision: &str) -> SourceIdentity {
    SourceIdentity::new("finite".into(), "app".into(), path.into(), revision.into())
}

fn point(
    source: &SourceIdentity,
    index: usize,
    kind: CfgPointKind,
    successors: Vec<CfgPointId>,
) -> ReferenceFlowPoint {
    ReferenceFlowPoint {
        id: CfgPointId(index),
        kind,
        source_span: SourceSpan::new(
            source.clone(),
            ByteSpan::new(index as u32, index as u32 + 1),
        ),
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

fn cfg(path: &str, revision: &str) -> ScalarReferenceCfg {
    let source = source(path, revision);
    let mut cfg = ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(source.clone()));
    cfg.initial_points = vec![
        point(
            &source,
            0,
            CfgPointKind::Entry,
            vec![CfgPointId(2), CfgPointId(1)],
        ),
        point(&source, 1, CfgPointKind::ScopeEnter, vec![CfgPointId(2)]),
        point(&source, 2, CfgPointKind::Return, vec![]),
    ];
    let shape = ValueShapeId {
        producer: cfg.producer_owner.clone(),
        index: 11,
    };
    cfg.evaluated_values.shapes.push(ResolvedValueShape {
        id: shape.clone(),
        ty: ScalarType::U8,
        payload: ValuePayloadShape::Plain(PlainValueKind::U8),
    });
    cfg.evaluated_values.values.push(EvaluatedValueFact {
        id: EvaluatedValueId {
            resolved: ResolvedValueOutputId {
                producer: cfg.producer_owner.clone(),
                occurrence: SourceSpan::new(source, ByteSpan::new(20, 25)),
                output_projection: vec![3, 1],
                output: 2,
            },
            point: CfgPointId(2),
        },
        shape,
        cases: Vec::new(),
    });
    cfg
}

fn event(cfg: &ScalarReferenceCfg, index: usize) -> SourceEventId {
    SourceEventId {
        owner: cfg.producer_owner.clone(),
        point: CfgPointId(index),
    }
}

fn shape(cfg: &ScalarReferenceCfg, index: usize) -> ValueShapeId {
    ValueShapeId {
        producer: cfg.producer_owner.clone(),
        index,
    }
}

fn callable(
    cfg: &ScalarReferenceCfg,
    output: ScalarType,
    parameter: ScalarType,
) -> ResolvedValueShape {
    let signature = ScalarType::Callable {
        outputs: ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty: output,
                span: ByteSpan::new(30, 32),
            }],
            span: ByteSpan::new(30, 32),
        },
        parameters: vec![parameter],
    };
    ResolvedValueShape {
        id: shape(cfg, 40),
        ty: signature.clone(),
        payload: ValuePayloadShape::Callable { signature },
    }
}

#[test]
fn finite_source_epoch_qualified_events_preserve_original_order_and_spans() {
    let first = cfg("src/first.w", "r1");
    let second = cfg("src/second.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[first.producer_owner.clone(), second.producer_owner.clone()], &[], &[]);
    let epoch = SourceEpoch::freeze(&[&second, &first], &[], &[], &contexts).unwrap();
    assert_eq!(epoch.events().len(), 6);
    assert_eq!(
        epoch.events()[0],
        SourceEvent {
            id: event(&second, 0),
            source_span: second.initial_points[0].source_span.clone(),
            kind: SourceEventKind::Entry,
            successors: vec![event(&second, 2), event(&second, 1)],
        }
    );
    assert_eq!(epoch.events()[3].id, event(&first, 0));
    assert_ne!(epoch.events()[0].id, epoch.events()[3].id);
    assert_eq!(epoch.events()[1].kind, SourceEventKind::ScopeEnter);
    assert_eq!(epoch.events()[2].kind, SourceEventKind::Return);
    assert_eq!(epoch.shapes()[0], second.evaluated_values.shapes[0]);
    assert_eq!(
        epoch.outputs()[0],
        SourceOutputPort {
            value: second.evaluated_values.values[0].id.clone(),
            shape: second.evaluated_values.values[0].shape.clone(),
        }
    );
}

#[test]
fn finite_source_epoch_revisions_keep_events_shapes_and_outputs_distinct() {
    let first = cfg("src/main.w", "r1");
    let second = cfg("src/main.w", "r2");
    let contexts = evaluated_value::template::concrete_contexts(&[first.producer_owner.clone(), second.producer_owner.clone()], &[], &[]);
    let epoch = SourceEpoch::freeze(&[&first, &second], &[], &[], &contexts).unwrap();
    assert_ne!(epoch.events()[0].id, epoch.events()[3].id);
    assert_ne!(epoch.shapes()[0].id, epoch.shapes()[1].id);
    assert_ne!(epoch.outputs()[0].value, epoch.outputs()[1].value);
    assert_eq!(
        epoch.outputs()[1].value.resolved.occurrence,
        second.evaluated_values.values[0].id.resolved.occurrence
    );
    assert_eq!(
        epoch.outputs()[1].value.resolved.output_projection,
        vec![3, 1]
    );
}

#[test]
fn finite_source_epoch_runtime_observations_and_output_cases_supply_zero_rows() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    let frozen = SourceEpoch::freeze(&[&cfg], &[], &[], &contexts).unwrap();
    let source = cfg.initial_points[0].source_span.source.clone();
    cfg.points.push(point(
        &source,
        99,
        CfgPointKind::Join,
        vec![CfgPointId(800)],
    ));
    cfg.facts.push(ReferenceOrigin::Null {
        span: ByteSpan::new(90, 91),
    });
    cfg.environments.push(CallableEnvironmentFact {
        literal: SourceSpan::new(source.clone(), ByteSpan::new(90, 99)),
        instance: CallableEnvironmentInstance::Capture,
        origins: BTreeSet::from([ReferenceOriginId(0)]),
        owners: BTreeSet::new(),
        loans: BTreeSet::new(),
        backings: Vec::new(),
        inputs: Vec::new(),
        capture_paths: Vec::new(),
    });
    cfg.initial_points[0]
        .possible_environments
        .insert(CallableEnvironmentId {
            literal: SourceSpan::new(source, ByteSpan::new(90, 99)),
            index: 800,
        });
    let value = &mut cfg.evaluated_values.values[0];
    value.cases.push(EvaluatedValueCase {
        id: ValueCaseId {
            value: value.id.clone(),
            alternative: 901,
        },
        path: EvaluatedControlPathId {
            producer: cfg.producer_owner.clone(),
            ordinal: 900,
        },
        availability: ValueAvailability::Available,
        payload: EvaluatedPayloadFact::Plain,
        dependencies: Vec::new(),
    });
    assert_eq!(SourceEpoch::freeze(&[&cfg], &[], &[], &contexts).unwrap(), frozen);
}

#[test]
fn finite_source_epoch_sparse_child_before_parent_and_imported_shape_accepted() {
    let mut first = cfg("src/first.w", "r1");
    let second = cfg("src/second.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[first.producer_owner.clone(), second.producer_owner.clone()], &[], &[]);
    let parent = shape(&first, 4);
    first.evaluated_values.shapes.push(ResolvedValueShape {
        id: parent.clone(),
        ty: ScalarType::RawPointer(Box::new(ScalarType::U8)),
        payload: ValuePayloadShape::Raw {
            pointee: shape(&second, 11),
        },
    });
    first.evaluated_values.values[0].shape = parent.clone();
    let epoch = SourceEpoch::freeze(&[&first, &second], &[], &[], &contexts).unwrap();
    assert_eq!(
        epoch
            .shapes()
            .iter()
            .map(|row| row.id.index)
            .collect::<Vec<_>>(),
        vec![11, 4, 11]
    );
    assert_eq!(epoch.outputs()[0].shape, parent);
    first.evaluated_values.values[0].shape = shape(&second, 11);
    assert_eq!(
        SourceEpoch::freeze(&[&first, &second], &[], &[], &contexts)
            .unwrap()
            .outputs()[0]
            .shape,
        shape(&second, 11)
    );
}

#[test]
fn finite_source_epoch_cyclic_graph_edges_stay_finite() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    let structure = ScalarStructId {
        source: cfg.initial_points[0].source_span.source.clone(),
        index: 2,
    };
    cfg.evaluated_values.shapes.push(ResolvedValueShape {
        id: shape(&cfg, 70),
        ty: ScalarType::Struct(structure.clone()),
        payload: ValuePayloadShape::Struct {
            structure: structure.clone(),
            fields: vec![ValueShapeField {
                field: ScalarStructFieldId {
                    structure,
                    index: 0,
                },
                shape: shape(&cfg, 90),
            }],
        },
    });
    cfg.evaluated_values.shapes.push(ResolvedValueShape {
        id: shape(&cfg, 90),
        ty: ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Shared,
            inner: Box::new(cfg.evaluated_values.shapes[1].ty.clone()),
        },
        payload: ValuePayloadShape::Checked {
            mutability: ScalarReferenceMutability::Shared,
            referent: shape(&cfg, 70),
        },
    });
    let epoch = SourceEpoch::freeze(&[&cfg], &[], &[], &contexts).unwrap();
    assert_eq!(epoch.shapes(), cfg.evaluated_values.shapes);
    assert_eq!(epoch.shapes().len(), 3);
}

#[test]
fn finite_source_epoch_duplicate_owner_rejected() {
    let cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    assert_eq!(
        SourceEpoch::freeze(&[&cfg, &cfg], &[], &[], &contexts),
        Err(SourceEpochError::DuplicateOwner(cfg.producer_owner.clone()))
    );
}

#[test]
fn finite_source_epoch_invalid_original_point_id_rejected() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.initial_points[1].id = CfgPointId(20);
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::InvalidOriginalPointId {
            event: event(&cfg, 20),
            expected: CfgPointId(1)
        })
    );
}

#[test]
fn finite_source_epoch_missing_original_successor_rejected_despite_runtime_point() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    let source = cfg.initial_points[0].source_span.source.clone();
    cfg.points
        .push(point(&source, 3, CfgPointKind::Return, vec![]));
    cfg.initial_points[0].successors.push(CfgPointId(3));
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::MissingOriginalSuccessor {
            event: event(&cfg, 0),
            successor: event(&cfg, 3)
        })
    );
}

#[test]
fn finite_source_epoch_duplicate_shape_equal_rows_rejected_globally() {
    let first = cfg("src/first.w", "r1");
    let mut second = cfg("src/second.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[first.producer_owner.clone(), second.producer_owner.clone()], &[], &[]);
    second
        .evaluated_values
        .shapes
        .push(first.evaluated_values.shapes[0].clone());
    assert_eq!(
        SourceEpoch::freeze(&[&first, &second], &[], &[], &contexts),
        Err(SourceEpochError::DuplicateShape(shape(&first, 11)))
    );
}

#[test]
fn finite_source_epoch_duplicate_output_equal_rows_rejected_globally() {
    let first = cfg("src/first.w", "r1");
    let mut second = cfg("src/second.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[first.producer_owner.clone(), second.producer_owner.clone()], &[], &[]);
    second
        .evaluated_values
        .values
        .push(first.evaluated_values.values[0].clone());
    assert_eq!(
        SourceEpoch::freeze(&[&first, &second], &[], &[], &contexts),
        Err(SourceEpochError::DuplicateOutput(
            first.evaluated_values.values[0].id.clone()
        ))
    );
}

#[test]
fn finite_source_epoch_foreign_output_owner_rejected() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.evaluated_values.values[0].id.resolved.producer =
        ProducerCfgOwnerId::Module(source("src/foreign.w", "r1"));
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::ForeignOutputOwner {
            value: cfg.evaluated_values.values[0].id.clone(),
            owner: cfg.producer_owner.clone()
        })
    );
}

#[test]
fn finite_source_epoch_foreign_output_source_rejected() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.evaluated_values.values[0].id.resolved.occurrence.source = source("src/main.w", "r2");
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::ForeignOutputSource {
            value: cfg.evaluated_values.values[0].id.clone(),
            event: event(&cfg, 2),
            source: cfg.initial_points[2].source_span.source.clone(),
        })
    );
}

#[test]
fn finite_source_epoch_missing_output_event_rejected_despite_runtime_point() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.evaluated_values.values[0].id.point = CfgPointId(3);
    let source = cfg.initial_points[0].source_span.source.clone();
    cfg.points
        .push(point(&source, 3, CfgPointKind::Return, vec![]));
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::MissingOutputEvent {
            value: cfg.evaluated_values.values[0].id.clone(),
            event: event(&cfg, 3)
        })
    );
}

#[test]
fn finite_source_epoch_missing_output_shape_rejected() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.evaluated_values.values[0].shape = shape(&cfg, 99);
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::MissingOutputShape {
            value: cfg.evaluated_values.values[0].id.clone(),
            shape: shape(&cfg, 99)
        })
    );
}

#[test]
fn finite_source_epoch_every_payload_child_reference_validated() {
    let cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    let structure = ScalarStructId {
        source: cfg.initial_points[0].source_span.source.clone(),
        index: 0,
    };
    let enumeration = ScalarEnumId {
        source: cfg.initial_points[0].source_span.source.clone(),
        index: 0,
    };
    let target = shape(&cfg, 99);
    let payloads = vec![
        ValuePayloadShape::Checked {
            mutability: ScalarReferenceMutability::Shared,
            referent: target.clone(),
        },
        ValuePayloadShape::Raw {
            pointee: target.clone(),
        },
        ValuePayloadShape::Struct {
            structure: structure.clone(),
            fields: vec![ValueShapeField {
                field: ScalarStructFieldId {
                    structure,
                    index: 0,
                },
                shape: target.clone(),
            }],
        },
        ValuePayloadShape::Enum {
            enumeration,
            variants: vec![ValueShapeVariant {
                ordinal: 7,
                fields: vec![target.clone()],
            }],
        },
        ValuePayloadShape::FixedArray {
            element: target.clone(),
            length: 2,
        },
        ValuePayloadShape::RuntimeArray {
            element: target.clone(),
        },
    ];
    for payload in payloads {
        let mut missing = cfg.clone();
        missing.evaluated_values.shapes[0].ty = match &payload {
            ValuePayloadShape::TemplateParameter { .. } => missing.evaluated_values.shapes[0].ty.clone(),
            ValuePayloadShape::Checked { mutability, .. } => ScalarType::CheckedReference {
                mutability: *mutability,
                inner: Box::new(ScalarType::Bool),
            },
            ValuePayloadShape::Raw { .. } => ScalarType::RawPointer(Box::new(ScalarType::Bool)),
            ValuePayloadShape::Struct { structure, .. } => ScalarType::Struct(structure.clone()),
            ValuePayloadShape::Enum { enumeration, .. } => ScalarType::Enum(enumeration.clone()),
            ValuePayloadShape::FixedArray { length, .. } => ScalarType::Array {
                element: Box::new(ScalarType::Bool),
                length: *length,
                length_span: ByteSpan::new(4, 5),
                span: ByteSpan::new(0, 6),
            },
            ValuePayloadShape::RuntimeArray { .. } => ScalarType::RuntimeArray {
                element: Box::new(ScalarType::Bool),
                span: ByteSpan::new(0, 6),
            },
            ValuePayloadShape::Plain(_) | ValuePayloadShape::Callable { .. } => {
                panic!("child fixture payload")
            }
        };
        missing.evaluated_values.shapes[0].payload = payload;
        assert_eq!(
            SourceEpoch::freeze(&[&missing], &[], &[], &contexts),
            Err(SourceEpochError::MissingShapeReference {
                shape: shape(&cfg, 11),
                target: target.clone()
            })
        );
        missing.evaluated_values.shapes.push(ResolvedValueShape {
            id: target.clone(),
            ty: ScalarType::Bool,
            payload: ValuePayloadShape::Plain(PlainValueKind::Bool),
        });
        assert_eq!(
            SourceEpoch::freeze(&[&missing], &[], &[], &contexts).unwrap().shapes(),
            missing.evaluated_values.shapes
        );
    }
}

#[test]
fn finite_source_epoch_unresolved_original_shape_types_rejected() {
    let cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    let types = vec![
        ScalarType::Named {
            name: "Missing".into(),
            span: ByteSpan::new(3, 10),
        },
        ScalarType::Qualified {
            receiver: "dep".into(),
            receiver_span: ByteSpan::new(3, 6),
            member: "Missing".into(),
            member_span: ByteSpan::new(7, 14),
            span: ByteSpan::new(3, 14),
        },
        ScalarType::Error,
    ];
    for ty in types {
        let mut unresolved = cfg.clone();
        unresolved.evaluated_values.shapes[0].ty = ty.clone();
        assert_eq!(
            SourceEpoch::freeze(&[&unresolved], &[], &[], &contexts),
            Err(SourceEpochError::UnresolvedShapeType {
                shape: shape(&cfg, 11),
                ty
            })
        );
    }
}

#[test]
fn finite_source_epoch_callable_ports_resolve_after_all_rows_collected() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.evaluated_values
        .shapes
        .insert(0, callable(&cfg, ScalarType::U8, ScalarType::U8));
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts).unwrap().shapes(),
        cfg.evaluated_values.shapes
    );
}

#[test]
fn finite_source_epoch_missing_callable_output_and_parameter_types_rejected() {
    let cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    for (output, parameter) in [
        (ScalarType::Bool, ScalarType::U8),
        (ScalarType::U8, ScalarType::Bool),
    ] {
        let mut missing = cfg.clone();
        missing
            .evaluated_values
            .shapes
            .push(callable(&cfg, output, parameter));
        assert_eq!(
            SourceEpoch::freeze(&[&missing], &[], &[], &contexts),
            Err(SourceEpochError::MissingCallableSignatureType {
                shape: shape(&cfg, 40),
                ty: ScalarType::Bool
            })
        );
    }
}

#[test]
fn finite_source_epoch_callable_type_matching_is_producer_qualified() {
    let mut first = cfg("src/first.w", "r1");
    let mut second = cfg("src/second.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[first.producer_owner.clone(), second.producer_owner.clone()], &[], &[]);
    second.evaluated_values.shapes[0].ty = ScalarType::Bool;
    second.evaluated_values.shapes[0].payload = ValuePayloadShape::Plain(PlainValueKind::Bool);
    first
        .evaluated_values
        .shapes
        .push(callable(&first, ScalarType::U8, ScalarType::Bool));
    assert_eq!(
        SourceEpoch::freeze(&[&first, &second], &[], &[], &contexts),
        Err(SourceEpochError::MissingCallableSignatureType {
            shape: shape(&first, 40),
            ty: ScalarType::Bool
        })
    );
}

#[test]
fn finite_source_epoch_ambiguous_callable_signature_type_rejected() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    let mut duplicate_type = cfg.evaluated_values.shapes[0].clone();
    duplicate_type.id = shape(&cfg, 12);
    cfg.evaluated_values.shapes.push(duplicate_type);
    cfg.evaluated_values
        .shapes
        .push(callable(&cfg, ScalarType::U8, ScalarType::U8));
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::AmbiguousCallableSignatureType {
            shape: shape(&cfg, 40),
            ty: ScalarType::U8,
            matches: vec![shape(&cfg, 11), shape(&cfg, 12)],
        })
    );
}

#[test]
fn finite_source_epoch_invalid_callable_signature_has_typed_outcome() {
    let mut cfg = cfg("src/main.w", "r1");
    let contexts = evaluated_value::template::concrete_contexts(&[cfg.producer_owner.clone()], &[], &[]);
    cfg.evaluated_values.shapes[0].payload = ValuePayloadShape::Callable {
        signature: ScalarType::U8,
    };
    assert_eq!(
        SourceEpoch::freeze(&[&cfg], &[], &[], &contexts),
        Err(SourceEpochError::InvalidCallableSignature {
            shape: shape(&cfg, 11),
            signature: ScalarType::U8
        })
    );
}

#[test]
fn finite_source_epoch_empty_complete_list_is_empty() {
    let epoch = SourceEpoch::freeze(&[], &[], &[], &[]).unwrap();
    assert!(epoch.events().is_empty());
    assert!(epoch.shapes().is_empty());
    assert!(epoch.outputs().is_empty());
}
