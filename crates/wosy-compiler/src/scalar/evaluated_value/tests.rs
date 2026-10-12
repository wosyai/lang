use super::*;

fn schema_program(layout: ScalarTargetLayout) -> ScalarProgram {
    let source = source("src/main.w");
    ScalarProgram {
        source: source.clone(),
        items: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        target_layout: layout,
        resolved_literals: RefCell::new(Vec::new()),
        resolved_assignment_types: RefCell::new(Vec::new()),
        resolved_expression_outputs: RefCell::new(Vec::new()),
        top_level_reference_cfg: ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(source)),
        terminal_owners: Vec::new(),
        resolved_assignment_outputs: RefCell::new(HashMap::new()),
    }
}

fn schema_expression(text: &str, source: &SourceIdentity) -> ScalarExpression {
    let parsed = crate::parse_source(source.clone(), format!("%%start\n{text};\n%%end"), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Expression)
        .expect("expression");
    derive_expression(&node, source)
}

fn source(path: &str) -> SourceIdentity {
    SourceIdentity::new("schema".into(), "app".into(), path.into(), "r1".into())
}

fn typed_outputs(layout: ScalarTargetLayout, project: bool) {
    let source = source("src/main.w");
    let text = "%%start\n(x, y = 3, 4);\n%%end";
    let parsed = crate::parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Assignment)
        .expect("canonical assignment");
    let expression = derive_expression(&node, &source);
    let program = ScalarProgram {
        source: source.clone(),
        items: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        target_layout: layout,
        resolved_literals: RefCell::new(Vec::new()),
        resolved_assignment_types: RefCell::new(Vec::new()),
        resolved_expression_outputs: RefCell::new(Vec::new()),
        top_level_reference_cfg: ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(
            source.clone(),
        )),
        terminal_owners: Vec::new(),
        resolved_assignment_outputs: RefCell::new(HashMap::new()),
    };
    let scope = BTreeMap::from([("x".into(), ScalarType::I32), ("y".into(), ScalarType::U64)]);
    let mut diagnostics = Vec::new();
    let fact = if project {
        let module = ScalarModule::from_program(program, Vec::new());
        let modules = vec![module.clone()];
        let ty = typecheck::expression_type_in_module(
            &expression,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &module,
            &modules,
            &mut diagnostics,
            false,
        );
        assert_eq!(ty, ScalarType::I32);
        let fact = module.resolved_assignment_types.borrow()[0].clone();
        fact
    } else {
        let ty = typecheck::expression_type(
            &expression,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            false,
        );
        assert_eq!(ty, ScalarType::I32);
        let fact = program.resolved_assignment_types.borrow()[0].clone();
        fact
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        fact.previous
            .outputs
            .iter()
            .map(|output| &output.ty)
            .collect::<Vec<_>>(),
        vec![&ScalarType::I32, &ScalarType::U64]
    );
    assert_eq!(
        fact.replacements
            .outputs
            .iter()
            .map(|output| &output.ty)
            .collect::<Vec<_>>(),
        vec![&ScalarType::I32, &ScalarType::U64]
    );
    assert_eq!(fact.occurrence.source_span.source, source);
    let serialized = serde_json::to_vec(&fact).expect("serialize type fact");
    assert_eq!(
        serde_json::from_slice::<ResolvedAssignmentTypeFact>(&serialized).expect("type fact"),
        fact
    );
}

#[test]
fn compositional_assignment_result_schema_outputs_program_native64() {
    typed_outputs(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn compositional_assignment_result_schema_outputs_program_wasm32() {
    typed_outputs(ScalarTargetLayout::WASM32, false);
}

#[test]
fn compositional_assignment_result_schema_outputs_project_native64() {
    typed_outputs(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn compositional_assignment_result_schema_outputs_project_wasm32() {
    typed_outputs(ScalarTargetLayout::WASM32, true);
}

#[test]
fn compositional_assignment_result_schema_plain_checked_raw_callable_and_array_shapes() {
    let producer = ProducerCfgOwnerId::Module(source("src/main.w"));
    let mut shapes = Vec::new();
    let plain = [
        ScalarType::Unit,
        ScalarType::Bool,
        ScalarType::Char,
        ScalarType::ArtifactId,
        ScalarType::I8,
        ScalarType::I16,
        ScalarType::I32,
        ScalarType::I64,
        ScalarType::I128,
        ScalarType::U8,
        ScalarType::U16,
        ScalarType::U32,
        ScalarType::U64,
        ScalarType::U128,
        ScalarType::F32,
        ScalarType::F64,
    ];
    for ty in plain {
        let id = intern_value_shape(&mut shapes, &producer, &ty, &[], &[], &producer, &[]).expect("plain shape");
        assert_eq!(shapes[id.index].ty, ty);
        assert!(matches!(
            shapes[id.index].payload,
            ValuePayloadShape::Plain(_)
        ));
    }
    let span = ByteSpan::new(10, 20);
    let types = [
        ScalarType::RawPointer(Box::new(ScalarType::U8)),
        ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Shared,
            inner: Box::new(ScalarType::U8),
        },
        ScalarType::CheckedReference {
            mutability: ScalarReferenceMutability::Mutable,
            inner: Box::new(ScalarType::U8),
        },
        ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: vec![ScalarOutput {
                    ty: ScalarType::U32,
                    span,
                }],
                span,
            },
            parameters: vec![ScalarType::I32],
        },
        ScalarType::Array {
            element: Box::new(ScalarType::U8),
            length: 4,
            length_span: span,
            span,
        },
        ScalarType::RuntimeArray {
            element: Box::new(ScalarType::U8),
            span,
        },
    ];
    for ty in types {
        let id = intern_value_shape(&mut shapes, &producer, &ty, &[], &[], &producer, &[]).expect("shape");
        let count = shapes.len();
        assert_eq!(
            intern_value_shape(&mut shapes, &producer, &ty, &[], &[], &producer, &[]).expect("reuse"),
            id
        );
        assert_eq!(shapes.len(), count);
        assert_eq!(shapes[id.index].ty, ty);
    }
    let encoded = serde_json::to_vec(&shapes).expect("shape serialization");
    assert_eq!(
        serde_json::from_slice::<Vec<ResolvedValueShape>>(&encoded).expect("shapes"),
        shapes
    );
}

#[test]
fn compositional_assignment_result_schema_qualified_ids_remap_equal_offsets() {
    let first = ProducerCfgOwnerId::Module(source("src/first.w"));
    let second = ProducerCfgOwnerId::Module(source("src/second.w"));
    let first_value = EvaluatedValueId {
        resolved: ResolvedValueOutputId {
            producer: first.clone(),
            occurrence: SourceSpan::new(source("src/first.w"), ByteSpan::new(10, 20)),
            output_projection: vec![2],
            output: 1,
        },
        point: CfgPointId(3),
    };
    let second_value = EvaluatedValueId {
        resolved: ResolvedValueOutputId {
            producer: second.clone(),
            occurrence: SourceSpan::new(source("src/second.w"), ByteSpan::new(10, 20)),
            output_projection: vec![2],
            output: 1,
        },
        point: CfgPointId(3),
    };
    assert_ne!(first_value, second_value);
    let imported = ProducerCfgOwnerId::Module(source("src/import.w"));
    let remap = ValueSchemaRemap {
        imports: Vec::new(),
        producers: vec![
            (first.clone(), imported.clone()),
            (second.clone(), imported.clone()),
        ],
        points: vec![
            ValuePointRemap {
                producer: first,
                point: CfgPointId(3),
                imported: CfgPointId(8),
            },
            ValuePointRemap {
                producer: second,
                point: CfgPointId(3),
                imported: CfgPointId(9),
            },
        ],
        shapes: Vec::new(),
    };
    let mapped_first = remap.value(&first_value).expect("first remap");
    let mapped_second = remap.value(&second_value).expect("second remap");
    assert_eq!(mapped_first.point, CfgPointId(8));
    assert_eq!(mapped_second.point, CfgPointId(9));
    assert_eq!(
        mapped_first.resolved.occurrence,
        first_value.resolved.occurrence
    );
    assert_ne!(mapped_first, mapped_second);
    assert!(matches!(
        remap.point(&imported, CfgPointId(3)),
        Err(ValueSchemaRemapError::MissingPoint { .. })
    ));
    let encoded = serde_json::to_vec(&mapped_first).expect("value serialization");
    assert_eq!(
        serde_json::from_slice::<EvaluatedValueId>(&encoded).expect("value"),
        mapped_first
    );
    let cfg = ScalarReferenceCfg::new(imported);
    assert!(cfg.evaluated_values.values.is_empty());
    assert!(cfg.evaluated_values.snapshots.is_empty());
    assert!(cfg.evaluated_values.commits.is_empty());
    assert!(cfg.evaluated_values.results.is_empty());
    let encoded = serde_json::to_vec(&cfg).expect("empty CFG serialization");
    assert_eq!(
        serde_json::from_slice::<ScalarReferenceCfg>(&encoded).expect("CFG"),
        cfg
    );
}

#[test]
fn compositional_assignment_result_schema_recursive_resolved_struct_signatures() {
    let source = source("src/recursive.w");
    let parsed = crate::parse_source(
        source.clone(),
        "%%start\nstruct Node { *Node next; *Node(*Node) visit; }\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::StructDecl)
        .expect("resolved declaration source");
    let id = ScalarStructId {
        source: source.clone(),
        index: 0,
    };
    let structure = derive_struct(node, id.clone(), ScalarTargetLayout::NATIVE64);
    let mut program = ScalarProgram {
        source: source.clone(),
        items: Vec::new(),
        structs: vec![structure],
        enums: Vec::new(),
        target_layout: ScalarTargetLayout::NATIVE64,
        resolved_literals: RefCell::new(Vec::new()),
        resolved_assignment_types: RefCell::new(Vec::new()),
        resolved_expression_outputs: RefCell::new(Vec::new()),
        top_level_reference_cfg: ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(
            source.clone(),
        )),
        terminal_owners: Vec::new(),
        resolved_assignment_outputs: RefCell::new(HashMap::new()),
    };
    let mut diagnostics = Vec::new();
    resolve_program_types(&mut program, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let producer = ProducerCfgOwnerId::Module(source);
    let mut shapes = Vec::new();
    let root = intern_value_shape(
        &mut shapes,
        &producer,
        &ScalarType::Struct(id.clone()),
        &program.structs,
        &program.enums, &producer, &[],
    )
    .expect("recursive shape");
    let ValuePayloadShape::Struct { fields, .. } = &shapes[root.index].payload else {
        panic!("struct shape");
    };
    assert_eq!(fields.len(), 2);
    let pointer = &shapes[fields[0].shape.index];
    assert_eq!(
        pointer.payload,
        ValuePayloadShape::Checked {
            mutability: ScalarReferenceMutability::Shared,
            referent: root.clone(),
        }
    );
    assert!(matches!(
        shapes[fields[1].shape.index].payload,
        ValuePayloadShape::Callable { .. }
    ));
    let count = shapes.len();
    assert_eq!(
        intern_value_shape(
            &mut shapes,
            &producer,
            &ScalarType::Struct(id),
            &program.structs,
            &program.enums, &producer, &[]
        )
        .expect("same recursive shape"),
        root
    );
    assert_eq!(count, shapes.len());
    let unresolved = ScalarType::Named {
        name: "T".into(),
        span: ByteSpan::new(1, 2),
    };
    assert_eq!(
        intern_value_shape(
            &mut shapes,
            &producer,
            &unresolved,
            &program.structs,
            &program.enums, &producer, &[]
        ),
        Err(ValueShapeError::UnresolvedType(unresolved))
    );
    assert_eq!(count, shapes.len());
}

#[test]
fn compositional_assignment_result_schema_late_binding_requires_actual_cfg_point() {
    let producer = ProducerCfgOwnerId::Module(source("src/main.w"));
    let output = ResolvedValueOutputId {
        producer: producer.clone(),
        occurrence: SourceSpan::new(source("src/main.w"), ByteSpan::new(1, 4)),
        output_projection: Vec::new(),
        output: 0,
    };
    let cfg = ScalarReferenceCfg::new(producer);
    assert_eq!(
        bind_evaluated_value_id(&cfg, output.clone(), CfgPointId(0)),
        Err(EvaluatedValueBindingError::MissingPoint {
            output: output.clone(),
            point: CfgPointId(0)
        })
    );
    let other = ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(source("src/other.w")));
    assert!(matches!(
        bind_evaluated_value_id(&other, output, CfgPointId(0)),
        Err(EvaluatedValueBindingError::WrongProducer { .. })
    ));
}

fn consumer_outputs(layout: ScalarTargetLayout, project: bool) {
    let program = schema_program(layout);
    let source = program.source.clone();
    let module = ScalarModule::from_program(program.clone(), Vec::new());
    let span = ByteSpan::new(0, 1);
    let scope = BTreeMap::from([
        ("x".into(), ScalarType::I32),
        ("y".into(), ScalarType::U64),
        (
            "accept".into(),
            ScalarType::Callable {
                parameters: vec![ScalarType::I32, ScalarType::U64],
                outputs: ScalarOutputSequence {
                    outputs: vec![ScalarOutput {
                        ty: ScalarType::Bool,
                        span,
                    }],
                    span,
                },
            },
        ),
    ]);
    for (text, expected) in [
        ("accept((x, y = 3, 4))", vec![ScalarType::Bool]),
        ("{ (x, y = 3, 4) }", vec![ScalarType::I32, ScalarType::U64]),
        (
            "if (true) { (x, y = 3, 4) } else { 5, 6 }",
            vec![ScalarType::I32, ScalarType::U64],
        ),
    ] {
        let expression = schema_expression(text, &source);
        let mut diagnostics = Vec::new();
        let outputs = if project {
            typecheck::outputs::sequence_in_module(
                &expression,
                &expected,
                &scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &module,
                &[module.clone()],
                &mut diagnostics,
                false,
            )
        } else {
            typecheck::outputs::sequence(
                &expression,
                &expected,
                &scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &program,
                &mut diagnostics,
                false,
            )
        };
        assert!(diagnostics.is_empty(), "{text}: {diagnostics:?}");
        assert_eq!(
            outputs
                .outputs
                .iter()
                .map(|output| output.ty.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(outputs
            .outputs
            .iter()
            .all(|output| output.span.start <= output.span.end));
    }
    let expression = schema_expression("if (true) { (x, y = 3, 4) } else { 5, false }", &source);
    let mut diagnostics = Vec::new();
    if project {
        typecheck::outputs::sequence_in_module(
            &expression,
            &[ScalarType::I32, ScalarType::U64],
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &module,
            &[module.clone()],
            &mut diagnostics,
            false,
        );
    } else {
        typecheck::outputs::sequence(
            &expression,
            &[ScalarType::I32, ScalarType::U64],
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            false,
        );
    }
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0006"));
    assert!(diagnostics
        .iter()
        .flat_map(|diagnostic| &diagnostic.labels)
        .all(|label| label.span.source == source));
}

#[test]
fn compositional_assignment_result_schema_consumers_program_native64() {
    consumer_outputs(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn compositional_assignment_result_schema_consumers_program_wasm32() {
    consumer_outputs(ScalarTargetLayout::WASM32, false);
}

#[test]
fn compositional_assignment_result_schema_consumers_project_native64() {
    consumer_outputs(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn compositional_assignment_result_schema_consumers_project_wasm32() {
    consumer_outputs(ScalarTargetLayout::WASM32, true);
}

#[test]
fn compositional_assignment_result_schema_independent_producer_shape_indices() {
    let first = ProducerCfgOwnerId::Module(source("src/first.w"));
    let second = ProducerCfgOwnerId::Module(source("src/second.w"));
    let mut shapes = Vec::new();
    let first_id =
        intern_value_shape(&mut shapes, &first, &ScalarType::U32, &[], &[], &first, &[]).expect("first");
    let second_id =
        intern_value_shape(&mut shapes, &second, &ScalarType::U32, &[], &[], &second, &[]).expect("second");
    assert_eq!(first_id.index, 0);
    assert_eq!(second_id.index, 0);
    assert_ne!(first_id, second_id);
    let pointer = intern_value_shape(
        &mut shapes,
        &second,
        &ScalarType::RawPointer(Box::new(ScalarType::U32)),
        &[],
        &[], &second, &[],
    )
    .expect("pointer");
    assert_eq!(pointer.index, 1);
    assert_eq!(
        shapes
            .iter()
            .find(|shape| shape.id == pointer)
            .expect("qualified pointer")
            .payload,
        ValuePayloadShape::Raw {
            pointee: second_id.clone()
        }
    );
    assert_eq!(
        intern_value_shape(&mut shapes, &first, &ScalarType::U32, &[], &[], &first, &[]).expect("first reuse"),
        first_id
    );
    assert_eq!(
        intern_value_shape(&mut shapes, &second, &ScalarType::U32, &[], &[], &second, &[]).expect("second reuse"),
        second_id
    );
}

#[test]
fn compositional_assignment_result_schema_inferred_statement_and_observed_unbound() {
    let program = schema_program(ScalarTargetLayout::NATIVE64);
    let expression = schema_expression("(fresh = 3)", &program.source);
    let ScalarExpression::Assignment(assignment) = expression else {
        panic!("assignment")
    };
    let mut scope = BTreeMap::new();
    let mut diagnostics = Vec::new();
    typecheck::assignment_output_types(
        &assignment,
        &mut scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        &program,
        &mut diagnostics,
        false,
        false,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(program.resolved_assignment_types.borrow()[0]
        .previous
        .outputs
        .is_empty());
    assert_eq!(scope.get("fresh"), Some(&ScalarType::I32));
    diagnostics.clear();
    typecheck::expression_type(
        &ScalarExpression::Assignment(assignment),
        &BTreeMap::new(),
        &BTreeSet::new(),
        &BTreeMap::new(),
        &program,
        &mut diagnostics,
        false,
    );
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B0001"));
    assert!(diagnostics
        .iter()
        .flat_map(|diagnostic| &diagnostic.labels)
        .all(|label| label.span.source == program.source));
}

#[test]
fn compositional_assignment_result_schema_checked_selector_nested_types() {
    let program = schema_program(ScalarTargetLayout::NATIVE64);
    let expression = schema_expression("(*(target = replacement) = 9)", &program.source);
    let reference = ScalarType::CheckedReference {
        mutability: ScalarReferenceMutability::Mutable,
        inner: Box::new(ScalarType::U32),
    };
    let scope = BTreeMap::from([
        ("target".into(), reference.clone()),
        ("replacement".into(), reference.clone()),
    ]);
    let mut diagnostics = Vec::new();
    assert_eq!(
        typecheck::expression_type(
            &expression,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            false
        ),
        ScalarType::U32
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let facts = program.resolved_assignment_types.borrow();
    assert_eq!(facts.len(), 2);
    assert!(facts.iter().any(|fact| fact
        .previous
        .outputs
        .iter()
        .map(|output| &output.ty)
        .collect::<Vec<_>>()
        == vec![&reference]));
    assert!(facts.iter().any(|fact| fact
        .previous
        .outputs
        .iter()
        .map(|output| &output.ty)
        .collect::<Vec<_>>()
        == vec![&ScalarType::U32]));
}

#[test]
fn compositional_assignment_result_schema_full_tables_remap_and_serde() {
    let program = schema_program(ScalarTargetLayout::NATIVE64);
    let source = program.source.clone();
    let producer = ProducerCfgOwnerId::Module(source.clone());
    let imported_source = self::source("src/import.w");
    let imported = ProducerCfgOwnerId::Module(imported_source.clone());
    let span = ByteSpan::new(10, 20);
    let witness = SourceSpan::new(source.clone(), span);
    let binding = ReferenceBindingId {
        source: source.clone(),
        function_span: span,
        declaration_span: span,
        block_span: span,
        kind: ReferenceBindingKind::Declared,
    };
    let imported_binding = ReferenceBindingId {
        source: imported_source,
        ..binding.clone()
    };
    let place = ReferencePlaceId {
        source: source.clone(),
        function_span: span,
        declaration_span: span,
        block_span: span,
        binding: binding.clone(),
        projections: Vec::new(),
        place: ScalarPlace::Name {
            name: "x".into(),
            span,
        },
    };
    let imported_place = ReferencePlaceId {
        binding: imported_binding.clone(),
        source: imported_binding.source.clone(),
        ..place.clone()
    };
    let loan = ReferenceLoanId {
        mode: ScalarReferenceMutability::Shared,
        origin_place: place.clone(),
        creation_source: source.clone(),
        creation_span: span,
        parent: None,
    };
    let imported_loan = ReferenceLoanId {
        origin_place: imported_place.clone(),
        ..loan.clone()
    };
    let runtime = RuntimeValueId {
        source: source.clone(),
        function_span: span,
        creation_span: span,
        position: RuntimeValuePosition::Local,
    };
    let imported_runtime = RuntimeValueId {
        source: imported_binding.source.clone(),
        ..runtime.clone()
    };
    let backing = RuntimeBackingFact {
        value: runtime.clone(),
        owner: binding.clone(),
        kind: RuntimeBackingKind::OwnedAllocation,
        origin: witness.clone(),
        length: RuntimeArrayLengthFact::Parameter {
            binding: binding.clone(),
        },
        callable_family: Vec::new(),
    };
    let imported_backing = RuntimeBackingFact {
        value: imported_runtime.clone(),
        owner: imported_binding.clone(),
        length: RuntimeArrayLengthFact::Parameter {
            binding: imported_binding.clone(),
        },
        ..backing.clone()
    };
    let environment = CallableEnvironmentId {
        literal: witness.clone(),
        index: 1,
    };
    let imported_environment = CallableEnvironmentId {
        literal: witness.clone(),
        index: 7,
    };
    let holder = ConnectedCaptureHolderFact {
        holder: CallableHolderPlace::Binding(binding.clone()),
        environment: environment.clone(),
        producer: HolderProducerRef::CaptureCreation {
            point: CfgPointId(2),
            environment: environment.clone(),
        },
    };
    let imported_holder = ConnectedCaptureHolderFact {
        holder: CallableHolderPlace::Binding(imported_binding.clone()),
        environment: imported_environment.clone(),
        producer: HolderProducerRef::CaptureCreation {
            point: CfgPointId(12),
            environment: imported_environment.clone(),
        },
    };
    let id = EvaluatedValueId {
        resolved: ResolvedValueOutputId {
            producer: producer.clone(),
            occurrence: witness.clone(),
            output_projection: vec![1],
            output: 0,
        },
        point: CfgPointId(2),
    };
    let case_id = ValueCaseId {
        value: id.clone(),
        alternative: 3,
    };
    let shape = ValueShapeId {
        producer: producer.clone(),
        index: 0,
    };
    let imported_shape = ValueShapeId {
        producer: imported.clone(),
        index: 5,
    };
    let capability = OwnerCapabilityId {
        producer: producer.clone(),
        introduction: CfgPointId(2),
    };
    let remap = ValueSchemaRemap {
        producers: vec![(producer.clone(), imported.clone())],
        points: (0..5)
            .map(|index| ValuePointRemap {
                producer: producer.clone(),
                point: CfgPointId(index),
                imported: CfgPointId(index + 10),
            })
            .collect(),
        shapes: vec![(shape.clone(), imported_shape.clone())],
        imports: vec![ValueImportCorrespondence {
            producer: producer.clone(),
            bindings: vec![(binding.clone(), imported_binding.clone())],
            places: vec![(place.clone(), imported_place.clone())],
            origins: vec![ReferenceOriginId(6)],
            loans: vec![(loan.clone(), imported_loan.clone())],
            runtime_values: vec![(runtime.clone(), imported_runtime.clone())],
            environments: vec![(environment.clone(), imported_environment)],
            backings: vec![(backing.clone(), imported_backing.clone())],
            holders: vec![(holder.clone(), imported_holder.clone())],
        }],
    };
    let expression = schema_expression("(x = 4)", &source);
    let ScalarExpression::Assignment(assignment) = expression else {
        panic!("operation")
    };
    let operation = AssignmentOperationId {
        producer: producer.clone(),
        occurrence: (*assignment.occurrence).clone(),
    };
    let destination_id = AssignmentDestinationId {
        operation: operation.clone(),
        target: 0,
    };
    let consumer = ValueConsumerId {
        producer: producer.clone(),
        point: CfgPointId(4),
        kind: ValueConsumerKind::Binding {
            binding: binding.clone(),
            projection: vec![ValueProjection::RuntimeIndex {
                selector: id.clone(),
            }],
        },
    };
    let disposition = ValueDisposition {
        value: id.clone(),
        case: case_id.clone(),
        point: CfgPointId(4),
        action: ValueDispositionAction::MoveTo(consumer.clone()),
    };
    let dependency = EvaluatedDependencyFact {
        projection: Vec::new(),
        source: DependencySource::FixedReferent {
            origin: ReferenceOriginId(0),
            loan: loan.clone(),
        },
        action: DependencyAction::BorrowFixed {
            referent_lifetime: FixedLifetime::Capability(capability.clone()),
            use_owner: capability.clone(),
        },
        witness: witness.clone(),
    };
    let case = EvaluatedValueCase {
        id: case_id.clone(),
        path: EvaluatedControlPathId {
            producer: producer.clone(),
            ordinal: 2,
        },
        availability: ValueAvailability::Moved {
            place: place.clone(),
            move_point: CfgPointId(3),
            source: witness.clone(),
        },
        payload: EvaluatedPayloadFact::Checked {
            origins: BTreeSet::from([ReferenceOriginId(0)]),
            state: CheckedValueState::Expired {
                origin: ReferenceOriginId(0),
                effect_point: CfgPointId(3),
            },
        },
        dependencies: vec![dependency],
    };
    let destination = EvaluatedAssignmentDestination {
        id: destination_id.clone(),
        point: CfgPointId(1),
        source_span: witness.clone(),
        shape: shape.clone(),
        storage: EvaluatedDestinationStorage::Through {
            pointer: id.clone(),
            cases: vec![DestinationCase {
                case: case_id.clone(),
                place: place.clone(),
                access: ReferenceBorrowAccess::Through {
                    pointer_point: CfgPointId(2),
                    parent_origin: ReferenceOriginId(0),
                },
            }],
        },
        selector_inputs: vec![id.clone()],
    };
    let snapshot = AssignmentSnapshot {
        destination: destination_id.clone(),
        point: CfgPointId(3),
        state: SnapshotState::Saved(id.clone()),
    };
    let commit = AssignmentCommit {
        destination: destination_id,
        point: CfgPointId(4),
        replacement: id.clone(),
        previous: CfgPointId(3),
        dispositions: vec![disposition.clone()],
    };
    let result = AssignmentResult {
        operation: operation.clone(),
        output: 0,
        snapshot: CfgPointId(3),
        value: id.clone(),
        consumers: vec![consumer.clone()],
    };
    let previous = AssignmentPreviousPlan {
        target: 0,
        previous: PreviousOutputPlan::Existing {
            output: id.resolved.clone(),
            shape: shape.clone(),
        },
    };
    let mut tables = EvaluatedValueTables::new();
    tables.shapes.push(ResolvedValueShape {
        id: shape.clone(),
        ty: ScalarType::U32,
        payload: ValuePayloadShape::Plain(PlainValueKind::U32),
    });
    tables.values.push(EvaluatedValueFact {
        id: id.clone(),
        shape: shape.clone(),
        cases: vec![case.clone()],
    });
    tables.assignment_previous.push(previous.clone());
    tables.assignment_plans.push(ResolvedAssignmentPlan {
        operation: operation.clone(),
        targets: vec![ResolvedAssignmentTarget {
            ordinal: 0,
            source_span: witness.clone(),
            place: place.place.clone(),
            ty: ScalarType::U32,
            storage: ResolvedAssignmentStorage::Existing(place.clone()),
        }],
        replacements: vec![AssignmentReplacementProjection {
            expression_ordinal: 0,
            producer_output: 0,
            destination_ordinal: 0,
            source_type: ScalarType::U32,
        }],
        previous_outputs: vec![previous],
        consumption: AssignmentConsumption::Received {
            outputs: vec![consumer],
            discards: vec![id.resolved.clone()],
        },
    });
    tables
        .assignment_evaluations
        .push(ScalarAssignmentEvaluation {
            operation,
            rhs: vec![AssignmentRhsEvaluation {
                expression_ordinal: 0,
                producer_output: 0,
                value: id.clone(),
            }],
            destinations: vec![destination.clone()],
            previous: vec![snapshot.clone()],
            writes: vec![commit.clone()],
            results: vec![result.clone()],
        });
    tables.dispositions.push(disposition);
    tables.destinations.push(destination);
    tables.snapshots.push(snapshot);
    tables.commits.push(commit);
    tables.results.push(result);
    let sources = vec![remap::MechanicalSchemaTableSource {
        producer: producer.clone(), tables: tables.clone(),
        policy_context: SourceCapturePolicyContext { structs: Vec::new(), enums: Vec::new(), generic_scope: producer.clone(), generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() },
        capture_inventory: SourceCaptureInventory { phase: SourceCaptureConstructionPhase::Complete, required: Vec::new() },
        initial_points: Vec::new(), capture_points: Vec::new(), top_level: ScalarTopLevelCorrespondence::default(), children: Vec::new(),
    }];

    let mapped_graph = remap.mechanical_schema_tables(&sources).expect("whole tables");
    let roots = mapped_graph.iter().filter(|row| row.source == producer).collect::<Vec<_>>();
    let [root] = roots.as_slice() else { panic!("unique original synthetic root"); };
    assert_eq!(root.capture_inventory, sources[0].capture_inventory);
    assert!(root.children.is_empty());
    let mapped = root.tables.clone();
    assert_eq!(mapped.values[0].id.point, CfgPointId(12));
    assert_eq!(mapped.values[0].shape, imported_shape);
    assert_eq!(
        mapped.assignment_plans[0].targets[0].storage,
        ResolvedAssignmentStorage::Existing(imported_place.clone())
    );
    assert_eq!(mapped.commits[0].previous, CfgPointId(13));
    assert_eq!(mapped.results[0].operation.producer, imported);
    assert_eq!(mapped.values[0].cases[0].dependencies[0].witness, witness);
    assert_eq!(
        mapped.values[0].cases[0].dependencies[0].source,
        DependencySource::FixedReferent {
            origin: ReferenceOriginId(6),
            loan: imported_loan
        }
    );
    let encoded = serde_json::to_vec(&mapped).expect("tables serialization");
    assert_eq!(
        serde_json::from_slice::<EvaluatedValueTables>(&encoded).expect("tables"),
        mapped
    );
    for (payload, expected) in [
        (
            EvaluatedPayloadFact::Callable {
                producers: vec![holder],
            },
            EvaluatedPayloadFact::Callable {
                producers: vec![imported_holder],
            },
        ),
        (
            EvaluatedPayloadFact::RuntimeBacking {
                backing,
                elements: ElementViewFact {
                    element: shape,
                    backing: runtime.clone(),
                    selections: vec![EvaluatedElementSelection {
                        selector: id.clone(),
                        case: case_id,
                    }],
                },
            },
            EvaluatedPayloadFact::RuntimeBacking {
                backing: imported_backing,
                elements: ElementViewFact {
                    element: imported_shape.clone(),
                    backing: imported_runtime,
                    selections: vec![EvaluatedElementSelection {
                        selector: remap.value(&id).expect("value"),
                        case: remap.case(&case.id).expect("case"),
                    }],
                },
            },
        ),
    ] {
        let mut payload_case = case.clone();
        payload_case.payload = payload;
        assert_eq!(
            remap
                .evaluated_case(&payload_case)
                .expect("payload")
                .payload,
            expected
        );
    }
    let family = OwnerRecordFamilyId {
        producer: producer.clone(),
        creation: CfgPointId(2),
        projection: vec![ValueProjection::RuntimeIndex {
            selector: id.clone(),
        }],
    };
    for dependency_source in [
        DependencySource::Allocation(family),
        DependencySource::Environment(environment),
        DependencySource::BindingCell(binding.clone()),
        DependencySource::Region {
            instance: binding.clone(),
            membership: runtime,
        },
        DependencySource::Manual {
            boundary: witness.clone(),
        },
    ] {
        for action in [
            DependencyAction::PromoteShared {
                source_owner: capability.clone(),
                receiving_owner: capability.clone(),
            },
            DependencyAction::TransferUnique {
                source_owner: capability.clone(),
                receiving_owner: capability.clone(),
            },
            DependencyAction::BorrowFixed {
                referent_lifetime: FixedLifetime::StaticStorage(witness.clone()),
                use_owner: capability.clone(),
            },
            DependencyAction::RegionMembership {
                region_owner: capability.clone(),
                use_owner: capability.clone(),
            },
            DependencyAction::ManualRaw {
                boundary: witness.clone(),
            },
        ] {
            let mut dependency_case = case.clone();
            dependency_case.dependencies = vec![EvaluatedDependencyFact {
                projection: vec![ValueProjection::RuntimeIndex {
                    selector: id.clone(),
                }],
                source: dependency_source.clone(),
                action,
                witness: witness.clone(),
            }];
            let mapped = remap
                .evaluated_case(&dependency_case)
                .expect("dependency actions");
            assert_eq!(mapped.dependencies[0].witness, witness);
            assert_eq!(
                mapped.dependencies[0].projection,
                vec![ValueProjection::RuntimeIndex {
                    selector: remap.value(&id).expect("selector")
                }]
            );
            if !matches!(dependency_source, DependencySource::Manual { .. }) {
                assert_ne!(mapped.dependencies[0].source, dependency_source);
            }
        }
    }
    for payload in [
        EvaluatedPayloadFact::Plain,
        EvaluatedPayloadFact::Raw {
            provenance: RawValueProvenance::Static {
                source: witness.clone(),
            },
        },
        EvaluatedPayloadFact::Raw {
            provenance: RawValueProvenance::CompilerStorage {
                source_place: place.clone(),
                creation: CfgPointId(2),
            },
        },
        EvaluatedPayloadFact::Raw {
            provenance: RawValueProvenance::ManualBoundary {
                operation: witness.clone(),
            },
        },
        EvaluatedPayloadFact::FixedAggregate {
            members: vec![EvaluatedMemberFact {
                projection: vec![ValueProjection::FixedIndex(1)],
                value: id.clone(),
            }],
        },
    ] {
        for availability in [
            ValueAvailability::Available,
            ValueAvailability::Uninitialized {
                source: witness.clone(),
            },
            ValueAvailability::PartiallyUnavailable {
                states: vec![
                    SubplaceAvailability {
                        projection: vec![ValueProjection::RuntimeIndex {
                            selector: id.clone(),
                        }],
                        state: SubplaceState::Moved {
                            point: CfgPointId(3),
                            source: witness.clone(),
                        },
                    },
                    SubplaceAvailability {
                        projection: vec![ValueProjection::FixedIndex(0)],
                        state: SubplaceState::Released {
                            point: CfgPointId(3),
                            source: witness.clone(),
                        },
                    },
                ],
            },
        ] {
            let mut payload_case = case.clone();
            payload_case.payload = payload.clone();
            payload_case.availability = availability;
            let mapped = remap
                .evaluated_case(&payload_case)
                .expect("payload and availability");
            assert_eq!(mapped.path.producer, imported);
            let encoded = serde_json::to_vec(&mapped).expect("case serialization");
            assert_eq!(
                serde_json::from_slice::<EvaluatedValueCase>(&encoded).expect("case"),
                mapped
            );
        }
    }
    let mut incomplete = remap.clone();
    incomplete.imports[0].loans.clear();
    assert_eq!(
        incomplete.mechanical_schema_tables(&sources),
        Err(ValueSchemaRemapError::MissingLoan(loan))
    );
    incomplete = remap.clone();
    incomplete.points.clear();
    assert!(matches!(
        incomplete.mechanical_schema_tables(&sources),
        Err(ValueSchemaRemapError::MissingPoint { .. })
    ));
}

fn aggregate_shapes(layout: ScalarTargetLayout, project: bool) {
    let mut program = schema_program(layout);
    let text = "%%start\nstruct copy Record { i32 value; u8[2] bytes; }\nenum copy Status { ready; done; }\n%%end";
    let parsed = crate::parse_source(program.source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let structure_id = ScalarStructId {
        source: program.source.clone(),
        index: 0,
    };
    let enum_id = ScalarEnumId {
        source: program.source.clone(),
        index: 0,
    };
    program.structs.push(derive_struct(
        parsed
            .result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::StructDecl)
            .expect("struct"),
        structure_id.clone(),
        layout,
    ));
    program.enums.push(derive_enum(
        parsed
            .result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::EnumDecl)
            .expect("enum"),
        enum_id.clone(),
    ));
    let mut diagnostics = Vec::new();
    resolve_program_types(&mut program, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let module = ScalarModule::from_program(program.clone(), Vec::new());
    let (structs, enums) = if project {
        (&module.structs, &module.enums)
    } else {
        (&program.structs, &program.enums)
    };
    let producer = ProducerCfgOwnerId::Module(program.source.clone());
    let mut shapes = Vec::new();
    let structure = intern_value_shape(
        &mut shapes,
        &producer,
        &ScalarType::Struct(structure_id.clone()),
        structs,
        enums, &producer, &[],
    )
    .expect("struct shape");
    let enumeration = intern_value_shape(
        &mut shapes,
        &producer,
        &ScalarType::Enum(enum_id.clone()),
        structs,
        enums, &producer, &[],
    )
    .expect("enum shape");
    let ValuePayloadShape::Struct { fields, .. } = &shapes
        .iter()
        .find(|shape| shape.id == structure)
        .expect("qualified struct")
        .payload
    else {
        panic!("struct")
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].field, structs[0].fields[0].id);
    assert!(matches!(
        shapes
            .iter()
            .find(|shape| shape.id == fields[1].shape)
            .expect("array")
            .payload,
        ValuePayloadShape::FixedArray { length: 2, .. }
    ));
    let ValuePayloadShape::Enum { variants, .. } = &shapes
        .iter()
        .find(|shape| shape.id == enumeration)
        .expect("qualified enum")
        .payload
    else {
        panic!("enum")
    };
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.ordinal)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert!(variants.iter().all(|variant| variant.fields.is_empty()));
    let expression = schema_expression(
        "(old_record, old_status = next_record, next_status)",
        &program.source,
    );
    let scope = BTreeMap::from([
        (
            "old_record".into(),
            ScalarType::Struct(structure_id.clone()),
        ),
        (
            "next_record".into(),
            ScalarType::Struct(structure_id.clone()),
        ),
        ("old_status".into(), ScalarType::Enum(enum_id.clone())),
        ("next_status".into(), ScalarType::Enum(enum_id.clone())),
    ]);
    let expected = vec![ScalarType::Struct(structure_id), ScalarType::Enum(enum_id)];
    let outputs = if project {
        typecheck::outputs::sequence_in_module(
            &expression,
            &expected,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &module,
            &[module.clone()],
            &mut diagnostics,
            false,
        )
    } else {
        typecheck::outputs::sequence(
            &expression,
            &expected,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            false,
        )
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        outputs
            .outputs
            .iter()
            .map(|output| output.ty.clone())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn compositional_assignment_result_schema_aggregate_shapes_program_native64() {
    aggregate_shapes(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn compositional_assignment_result_schema_aggregate_shapes_program_wasm32() {
    aggregate_shapes(ScalarTargetLayout::WASM32, false);
}

#[test]
fn compositional_assignment_result_schema_aggregate_shapes_project_native64() {
    aggregate_shapes(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn compositional_assignment_result_schema_aggregate_shapes_project_wasm32() {
    aggregate_shapes(ScalarTargetLayout::WASM32, true);
}

#[test]
fn compositional_assignment_result_schema_const_type_distinctness_and_shadow_labels() {
    let program = schema_program(ScalarTargetLayout::NATIVE64);
    let scope = BTreeMap::from([
        ("LIMIT".into(), ScalarType::I32),
        ("x".into(), ScalarType::I32),
    ]);
    for (text, code) in [
        ("(LIMIT = 4)", "B0007"),
        ("(x = false)", "B0003"),
        ("(x, x = 3, 4)", "B0002"),
    ] {
        let expression = schema_expression(text, &program.source);
        let mut diagnostics = Vec::new();
        typecheck::expression_type(
            &expression,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            false,
        );
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.code == code),
            "{text}: {diagnostics:?}"
        );
        assert!(diagnostics
            .iter()
            .flat_map(|diagnostic| &diagnostic.labels)
            .all(|label| label.span.source == program.source));
    }
    let expression = schema_expression("(fresh = 3)", &program.source);
    let ScalarExpression::Assignment(assignment) = expression else {
        panic!("assignment")
    };
    let mut diagnostics = Vec::new();
    typecheck::assignment_output_types(
        &assignment,
        &mut BTreeMap::new(),
        &BTreeSet::from(["fresh".into()]),
        &BTreeMap::from([("fresh".into(), ("fresh".into(), ByteSpan::new(1, 2)))]),
        &program,
        &mut diagnostics,
        false,
        false,
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B0002"),
        "{diagnostics:?}"
    );
}

#[test]
fn compositional_assignment_result_schema_static_use_inferred_targets_and_nested_operands() {
    let source = source("src/static.w");
    for (text, unused) in [
        ("i32(i32) use_old = fn(value) { (value = 4) }", false),
        (
            "i32(i32) use_rhs = fn(value) { fresh = (value = 4); fresh }",
            false,
        ),
        (
            "unit(i32) unused_parameter = fn(value) { fresh = 3; }",
            true,
        ),
    ] {
        let parsed = crate::parse_source(source.clone(), format!("%%start\n{text};\n%%end"), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let node = parsed
            .result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::FunctionDecl)
            .expect("function");
        let function = derive_function(&node, &source);
        let spans = typecheck::unused_binding_spans(&[ScalarItem::Function(function)]);
        assert_eq!(!spans.is_empty(), unused, "{text}: {spans:?}");
    }
}

#[test]
fn compositional_assignment_result_schema_contextual_overload_second_output_and_flattened_inputs() {
    let mut program = schema_program(ScalarTargetLayout::NATIVE64);
    let text = "%%start\nchoose = overload { (i32, u64)() => fn { 1, 2 }; (i32, bool)() => fn { 1, true }; };\naccept = overload { bool(i32, u64) => fn(left, right) { left; right; true }; };\n%%end";
    let parsed = crate::parse_source(program.source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for node in parsed
        .result
        .root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::OverloadDecl)
    {
        program.items.push(ScalarItem::Function(derive_overload(
            &node,
            &program.source,
        )));
    }
    let scope = BTreeMap::from([("x".into(), ScalarType::I32), ("y".into(), ScalarType::U64)]);
    let module = ScalarModule::from_program(program.clone(), Vec::new());
    for project in [false, true] {
        for (text, expected) in [
            ("choose()", vec![ScalarType::I32, ScalarType::U64]),
            ("accept((x, y = 3, 4))", vec![ScalarType::Bool]),
        ] {
            let expression = schema_expression(text, &program.source);
            let mut diagnostics = Vec::new();
            let outputs = if project {
                typecheck::outputs::sequence_in_module(
                    &expression,
                    &expected,
                    &scope,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    &module,
                    &[module.clone()],
                    &mut diagnostics,
                    false,
                )
            } else {
                typecheck::outputs::sequence(
                    &expression,
                    &expected,
                    &scope,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    &program,
                    &mut diagnostics,
                    false,
                )
            };
            assert!(diagnostics.is_empty(), "{text}: {diagnostics:?}");
            assert_eq!(
                outputs
                    .outputs
                    .iter()
                    .map(|output| output.ty.clone())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn compositional_assignment_result_schema_array_members_flatten_assignment_outputs() {
    let program = schema_program(ScalarTargetLayout::NATIVE64);
    let scope = BTreeMap::from([("x".into(), ScalarType::U32), ("y".into(), ScalarType::U32)]);
    let module = ScalarModule::from_program(program.clone(), Vec::new());
    let expression = schema_expression("[(x, y = 3, 4)]", &program.source);
    let span = ByteSpan::new(1, 2);
    let expected = ScalarType::Array {
        element: Box::new(ScalarType::U32),
        length: 2,
        length_span: span,
        span,
    };
    for project in [false, true] {
        let mut diagnostics = Vec::new();
        let outputs = if project {
            typecheck::outputs::sequence_in_module(
                &expression,
                std::slice::from_ref(&expected),
                &scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &module,
                &[module.clone()],
                &mut diagnostics,
                false,
            )
        } else {
            typecheck::outputs::sequence(
                &expression,
                std::slice::from_ref(&expected),
                &scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                &program,
                &mut diagnostics,
                false,
            )
        };
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(outputs.outputs.len(), 1);
        assert_eq!(outputs.outputs[0].ty, expected);
    }
}

#[test]
fn compositional_assignment_result_schema_final_projection_positions_and_canonical_rhs_projection()
{
    let program = schema_program(ScalarTargetLayout::NATIVE64);
    let parsed = crate::parse_source(
        program.source.clone(),
        "%%start\nfn { (x, y = 3, 4), true };\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Expression)
        .expect("expression");
    let mut expression = derive_expression(&node, &program.source);
    let span = ByteSpan::new(1, 2);
    let expected = ScalarType::Callable {
        outputs: ScalarOutputSequence {
            outputs: [ScalarType::I32, ScalarType::U64, ScalarType::Bool]
                .into_iter()
                .map(|ty| ScalarOutput { ty, span })
                .collect(),
            span,
        },
        parameters: Vec::new(),
    };
    let scope = BTreeMap::from([("x".into(), ScalarType::I32), ("y".into(), ScalarType::U64)]);
    let mut diagnostics = Vec::new();
    typecheck::outputs::sequence(
        &expression,
        std::slice::from_ref(&expected),
        &scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        &program,
        &mut diagnostics,
        false,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let facts = program.resolved_expression_outputs.borrow();
    record_expression_overload_selection(
        &mut expression,
        &scope,
        &BTreeMap::new(),
        &facts,
        &program.source,
    );
    let ScalarExpression::FunctionLiteral { body, .. } = expression else {
        panic!("literal")
    };
    assert_eq!(
        body.final_output_values
            .iter()
            .map(|value| value.position)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(body.final_output_values[1].ty, ScalarType::Bool);
    let ScalarBlockItem::Expression(ScalarExpression::Assignment(assignment)) = &body.items[0]
    else {
        panic!("assignment")
    };
    let assignment_node = node
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Assignment)
        .expect("canonical assignment");
    let output_list = assignment_node
        .children()
        .find(|node| node.kind() == SyntaxKind::OutputList)
        .expect("assignment output list");
    let rhs_spans = output_list
        .children()
        .filter(|node| node.kind() == SyntaxKind::Expression)
        .map(|node| wosy_syntax::byte_span(&node))
        .collect::<Vec<_>>();
    assert_eq!(assignment.values.len(), 2);
    assert_eq!(rhs_spans.len(), 2);
    assert_eq!(
        assignment.values[0],
        ScalarExpression::Integer {
            value: 3.into(),
            span: rhs_spans[0],
        }
    );
    assert_eq!(
        assignment.values[1],
        ScalarExpression::Integer {
            value: 4.into(),
            span: rhs_spans[1],
        }
    );
    assert_eq!(body.expressions[0], body.final_output_values[0].value);
}

#[test]
fn compositional_assignment_result_schema_qualified_output_selection_and_final_projection_equal_ranges(
) {
    let first = source("src/first-output.w");
    let second = source("src/second-output.w");
    let text = "fn(value) { pick(value), value }";
    let first_expression = schema_expression(text, &first);
    let second_expression = schema_expression(text, &second);
    assert_eq!(span_of(&first_expression), span_of(&second_expression));
    let parsed = crate::parse_source(first.clone(),
        "%%start\npick = overload { (i32, u64)(u32) => fn(value) { value; 1, 2 }; bool(bool) => fn(value) { value }; };\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::OverloadDecl)
        .expect("overload");
    let overloads = BTreeMap::from([("pick".into(), derive_overload(&node, &first))]);
    let mut facts = Vec::new();
    for (source, expression, input, call_types) in [
        (
            &first,
            &first_expression,
            ScalarType::U32,
            vec![ScalarType::I32, ScalarType::U64],
        ),
        (
            &second,
            &second_expression,
            ScalarType::Bool,
            vec![ScalarType::Bool],
        ),
    ] {
        let ScalarExpression::FunctionLiteral { body, .. } = expression else {
            panic!("literal")
        };
        let call = &body.final_output_values[0].value;
        let ScalarExpression::Call { arguments, .. } = call else {
            panic!("call")
        };
        let final_value = &body.final_output_values[1].value;
        let call_outputs = ScalarOutputSequence {
            outputs: call_types
                .into_iter()
                .map(|ty| ScalarOutput {
                    ty,
                    span: span_of(call),
                })
                .collect(),
            span: span_of(call),
        };
        let final_outputs = ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty: input.clone(),
                span: span_of(final_value),
            }],
            span: span_of(final_value),
        };
        let mut literal_outputs = call_outputs.outputs.clone();
        literal_outputs.extend(final_outputs.outputs.clone());
        facts.extend([
            ResolvedExpressionOutputFact {
                source_span: SourceSpan::new(source.clone(), span_of(expression)),
                outputs: ScalarOutputSequence {
                    outputs: vec![ScalarOutput {
                        ty: ScalarType::Callable {
                            parameters: vec![input.clone()],
                            outputs: ScalarOutputSequence {
                                outputs: literal_outputs,
                                span: body.span,
                            },
                        },
                        span: span_of(expression),
                    }],
                    span: span_of(expression),
                },
            },
            ResolvedExpressionOutputFact {
                source_span: SourceSpan::new(source.clone(), span_of(call)),
                outputs: call_outputs,
            },
            ResolvedExpressionOutputFact {
                source_span: SourceSpan::new(source.clone(), span_of(&arguments[0])),
                outputs: ScalarOutputSequence {
                    outputs: vec![ScalarOutput {
                        ty: input,
                        span: span_of(&arguments[0]),
                    }],
                    span: span_of(&arguments[0]),
                },
            },
            ResolvedExpressionOutputFact {
                source_span: SourceSpan::new(source.clone(), span_of(final_value)),
                outputs: final_outputs,
            },
        ]);
    }
    for index in 0..4 {
        assert_eq!(
            facts[index].source_span.range,
            facts[index + 4].source_span.range
        );
        assert_ne!(
            facts[index].source_span.source,
            facts[index + 4].source_span.source
        );
        assert_ne!(facts[index].outputs, facts[index + 4].outputs);
    }
    for (source, mut expression, arm, first_type, final_type, final_position) in [
        (
            &first,
            first_expression,
            0,
            ScalarType::I32,
            ScalarType::U32,
            2,
        ),
        (
            &second,
            second_expression,
            1,
            ScalarType::Bool,
            ScalarType::Bool,
            1,
        ),
    ] {
        record_expression_overload_selection(
            &mut expression,
            &BTreeMap::new(),
            &overloads,
            &facts,
            source,
        );
        let ScalarExpression::FunctionLiteral { body, .. } = expression else {
            panic!("literal")
        };
        let ScalarExpression::Call {
            overload_selection, ..
        } = &body.final_output_values[0].value
        else {
            panic!("call")
        };
        assert_eq!(
            overload_selection
                .as_ref()
                .expect("qualified selection")
                .arm_index,
            arm
        );
        assert_eq!(body.final_output_values[0].ty, first_type);
        assert_eq!(body.final_output_values[0].position, 0);
        assert_eq!(body.final_output_values[1].ty, final_type);
        assert_eq!(body.final_output_values[1].position, final_position);
        for (mirror, item) in body.final_output_values.iter().zip(&body.items) {
            let ScalarBlockItem::Expression(expression) = item else {
                panic!("final item")
            };
            assert_eq!(&mirror.value, expression);
        }
        assert_eq!(body.expressions[0], body.final_output_values[0].value);
        assert_eq!(body.expressions[1], body.final_output_values[1].value);
    }
}
