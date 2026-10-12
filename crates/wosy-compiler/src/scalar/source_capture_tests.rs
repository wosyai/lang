use super::*;

struct CaptureFixture {
    parent: ScalarReferenceCfg,
    literal: ScalarReferenceCfg,
    structs: Vec<ScalarStruct>,
    enums: Vec<ScalarEnum>,
}

fn point(
    source_span: SourceSpan,
    index: usize,
    kind: CfgPointKind,
    successors: Vec<CfgPointId>,
) -> ReferenceFlowPoint {
    ReferenceFlowPoint {
        id: CfgPointId(index),
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

fn binding(source: &SourceIdentity, declaration: u32) -> ReferenceBindingId {
    ReferenceBindingId {
        source: source.clone(),
        function_span: ByteSpan::new(0, 100),
        declaration_span: ByteSpan::new(declaration, declaration + 1),
        block_span: ByteSpan::new(0, 100),
        kind: ReferenceBindingKind::Declared,
    }
}

fn fixture(package: &str, revision: &str) -> CaptureFixture {
    let source = SourceIdentity::new(
        "finite".into(),
        package.into(),
        "src/main.w".into(),
        revision.into(),
    );
    let literal_span = SourceSpan::new(source.clone(), ByteSpan::new(20, 90));
    let structure = ScalarStructId {
        source: source.clone(),
        index: 0,
    };
    let field = ScalarStructFieldId {
        structure: structure.clone(),
        index: 0,
    };
    let captures = [
        ("plain", ScalarType::U8, ScalarType::U8),
        (
            "checked",
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Shared,
                inner: Box::new(ScalarType::U8),
            },
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Shared,
                inner: Box::new(ScalarType::U8),
            },
        ),
        (
            "record",
            ScalarType::Struct(structure.clone()),
            ScalarType::Bool,
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, (name, ty, place_type))| {
        let base = ScalarPlace::Name {
            name: name.into(),
            span: ByteSpan::new(50 + ordinal as u32, 51 + ordinal as u32),
        };
        let place = match ordinal {
            2 => ScalarPlace::Field {
                base: Box::new(base),
                field: ScalarFieldReference::Resolved(field.clone()),
                span: ByteSpan::new(52, 59),
            },
            _ => base,
        };
        closure::ClosureCapture {
            name: name.into(),
            binding: binding(&source, 2 + ordinal as u32),
            ty,
            place_type,
            use_span: SourceSpan::new(
                source.clone(),
                ByteSpan::new(50 + ordinal as u32, 60 + ordinal as u32),
            ),
            place,
        }
    })
    .collect::<Vec<_>>();
    let checked = &captures[1];
    let storage = ReferencePlaceId {
        source: checked.binding.source.clone(),
        function_span: checked.binding.function_span,
        declaration_span: checked.binding.declaration_span,
        block_span: checked.binding.block_span,
        binding: checked.binding.clone(),
        projections: Vec::new(),
        place: checked.place.clone(),
    };
    let body_span = ByteSpan::new(30, 89);
    let body = ScalarBlock {
        items: Vec::new(),
        terminated_items: Vec::new(),
        expressions: Vec::new(),
        final_output_values: Vec::new(),
        span: body_span,
        occurrence: Box::new(ScalarBlockOccurrence {
            source_span: SourceSpan::new(source.clone(), body_span),
            output_projection: Vec::new(),
        }),
        unsafe_context: false,
    };
    let signature = ScalarType::Callable {
        outputs: ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty: ScalarType::Unit,
                span: ByteSpan::new(20, 24),
            }],
            span: ByteSpan::new(20, 24),
        },
        parameters: vec![ScalarType::U8, ScalarType::Bool],
    };
    let mut parent = ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(source.clone()));
    parent
        .checked_bindings
        .insert(checked.binding.clone(), storage);
    parent.initial_points = vec![
        point(
            SourceSpan::new(source.clone(), ByteSpan::new(0, 100)),
            0,
            CfgPointKind::Entry,
            vec![CfgPointId(1)],
        ),
        point(
            literal_span.clone(),
            1,
            CfgPointKind::Capture {
                literal: literal_span.range,
                environment: CallableEnvironmentId {
                    literal: literal_span.clone(),
                    index: 987,
                },
                signature,
                captures,
                parameters: vec!["first".into(), "second".into()],
                parameter_spans: vec![ByteSpan::new(25, 26), ByteSpan::new(27, 28)],
                body,
            },
            vec![],
        ),
    ];
    parent.evaluated_values.shapes.push(ResolvedValueShape {
        id: ValueShapeId {
            producer: parent.producer_owner.clone(),
            index: 11,
        },
        ty: ScalarType::U8,
        payload: ValuePayloadShape::Plain(PlainValueKind::U8),
    });
    let mut literal = ScalarReferenceCfg::new(ProducerCfgOwnerId::Literal {
        enclosing: Box::new(parent.producer_owner.clone()),
        capture_point: CfgPointId(1),
        literal: literal_span.clone(),
    });
    literal
        .initial_points
        .push(point(literal_span, 0, CfgPointKind::Entry, vec![]));
    CaptureFixture {
        parent,
        literal,
        structs: vec![ScalarStruct {
            id: structure,
            name: "Record".into(),
            name_span: ByteSpan::new(5, 11),
            fields: vec![ScalarStructField {
                id: field,
                name: "flag".into(),
                name_span: ByteSpan::new(12, 16),
                ty: ScalarType::Bool,
                declaration_index: 0,
                offset: None,
                layout: None,
            }],
            copy_policy: CopyPolicy::Copy,
            copy_modifier_span: None,
            span: ByteSpan::new(5, 19),
            layout: None,
        }],
        enums: Vec::new(),
    }
}

fn original_contexts(fixture: &CaptureFixture) -> Vec<SourceCapturePolicyContext> {
    evaluated_value::template::concrete_contexts(&[fixture.parent.producer_owner.clone()], &fixture.structs, &fixture.enums)
}

fn freeze(fixture: &CaptureFixture, contexts: &[SourceCapturePolicyContext]) -> Result<SourceEpoch, SourceEpochError> {
    SourceEpoch::freeze(
        &[&fixture.parent, &fixture.literal],
        &fixture.structs,
        &fixture.enums, contexts,
    )
}

fn event(fixture: &CaptureFixture) -> SourceEventId {
    SourceEventId {
        owner: fixture.parent.producer_owner.clone(),
        point: CfgPointId(1),
    }
}

fn captures(fixture: &CaptureFixture) -> &[closure::ClosureCapture] {
    match &fixture.parent.initial_points[1].kind {
        CfgPointKind::Capture { captures, .. } => captures,
        _ => panic!("Capture fixture"),
    }
}

fn shape_row<'a>(epoch: &'a SourceEpoch, id: &ValueShapeId) -> &'a ResolvedValueShape {
    epoch
        .shapes()
        .iter()
        .find(|row| row.id == *id)
        .expect("actual shape row")
}

#[test]
fn finite_source_capture_ordered_source_ports_and_parameter_bindings() {
    let fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let epoch = freeze(&fixture, &contexts).unwrap();
    let template = &epoch.captures()[0];
    assert_eq!(epoch.captures().len(), 1);
    assert_eq!(template.event, event(&fixture));
    assert_eq!(template.literal_owner, fixture.literal.producer_owner);
    assert_eq!(
        template.literal_span,
        fixture.parent.initial_points[1].source_span
    );
    assert_eq!(template.inputs.len(), 3);
    for (ordinal, (port, capture)) in template.inputs.iter().zip(captures(&fixture)).enumerate() {
        assert_eq!(port.ordinal, ordinal);
        assert_eq!(port.name, capture.name);
        assert_eq!(port.binding, capture.binding);
        assert_eq!(port.use_span, capture.use_span);
        assert_eq!(port.place, capture.place);
        assert_eq!(shape_row(&epoch, &port.value_shape).ty, capture.ty);
        assert_eq!(shape_row(&epoch, &port.place_shape).ty, capture.place_type);
        assert_eq!(shape_row(&epoch, &port.formal_shape).ty, capture.ty);
        assert_eq!(port.value_shape.producer, fixture.parent.producer_owner);
        assert_eq!(port.formal_shape.producer, fixture.literal.producer_owner);
    }
    assert_eq!(
        template.inputs[0].storage,
        SourceCaptureStorage::Binding(captures(&fixture)[0].binding.clone())
    );
    assert_eq!(
        template.inputs[1].storage,
        SourceCaptureStorage::CheckedSlot(captures(&fixture)[1].binding.clone())
    );
    assert_ne!(
        template.inputs[2].value_shape,
        template.inputs[2].place_shape
    );
    let CfgPointKind::Capture {
        signature,
        parameters,
        parameter_spans,
        body,
        ..
    } = &fixture.parent.initial_points[1].kind
    else {
        panic!("Capture fixture")
    };
    assert_eq!(shape_row(&epoch, &template.signature).ty, *signature);
    assert_eq!(template.parameters.len(), 2);
    for (ordinal, port) in template.parameters.iter().enumerate() {
        assert_eq!(port.ordinal, ordinal);
        assert_eq!(port.name, parameters[ordinal]);
        assert_eq!(
            port.binding,
            ReferenceBindingId {
                source: template.literal_span.source.clone(),
                function_span: template.literal_span.range,
                declaration_span: parameter_spans[ordinal],
                block_span: body.span,
                kind: ReferenceBindingKind::Declared,
            }
        );
        assert_eq!(port.shape.producer, fixture.literal.producer_owner);
    }
    assert_eq!(
        shape_row(&epoch, &template.parameters[0].shape).ty,
        ScalarType::U8
    );
    assert_eq!(
        shape_row(&epoch, &template.parameters[1].shape).ty,
        ScalarType::Bool
    );
    assert_eq!(template.inputs[0].value_shape.index, 11);
    assert_eq!(epoch.shapes()[0], fixture.parent.evaluated_values.shapes[0]);
    assert!(epoch.outputs().is_empty());
}

#[test]
fn finite_source_capture_struct_enum_and_cyclic_graph_use_canonical_interner() {
    let mut fixture = fixture("app", "r1");
    let structure = fixture.structs[0].id.clone();
    fixture.structs[0].fields.push(ScalarStructField {
        id: ScalarStructFieldId {
            structure: structure.clone(),
            index: 1,
        },
        name: "next".into(),
        name_span: ByteSpan::new(17, 21),
        ty: ScalarType::RawPointer(Box::new(ScalarType::Struct(structure))),
        declaration_index: 1,
        offset: None,
        layout: None,
    });
    let enumeration = ScalarEnumId {
        source: fixture.structs[0].id.source.clone(),
        index: 0,
    };
    fixture.enums.push(ScalarEnum {
        id: enumeration.clone(),
        name: "State".into(),
        name_span: ByteSpan::new(101, 106),
        variants: vec![ScalarEnumVariant {
            name: "ready".into(),
            name_span: ByteSpan::new(107, 112),
            tag: 7,
        }],
        copy_policy: CopyPolicy::Copy,
        copy_modifier_span: None,
        span: ByteSpan::new(101, 113),
    });
    let CfgPointKind::Capture {
        captures: inputs, ..
    } = &mut fixture.parent.initial_points[1].kind
    else {
        panic!("Capture fixture")
    };
    inputs[0].ty = ScalarType::Enum(enumeration.clone());
    inputs[0].place_type = ScalarType::Enum(enumeration.clone());
    let contexts = original_contexts(&fixture);
    let epoch = freeze(&fixture, &contexts).unwrap();
    let template = &epoch.captures()[0];
    assert!(
        matches!(shape_row(&epoch, &template.inputs[0].value_shape).payload, ValuePayloadShape::Enum { ref enumeration, .. } if *enumeration == fixture.enums[0].id)
    );
    for id in [
        &template.inputs[2].value_shape,
        &template.inputs[2].formal_shape,
    ] {
        let ValuePayloadShape::Struct { fields, .. } = &shape_row(&epoch, id).payload else {
            panic!("Struct shape")
        };
        let ValuePayloadShape::Raw { pointee } = &shape_row(&epoch, &fields[1].shape).payload
        else {
            panic!("Raw shape")
        };
        assert_eq!(pointee, id);
    }
    let mut canonical = fixture.parent.evaluated_values.shapes.clone();
    let ty = &captures(&fixture)[2].ty;
    let expected = intern_value_shape(
        &mut canonical,
        &fixture.parent.producer_owner,
        ty,
        &fixture.structs,
        &fixture.enums, &fixture.parent.producer_owner, &[],
    )
    .unwrap();
    assert_eq!(
        shape_row(&epoch, &template.inputs[2].value_shape).ty,
        canonical.iter().find(|row| row.id == expected).unwrap().ty
    );
}

#[test]
fn finite_source_capture_shared_binding_sibling_events_have_distinct_formal_owners() {
    let mut fixture = fixture("app", "r1");
    let mut sibling_point = fixture.parent.initial_points[1].clone();
    sibling_point.id = CfgPointId(2);
    sibling_point.source_span.range = ByteSpan::new(120, 190);
    let CfgPointKind::Capture {
        literal,
        environment,
        ..
    } = &mut sibling_point.kind
    else {
        panic!("Capture fixture")
    };
    *literal = sibling_point.source_span.range;
    environment.literal = sibling_point.source_span.clone();
    fixture.parent.initial_points.push(sibling_point.clone());
    let mut sibling = ScalarReferenceCfg::new(ProducerCfgOwnerId::Literal {
        enclosing: Box::new(fixture.parent.producer_owner.clone()),
        capture_point: CfgPointId(2),
        literal: sibling_point.source_span.clone(),
    });
    sibling.initial_points.push(point(
        sibling_point.source_span,
        0,
        CfgPointKind::Entry,
        vec![],
    ));
    let epoch = SourceEpoch::freeze(
        &[&fixture.parent, &sibling, &fixture.literal],
        &fixture.structs,
        &fixture.enums, &original_contexts(&fixture),
    )
    .unwrap();
    assert_eq!(epoch.captures().len(), 2);
    for (first, second) in epoch.captures()[0]
        .inputs
        .iter()
        .zip(&epoch.captures()[1].inputs)
    {
        assert_eq!(first.binding, second.binding);
        assert_eq!(first.value_shape, second.value_shape);
        assert_ne!(first.formal_shape, second.formal_shape);
    }
    assert_ne!(epoch.captures()[0].event, epoch.captures()[1].event);
    assert_ne!(
        epoch.captures()[0].literal_owner,
        epoch.captures()[1].literal_owner
    );
}

#[test]
fn finite_source_capture_revision_and_package_qualify_identical_local_labels() {
    let first = fixture("app", "r1");
    for second in [fixture("app", "r2"), fixture("dep", "r1")] {
        let mut structs = first.structs.clone();
        structs.extend(second.structs.clone());
        let contexts = original_contexts(&first).into_iter().chain(original_contexts(&second)).collect::<Vec<_>>();
        let epoch = SourceEpoch::freeze(
            &[
                &first.parent,
                &first.literal,
                &second.parent,
                &second.literal,
            ],
            &structs,
            &[], &contexts,
        )
        .unwrap();
        let left = &epoch.captures()[0];
        let right = &epoch.captures()[1];
        assert_eq!(left.event.point, right.event.point);
        assert_eq!(left.literal_span.range, right.literal_span.range);
        assert_eq!(left.inputs[0].name, right.inputs[0].name);
        assert_ne!(left.event, right.event);
        assert_ne!(left.literal_owner, right.literal_owner);
        assert_ne!(left.inputs[0].binding, right.inputs[0].binding);
        assert_ne!(left.inputs[0].formal_shape, right.inputs[0].formal_shape);
    }
}

#[test]
fn finite_source_capture_runtime_indices_and_observations_leave_templates_unchanged() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let frozen = freeze(&fixture, &contexts).unwrap();
    let CfgPointKind::Capture { environment, .. } = &mut fixture.parent.initial_points[1].kind
    else {
        panic!("Capture fixture")
    };
    environment.index = 99999;
    fixture.parent.facts.push(ReferenceOrigin::Null {
        span: ByteSpan::new(90, 91),
    });
    let input = CallableCaptureInput {
        binding: captures(&fixture)[0].binding.clone(),
        origins: BTreeSet::from([ReferenceOriginId(0)]),
        environments: BTreeSet::new(),
        projected: Vec::new(),
        backings: Vec::new(),
        projected_backings: Vec::new(),
    };
    fixture.parent.inputs.push(input.clone());
    fixture.parent.capture_input_paths.push(vec![input.clone()]);
    fixture.parent.environments.push(CallableEnvironmentFact {
        literal: fixture.parent.initial_points[1].source_span.clone(),
        instance: CallableEnvironmentInstance::Capture,
        origins: BTreeSet::from([ReferenceOriginId(0)]),
        owners: BTreeSet::new(),
        loans: BTreeSet::new(),
        backings: Vec::new(),
        inputs: vec![input.clone()],
        capture_paths: vec![vec![input]],
    });
    assert_eq!(freeze(&fixture, &contexts).unwrap(), frozen);
}

#[test]
fn finite_source_capture_missing_lexical_owner_is_typed() {
    let fixture = fixture("app", "r1");
    assert_eq!(
        SourceEpoch::freeze(&[&fixture.parent], &fixture.structs, &fixture.enums, &original_contexts(&fixture)),
        Err(SourceEpochError::MissingLexicalOwner {
            event: event(&fixture),
            owner: fixture.literal.producer_owner.clone(),
        })
    );
}

#[test]
fn finite_source_capture_malformed_literal_range_and_environment_source_are_typed() {
    for change_range in [true, false] {
        let mut fixture = fixture("app", "r1");
        let contexts = original_contexts(&fixture);
        let CfgPointKind::Capture {
            literal,
            environment,
            ..
        } = &mut fixture.parent.initial_points[1].kind
        else {
            panic!("Capture fixture")
        };
        if change_range {
            *literal = ByteSpan::new(20, 91);
        } else {
            environment.literal.source.revision = "r2".into();
        }
        let literal = *literal;
        let environment_literal = environment.literal.clone();
        assert_eq!(
            freeze(&fixture, &contexts),
            Err(SourceEpochError::MalformedCaptureLiteral {
                event: event(&fixture),
                source_span: fixture.parent.initial_points[1].source_span.clone(),
                literal,
                environment_literal,
            })
        );
    }
}

#[test]
fn finite_source_capture_invalid_signature_is_typed() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let CfgPointKind::Capture { signature, .. } = &mut fixture.parent.initial_points[1].kind else {
        panic!("Capture fixture")
    };
    *signature = ScalarType::U8;
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::InvalidCaptureSignature {
            event: event(&fixture),
            signature: ScalarType::U8
        })
    );
}

#[test]
fn finite_source_capture_exact_name_span_type_arity_is_required() {
    for mismatch in 0..3 {
        let mut fixture = fixture("app", "r1");
        let contexts = original_contexts(&fixture);
        let CfgPointKind::Capture {
            signature,
            parameters,
            parameter_spans,
            ..
        } = &mut fixture.parent.initial_points[1].kind
        else {
            panic!("Capture fixture")
        };
        let ScalarType::Callable {
            parameters: types, ..
        } = signature
        else {
            panic!("Callable fixture")
        };
        match mismatch {
            0 => {
                parameters.push("third".into());
            }
            1 => {
                parameter_spans.push(ByteSpan::new(28, 29));
            }
            2 => {
                types.push(ScalarType::Bool);
            }
            _ => panic!("arity fixture"),
        }
        let names = parameters.len();
        let spans = parameter_spans.len();
        let types = types.len();
        let signature = signature.clone();
        assert_eq!(
            freeze(&fixture, &contexts),
            Err(SourceEpochError::CaptureParameterArity {
                event: event(&fixture),
                signature,
                names,
                spans,
                types
            })
        );
    }
}

#[test]
fn finite_source_capture_duplicate_binding_is_typed() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let CfgPointKind::Capture {
        captures: inputs, ..
    } = &mut fixture.parent.initial_points[1].kind
    else {
        panic!("Capture fixture")
    };
    inputs.push(inputs[0].clone());
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::DuplicateCaptureBinding {
            event: event(&fixture),
            binding: captures(&fixture)[0].binding.clone()
        })
    );
}

#[test]
fn finite_source_capture_missing_checked_storage_is_typed() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    fixture.parent.checked_bindings.clear();
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::MissingCheckedCaptureStorage {
            event: event(&fixture),
            binding: captures(&fixture)[1].binding.clone()
        })
    );
}

#[test]
fn finite_source_capture_foreign_checked_storage_binding_is_typed() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let checked = captures(&fixture)[1].binding.clone();
    let foreign = captures(&fixture)[0].binding.clone();
    fixture
        .parent
        .checked_bindings
        .get_mut(&checked)
        .unwrap()
        .binding = foreign.clone();
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::ForeignCheckedCaptureStorage {
            event: event(&fixture),
            binding: checked,
            storage_binding: foreign
        })
    );
}

#[test]
fn finite_source_capture_unresolved_capture_type_preserves_canonical_error() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let ty = ScalarType::Named {
        name: "Missing".into(),
        span: ByteSpan::new(10, 17),
    };
    let CfgPointKind::Capture { captures, .. } = &mut fixture.parent.initial_points[1].kind else {
        panic!("Capture fixture")
    };
    captures[0].ty = ty.clone();
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::CaptureShapeConstruction {
            event: event(&fixture),
            producer: fixture.parent.producer_owner.clone(),
            ty: ty.clone(),
            error: ValueShapeError::UnresolvedType(ty),
        })
    );
}

#[test]
fn finite_source_capture_missing_struct_declaration_preserves_canonical_error() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let structure = fixture.structs[0].id.clone();
    fixture.structs.clear();
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::CaptureShapeConstruction {
            event: event(&fixture),
            producer: fixture.parent.producer_owner.clone(),
            ty: ScalarType::Struct(structure.clone()),
            error: ValueShapeError::MissingStruct(structure),
        })
    );
}

#[test]
fn finite_source_capture_missing_enum_declaration_preserves_canonical_error() {
    let mut fixture = fixture("app", "r1");
    let contexts = original_contexts(&fixture);
    let enumeration = ScalarEnumId {
        source: fixture.structs[0].id.source.clone(),
        index: 99,
    };
    let CfgPointKind::Capture { captures, .. } = &mut fixture.parent.initial_points[1].kind else {
        panic!("Capture fixture")
    };
    captures[0].ty = ScalarType::Enum(enumeration.clone());
    assert_eq!(
        freeze(&fixture, &contexts),
        Err(SourceEpochError::CaptureShapeConstruction {
            event: event(&fixture),
            producer: fixture.parent.producer_owner.clone(),
            ty: ScalarType::Enum(enumeration.clone()),
            error: ValueShapeError::MissingEnum(enumeration),
        })
    );
}
