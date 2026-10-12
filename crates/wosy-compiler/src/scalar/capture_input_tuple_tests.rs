use super::*;

struct Fixture {
    owner: ProducerCfgOwnerId,
    point: CfgPointId,
    span: SourceSpan,
    declarations: Vec<closure::ClosureCapture>,
    path: Vec<CallableCaptureInput>,
}

fn fixture(package: &str, revision: &str) -> Fixture {
    let source = SourceIdentity::new(
        "finite".into(),
        package.into(),
        "src/main.w".into(),
        revision.into(),
    );
    let span = SourceSpan::new(source.clone(), ByteSpan::new(20, 90));
    let structure = ScalarStructId {
        source: source.clone(),
        index: 0,
    };
    let field = ScalarFieldReference::Resolved(ScalarStructFieldId {
        structure: structure.clone(),
        index: 0,
    });
    let signature = ScalarType::Callable {
        outputs: ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty: ScalarType::Unit,
                span: ByteSpan::new(5, 9),
            }],
            span: ByteSpan::new(5, 9),
        },
        parameters: Vec::new(),
    };
    let declarations = ["plain", "checked", "record"]
        .into_iter()
        .enumerate()
        .map(|(ordinal, name)| {
            let range = ByteSpan::new(50 + ordinal as u32, 51 + ordinal as u32);
            let base = ScalarPlace::Name {
                name: name.into(),
                span: range,
            };
            let checked = ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Shared,
                inner: Box::new(signature.clone()),
            };
            let (ty, place_type, place) = match ordinal {
                0 => (ScalarType::U8, ScalarType::U8, base),
                1 => (checked.clone(), checked, base),
                2 => (
                    ScalarType::Struct(structure.clone()),
                    signature.clone(),
                    ScalarPlace::Field {
                        base: Box::new(base),
                        field: field.clone(),
                        span: range,
                    },
                ),
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
                ty,
                place_type,
                use_span: SourceSpan::new(source.clone(), range),
                place,
            }
        })
        .collect::<Vec<_>>();
    let path = declarations
        .iter()
        .map(|declaration| CallableCaptureInput {
            binding: declaration.binding.clone(),
            origins: BTreeSet::new(),
            environments: BTreeSet::new(),
            projected: Vec::new(),
            backings: Vec::new(),
            projected_backings: Vec::new(),
        })
        .collect::<Vec<_>>();
    Fixture {
        owner: ProducerCfgOwnerId::Module(source),
        point: CfgPointId(8),
        span,
        declarations,
        path,
    }
}

fn event(fixture: &Fixture) -> SourceEventId {
    SourceEventId {
        owner: fixture.owner.clone(),
        point: fixture.point,
    }
}

fn tuple(fixture: &Fixture) -> Result<CaptureInputTuple<'_>, CaptureInputTupleError> {
    CaptureInputTuple::new(
        &fixture.owner,
        fixture.point,
        &fixture.span,
        &fixture.declarations,
        &fixture.path,
    )
}

fn projected_place(declaration: &closure::ClosureCapture) -> ReferencePlaceId {
    let ScalarPlace::Field { field, .. } = &declaration.place else {
        panic!("projected capture fixture")
    };
    ReferencePlaceId {
        source: declaration.binding.source.clone(),
        function_span: declaration.binding.function_span,
        declaration_span: declaration.binding.declaration_span,
        block_span: declaration.binding.block_span,
        binding: declaration.binding.clone(),
        projections: vec![ReferencePlaceProjection::Field(field.clone())],
        place: declaration.place.clone(),
    }
}

fn observe(fixture: &mut Fixture) {
    fixture.path[1].origins = BTreeSet::from([ReferenceOriginId(10), ReferenceOriginId(11)]);
    fixture.path[1].environments.insert(CallableEnvironmentId {
        literal: SourceSpan::new(fixture.span.source.clone(), ByteSpan::new(101, 120)),
        index: 4,
    });
    let place = projected_place(&fixture.declarations[2]);
    fixture.path[2]
        .projected
        .push(CallableProjectedEnvironment {
            place: place.clone(),
            environments: BTreeSet::from([CallableEnvironmentId {
                literal: SourceSpan::new(fixture.span.source.clone(), ByteSpan::new(121, 140)),
                index: 5,
            }]),
        });
    let backing = RuntimeBackingFact {
        value: RuntimeValueId {
            source: fixture.span.source.clone(),
            function_span: ByteSpan::new(0, 100),
            creation_span: ByteSpan::new(10, 19),
            position: RuntimeValuePosition::Local,
        },
        owner: fixture.declarations[2].binding.clone(),
        kind: RuntimeBackingKind::OwnedAllocation,
        origin: SourceSpan::new(fixture.span.source.clone(), ByteSpan::new(10, 19)),
        length: RuntimeArrayLengthFact::Parameter {
            binding: fixture.declarations[0].binding.clone(),
        },
        callable_family: Vec::new(),
    };
    fixture.path[2].backings.push(backing.clone());
    fixture.path[2]
        .projected_backings
        .push(RuntimeProjectedBackingFact { place, backing });
}

fn build(
    fixture: &Fixture,
    provenance: &HolderProvenance,
) -> Result<CaptureInputPathFact, ReferenceAnalysisError> {
    build_capture_input_path_fact(
        &fixture.owner,
        fixture.point,
        &fixture.span,
        &fixture.declarations,
        &fixture.path,
        provenance,
    )
}

#[test]
fn finite_capture_input_tuple_zero_arity_is_explicit_and_builds_empty_fact() {
    let fixture = fixture("app", "r1");
    let tuple =
        CaptureInputTuple::new(&fixture.owner, fixture.point, &fixture.span, &[], &[]).unwrap();
    assert!(tuple.pairs().is_empty());
    let fact = build_capture_input_path_fact(
        &fixture.owner,
        fixture.point,
        &fixture.span,
        &[],
        &[],
        &HolderProvenance::default(),
    )
    .unwrap();
    assert_eq!(
        fact,
        CaptureInputPathFact {
            capture: fixture.point,
            capture_span: fixture.span.clone(),
            owner: fixture.owner.clone(),
            inputs: Vec::new()
        }
    );
}

#[test]
fn finite_capture_input_tuple_complete_ordered_pairs_borrow_unmodified_actual_records() {
    let mut fixture = fixture("app", "r1");
    observe(&mut fixture);
    let original_path = fixture.path.clone();
    let original_declarations = fixture.declarations.clone();
    let tuple = tuple(&fixture).unwrap();
    assert_eq!(tuple.pairs().len(), 3);
    for (ordinal, pair) in tuple.pairs().iter().enumerate() {
        assert!(std::ptr::eq(
            pair.declaration,
            &fixture.declarations[ordinal]
        ));
        assert!(std::ptr::eq(pair.snapshot, &fixture.path[ordinal]));
        assert_eq!(pair.declaration.binding, pair.snapshot.binding);
        assert_eq!(*pair.snapshot, original_path[ordinal]);
        assert_eq!(*pair.declaration, original_declarations[ordinal]);
    }
    assert!(matches!(tuple.pairs()[0].declaration.ty, ScalarType::U8));
    assert!(matches!(
        tuple.pairs()[1].declaration.ty,
        ScalarType::CheckedReference { .. }
    ));
    assert!(matches!(
        tuple.pairs()[2].declaration.place,
        ScalarPlace::Field { .. }
    ));
    assert_eq!(
        tuple.pairs()[1].snapshot.origins,
        BTreeSet::from([ReferenceOriginId(10), ReferenceOriginId(11)])
    );
    assert_eq!(tuple.pairs()[2].snapshot.projected.len(), 1);
    assert_eq!(tuple.pairs()[2].snapshot.backings.len(), 1);
    assert_eq!(tuple.pairs()[2].snapshot.projected_backings.len(), 1);
}

#[test]
fn finite_capture_input_tuple_shorter_and_longer_paths_have_exact_arity_errors() {
    for supplied in [2, 4] {
        let mut fixture = fixture("app", "r1");
        if supplied == 2 {
            fixture.path.pop();
        } else {
            fixture.path.push(fixture.path[0].clone());
        }
        assert_eq!(
            tuple(&fixture).unwrap_err(),
            CaptureInputTupleError::Arity {
                event: event(&fixture),
                source_span: fixture.span.clone(),
                declared: 3,
                supplied,
            }
        );
    }
}

#[test]
fn finite_capture_input_tuple_wrong_binding_has_exact_qualified_ordinal_error() {
    let mut fixture = fixture("app", "r1");
    fixture.path[1].binding = fixture.path[0].binding.clone();
    assert_eq!(
        tuple(&fixture).unwrap_err(),
        CaptureInputTupleError::Binding {
            event: event(&fixture),
            source_span: fixture.span.clone(),
            ordinal: 1,
            expected: fixture.declarations[1].binding.clone(),
            actual: fixture.path[1].binding.clone(),
        }
    );
}

#[test]
fn finite_capture_input_tuple_reversed_path_is_rejected_without_reordering() {
    let mut fixture = fixture("app", "r1");
    fixture.path.reverse();
    let original_path = fixture.path.clone();
    assert_eq!(
        tuple(&fixture).unwrap_err(),
        CaptureInputTupleError::Binding {
            event: event(&fixture),
            source_span: fixture.span.clone(),
            ordinal: 0,
            expected: fixture.declarations[0].binding.clone(),
            actual: fixture.path[0].binding.clone(),
        }
    );
    assert_eq!(fixture.path, original_path);
}

#[test]
fn finite_capture_input_tuple_duplicate_declaration_is_rejected() {
    let mut fixture = fixture("app", "r1");
    fixture.declarations[1].binding = fixture.declarations[0].binding.clone();
    fixture.path[1].binding = fixture.declarations[0].binding.clone();
    assert_eq!(
        tuple(&fixture).unwrap_err(),
        CaptureInputTupleError::DuplicateDeclaration {
            event: event(&fixture),
            source_span: fixture.span.clone(),
            ordinal: 1,
            binding: fixture.declarations[0].binding.clone(),
        }
    );
}

#[test]
fn finite_capture_input_tuple_equal_local_labels_different_revisions_packages_reject() {
    let original = fixture("app", "r1");
    for foreign in [fixture("app", "r2"), fixture("dep", "r1")] {
        assert_eq!(foreign.point, original.point);
        assert_eq!(foreign.span.range, original.span.range);
        assert_eq!(
            foreign.declarations[0].binding.declaration_span,
            original.declarations[0].binding.declaration_span
        );
        let result = CaptureInputTuple::new(
            &original.owner,
            original.point,
            &original.span,
            &original.declarations,
            &foreign.path,
        );
        assert_eq!(
            result.unwrap_err(),
            CaptureInputTupleError::Binding {
                event: event(&original),
                source_span: original.span.clone(),
                ordinal: 0,
                expected: original.declarations[0].binding.clone(),
                actual: foreign.path[0].binding.clone(),
            }
        );
    }
}

#[test]
fn finite_capture_input_tuple_sibling_events_can_share_actual_binding_records() {
    let fixture = fixture("app", "r1");
    let first = tuple(&fixture).unwrap();
    let sibling_span = SourceSpan::new(fixture.span.source.clone(), ByteSpan::new(120, 190));
    let second = CaptureInputTuple::new(
        &fixture.owner,
        CfgPointId(9),
        &sibling_span,
        &fixture.declarations,
        &fixture.path,
    )
    .unwrap();
    for (left, right) in first.pairs().iter().zip(second.pairs()) {
        assert!(std::ptr::eq(left.declaration, right.declaration));
        assert!(std::ptr::eq(left.snapshot, right.snapshot));
    }
}

#[test]
fn finite_capture_input_tuple_separate_paths_keep_their_own_complete_snapshots() {
    let mut first = fixture("app", "r1");
    observe(&mut first);
    let mut second_path = first.path.clone();
    second_path[1].origins = BTreeSet::from([ReferenceOriginId(90)]);
    second_path[1].environments.clear();
    second_path[2].projected.clear();
    let left = tuple(&first).unwrap();
    let right = CaptureInputTuple::new(
        &first.owner,
        first.point,
        &first.span,
        &first.declarations,
        &second_path,
    )
    .unwrap();
    assert_ne!(
        left.pairs()[1].snapshot.origins,
        right.pairs()[1].snapshot.origins
    );
    assert!(!left.pairs()[1].snapshot.environments.is_empty());
    assert!(right.pairs()[1].snapshot.environments.is_empty());
    assert!(!left.pairs()[2].snapshot.projected.is_empty());
    assert!(right.pairs()[2].snapshot.projected.is_empty());
    assert!(std::ptr::eq(right.pairs()[1].snapshot, &second_path[1]));
}

#[test]
fn finite_capture_input_tuple_valid_correspondence_preserves_missing_holder_failure() {
    let mut fixture = fixture("app", "r1");
    observe(&mut fixture);
    assert!(tuple(&fixture).is_ok());
    assert_eq!(
        build(&fixture, &HolderProvenance::default()),
        Err(ReferenceAnalysisError::MissingContext {
            source_span: fixture.declarations[1].use_span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
        })
    );
}

#[test]
fn finite_capture_input_tuple_projected_missing_holder_keeps_original_use_span_failure() {
    let mut fixture = fixture("app", "r1");
    observe(&mut fixture);
    fixture.path[1].environments.clear();
    assert!(tuple(&fixture).is_ok());
    assert_eq!(
        build(&fixture, &HolderProvenance::default()),
        Err(ReferenceAnalysisError::MissingContext {
            source_span: fixture.declarations[2].use_span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingGlobalCallableProducer,
        })
    );
}

#[test]
fn finite_capture_input_tuple_constructor_errors_precede_holder_provenance_lookup() {
    let mut fixture = fixture("app", "r1");
    observe(&mut fixture);
    fixture.path.pop();
    assert_eq!(
        build(&fixture, &HolderProvenance::default()),
        Err(ReferenceAnalysisError::MissingContext {
            source_span: fixture.span.clone(),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        })
    );
}

#[test]
fn finite_capture_input_tuple_binding_and_duplicate_errors_map_to_capture_context() {
    for duplicate in [true, false] {
        let mut fixture = fixture("app", "r1");
        if duplicate {
            fixture.declarations[1].binding = fixture.declarations[0].binding.clone();
        }
        fixture.path[1].binding = fixture.path[0].binding.clone();
        assert_eq!(
            build(&fixture, &HolderProvenance::default()),
            Err(ReferenceAnalysisError::MissingContext {
                source_span: fixture.span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            })
        );
    }
}

#[test]
fn finite_capture_input_tuple_productive_facts_preserve_actual_direct_projected_producers() {
    let mut fixture = fixture("app", "r1");
    observe(&mut fixture);
    let environment = fixture.path[1].environments.iter().next().unwrap().clone();
    let holder = CallableHolderPlace::Binding(fixture.declarations[1].binding.clone());
    let producer = HolderProducerRef::CaptureCreation {
        point: CfgPointId(42),
        environment: environment.clone(),
    };
    let projected = &fixture.path[2].projected[0];
    let projected_environment = projected.environments.iter().next().unwrap().clone();
    let projected_holder = CallableHolderPlace::Projection(projected.place.clone());
    let projected_producer = HolderProducerRef::GlobalCallsiteInput {
        call: SourceSpan::new(fixture.span.source.clone(), ByteSpan::new(200, 210)),
        holder: projected_holder.clone(),
        environment: projected_environment.clone(),
    };
    let mut provenance = HolderProvenance::default();
    provenance.insert(environment.clone(), holder.clone(), producer.clone());
    provenance.insert(
        projected_environment.clone(),
        projected_holder.clone(),
        projected_producer.clone(),
    );
    assert_eq!(
        build(&fixture, &provenance).unwrap(),
        CaptureInputPathFact {
            capture: fixture.point,
            capture_span: fixture.span.clone(),
            owner: fixture.owner.clone(),
            inputs: vec![
                CaptureInputFact {
                    binding: fixture.declarations[0].binding.clone(),
                    origin: CaptureInputOriginFact::Value
                },
                CaptureInputFact {
                    binding: fixture.declarations[1].binding.clone(),
                    origin: CaptureInputOriginFact::Callable {
                        holders: vec![ConnectedCaptureHolderFact {
                            holder,
                            environment,
                            producer
                        }],
                    }
                },
                CaptureInputFact {
                    binding: fixture.declarations[2].binding.clone(),
                    origin: CaptureInputOriginFact::Callable {
                        holders: vec![ConnectedCaptureHolderFact {
                            holder: projected_holder,
                            environment: projected_environment,
                            producer: projected_producer
                        }],
                    }
                },
            ],
        }
    );
}
