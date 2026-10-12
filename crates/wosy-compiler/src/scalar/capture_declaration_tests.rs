use super::*;

fn fixture(package: &str, revision: &str) -> (ProducerCfgOwnerId, ReferenceFlowPoint) {
    let source = SourceIdentity::new(
        "finite".into(),
        package.into(),
        "src/main.w".into(),
        revision.into(),
    );
    let span = SourceSpan::new(source.clone(), ByteSpan::new(20, 90));
    let body_span = ByteSpan::new(30, 89);
    let names = ["plain", "raw"];
    let captures = names
        .into_iter()
        .enumerate()
        .map(|(ordinal, name)| {
            let use_range = ByteSpan::new(50 + ordinal as u32, 51 + ordinal as u32);
            let place = match ordinal {
                0 => ScalarPlace::Name {
                    name: name.into(),
                    span: use_range,
                },
                1 => ScalarPlace::Dereference {
                    pointer: Box::new(ScalarExpression::Name {
                        name: name.into(),
                        span: use_range,
                    }),
                    span: use_range,
                },
                _ => panic!("capture fixture"),
            };
            closure::ClosureCapture {
                name: name.into(),
                binding: ReferenceBindingId {
                    source: source.clone(),
                    function_span: ByteSpan::new(0, 100),
                    declaration_span: ByteSpan::new(2 + ordinal as u32, 3 + ordinal as u32),
                    block_span: ByteSpan::new(0, 100),
                    kind: ReferenceBindingKind::Declared,
                },
                ty: match ordinal {
                    0 => ScalarType::U8,
                    1 => ScalarType::RawPointer(Box::new(ScalarType::U8)),
                    _ => panic!("capture fixture"),
                },
                place_type: ScalarType::U8,
                use_span: SourceSpan::new(source.clone(), use_range),
                place,
            }
        })
        .collect();
    let kind = CfgPointKind::Capture {
        literal: span.range,
        environment: CallableEnvironmentId {
            literal: span.clone(),
            index: 987,
        },
        signature: ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: vec![ScalarOutput {
                    ty: ScalarType::Unit,
                    span: ByteSpan::new(20, 24),
                }],
                span: ByteSpan::new(20, 24),
            },
            parameters: vec![
                ScalarType::CheckedReference {
                    mutability: ScalarReferenceMutability::Mutable,
                    inner: Box::new(ScalarType::U8),
                },
                ScalarType::RuntimeArray {
                    element: Box::new(ScalarType::U8),
                    span: ByteSpan::new(25, 28),
                },
            ],
        },
        captures,
        parameters: vec!["first".into(), "second".into()],
        parameter_spans: vec![ByteSpan::new(25, 26), ByteSpan::new(27, 28)],
        body: ScalarBlock {
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
        },
    };
    (
        ProducerCfgOwnerId::Module(source),
        ReferenceFlowPoint {
            id: CfgPointId(0),
            kind,
            source_span: span,
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
            successors: Vec::new(),
        },
    )
}

fn declaration<'a>(
    owner: &ProducerCfgOwnerId,
    point: &'a ReferenceFlowPoint,
) -> Result<SourceCaptureDeclaration<'a>, SourceEpochError> {
    let CfgPointKind::Capture {
        literal,
        environment,
        signature,
        captures,
        parameters,
        parameter_spans,
        body,
    } = &point.kind
    else {
        panic!("Capture fixture")
    };
    SourceCaptureDeclaration::new(
        owner,
        &point.source_span,
        point.id,
        *literal,
        &environment.literal,
        signature,
        captures,
        parameters,
        parameter_spans,
        body,
    )
}

fn event(owner: &ProducerCfgOwnerId, point: &ReferenceFlowPoint) -> SourceEventId {
    SourceEventId {
        owner: owner.clone(),
        point: point.id,
    }
}

#[test]
fn finite_capture_declaration_borrows_original_source_and_orders_parameter_recipes() {
    let (owner, point) = fixture("app", "r1");
    let declaration = declaration(&owner, &point).unwrap();
    let CfgPointKind::Capture {
        signature,
        captures,
        parameters,
        parameter_spans,
        body,
        ..
    } = &point.kind
    else {
        panic!("Capture fixture")
    };
    let ScalarType::Callable {
        outputs,
        parameters: types,
    } = signature
    else {
        panic!("Callable fixture")
    };
    assert_eq!(declaration.event, event(&owner, &point));
    assert_eq!(
        declaration.literal_owner,
        ProducerCfgOwnerId::Literal {
            enclosing: Box::new(owner.clone()),
            capture_point: point.id,
            literal: point.source_span.clone(),
        }
    );
    assert!(std::ptr::eq(declaration.literal_span, &point.source_span));
    assert!(std::ptr::eq(declaration.signature, signature));
    assert!(std::ptr::eq(declaration.body, body));
    assert!(std::ptr::eq(declaration.outputs, outputs));
    assert!(std::ptr::eq(declaration.captures, captures.as_slice()));
    assert_eq!(declaration.captures[1].place, captures[1].place);
    assert_eq!(declaration.captures[1].place_type, captures[1].place_type);
    assert_eq!(declaration.parameters.len(), 2);
    for (ordinal, parameter) in declaration.parameters.iter().enumerate() {
        assert_eq!(parameter.ordinal, ordinal);
        assert!(std::ptr::eq(parameter.name, &parameters[ordinal]));
        assert!(std::ptr::eq(parameter.ty, &types[ordinal]));
        assert_eq!(
            parameter.binding,
            ReferenceBindingId {
                source: point.source_span.source.clone(),
                function_span: point.source_span.range,
                declaration_span: parameter_spans[ordinal],
                block_span: body.span,
                kind: ReferenceBindingKind::Declared,
            }
        );
    }
}

#[test]
fn finite_capture_declaration_source_epoch_consumes_identical_binding_recipes() {
    let (owner, point) = fixture("app", "r1");
    let declaration = declaration(&owner, &point).unwrap();
    let mut parent = ScalarReferenceCfg::new(owner.clone());
    parent.initial_points.push(point.clone());
    let child = ScalarReferenceCfg::new(declaration.literal_owner.clone());
    let epoch = SourceEpoch::freeze(&[&parent, &child], &[], &[], &evaluated_value::template::concrete_contexts(&[owner.clone()], &[], &[])).unwrap();
    let template = &epoch.captures()[0];
    assert_eq!(template.event, declaration.event);
    assert_eq!(template.literal_owner, declaration.literal_owner);
    assert_eq!(template.literal_span, *declaration.literal_span);
    for (port, parameter) in template.parameters.iter().zip(&declaration.parameters) {
        assert_eq!(port.ordinal, parameter.ordinal);
        assert_eq!(port.name, *parameter.name);
        assert_eq!(port.binding, parameter.binding);
        let shape = epoch
            .shapes()
            .iter()
            .find(|row| row.id == port.shape)
            .unwrap();
        assert_eq!(shape.ty, *parameter.ty);
    }
    for (port, capture) in template.inputs.iter().zip(declaration.captures) {
        assert_eq!(port.binding, capture.binding);
        assert_eq!(port.place, capture.place);
        assert_eq!(port.use_span, capture.use_span);
    }
}

#[test]
fn finite_capture_declaration_revisions_and_packages_qualify_identical_local_labels() {
    let (owner, point) = fixture("app", "r1");
    let first = declaration(&owner, &point).unwrap();
    for (owner, point) in [fixture("app", "r2"), fixture("dep", "r1")] {
        let second = declaration(&owner, &point).unwrap();
        assert_eq!(first.event.point, second.event.point);
        assert_eq!(first.literal_span.range, second.literal_span.range);
        assert_eq!(first.parameters[0].name, second.parameters[0].name);
        assert_ne!(first.event, second.event);
        assert_ne!(first.literal_owner, second.literal_owner);
        assert_ne!(first.parameters[0].binding, second.parameters[0].binding);
        assert_ne!(first.captures[0].binding, second.captures[0].binding);
    }
}

#[test]
fn finite_capture_declaration_environment_index_is_outside_source_inputs() {
    let (owner, point) = fixture("app", "r1");
    let original = declaration(&owner, &point).unwrap();
    let mut observed = point.clone();
    let CfgPointKind::Capture { environment, .. } = &mut observed.kind else {
        panic!("Capture fixture")
    };
    environment.index = 99999;
    observed.possible_origins = Some(BTreeSet::from([ReferenceOriginId(999)]));
    assert_eq!(declaration(&owner, &observed).unwrap(), original);
}

#[test]
fn finite_capture_declaration_malformed_literal_retains_exact_source_error() {
    for change_range in [true, false] {
        let (owner, mut point) = fixture("app", "r1");
        let CfgPointKind::Capture {
            literal,
            environment,
            ..
        } = &mut point.kind
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
            declaration(&owner, &point),
            Err(SourceEpochError::MalformedCaptureLiteral {
                event: event(&owner, &point),
                source_span: point.source_span.clone(),
                literal,
                environment_literal,
            })
        );
    }
}

#[test]
fn finite_capture_declaration_non_callable_signature_retains_exact_source_error() {
    let (owner, mut point) = fixture("app", "r1");
    let CfgPointKind::Capture { signature, .. } = &mut point.kind else {
        panic!("Capture fixture")
    };
    *signature = ScalarType::U8;
    assert_eq!(
        declaration(&owner, &point),
        Err(SourceEpochError::InvalidCaptureSignature {
            event: event(&owner, &point),
            signature: ScalarType::U8,
        })
    );
}

#[test]
fn finite_capture_declaration_parameter_arity_retains_exact_source_error() {
    for mismatch in 0..3 {
        let (owner, mut point) = fixture("app", "r1");
        let CfgPointKind::Capture {
            signature,
            parameters,
            parameter_spans,
            ..
        } = &mut point.kind
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
            declaration(&owner, &point),
            Err(SourceEpochError::CaptureParameterArity {
                event: event(&owner, &point),
                signature,
                names,
                spans,
                types,
            })
        );
    }
}

#[test]
fn finite_capture_declaration_duplicate_binding_retains_exact_source_error() {
    let (owner, mut point) = fixture("app", "r1");
    let CfgPointKind::Capture { captures, .. } = &mut point.kind else {
        panic!("Capture fixture")
    };
    let binding = captures[0].binding.clone();
    captures.push(captures[0].clone());
    assert_eq!(
        declaration(&owner, &point),
        Err(SourceEpochError::DuplicateCaptureBinding {
            event: event(&owner, &point),
            binding
        })
    );
}
