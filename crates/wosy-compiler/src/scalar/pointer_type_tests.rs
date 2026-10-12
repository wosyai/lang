use super::*;

fn support_source(path: &str) -> SourceIdentity {
    SourceIdentity::new(
        "pointer-type-support".into(),
        "app".into(),
        path.into(),
        "r1".into(),
    )
}

fn pointer_builder(
    text: &str,
    scope: BTreeMap<String, ScalarType>,
    structs: Vec<ScalarStruct>,
    layout: ScalarTargetLayout,
    project: bool,
    source: SourceIdentity,
) -> (ReferenceFlowBuilder, ScalarPlace, ScalarType) {
    let parsed = crate::parse_source(source.clone(), format!("%%start\n{text};\n%%end"), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::Expression)
        .expect("expression");
    let pointer = derive_expression(&node, &source);
    let span = expression_span(&pointer);
    let owner = ProducerCfgOwnerId::Module(source.clone());
    let program = ScalarProgram {
        source: source.clone(),
        items: Vec::new(),
        structs: structs.clone(),
        enums: Vec::new(),
        target_layout: layout,
        resolved_literals: RefCell::new(Vec::new()),
        resolved_assignment_types: RefCell::new(Vec::new()),
        resolved_expression_outputs: RefCell::new(Vec::new()),
        top_level_reference_cfg: ScalarReferenceCfg::new(owner.clone()),
        terminal_owners: Vec::new(),
        resolved_assignment_outputs: RefCell::new(HashMap::new()),
    };
    let mut diagnostics = Vec::new();
    let (ty, value_types) = if project {
        let module = ScalarModule::from_program(program, Vec::new());
        let modules = vec![module.clone()];
        let ty = typecheck::expression_type_in_module(
            &pointer,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &module,
            &modules,
            &mut diagnostics,
            true,
        );
        let value_types = assignment::SourceValueTypeFacts {
            assignments: module.resolved_assignment_types.borrow().clone(),
            expressions: module.resolved_expression_outputs.borrow().clone(),
        };
        (ty, value_types)
    } else {
        let ty = typecheck::expression_type(
            &pointer,
            &scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &program,
            &mut diagnostics,
            true,
        );
        let value_types = assignment::SourceValueTypeFacts {
            assignments: program.resolved_assignment_types.borrow().clone(),
            expressions: program.resolved_expression_outputs.borrow().clone(),
        };
        (ty, value_types)
    };
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let source_span = SourceSpan::new(source.clone(), span);
    assert_eq!(value_types.selected_output(&source_span, 0), Ok(&ty));
    let builder = ReferenceFlowBuilder {
        rebind_association_count: 0,
        rebind_associations: BTreeMap::new(),
        source,
        function_span: span,
        assignment_context_span: span,
        names: BTreeMap::new(),
        global_bindings: BTreeMap::new(),
        module_names: HashSet::new(),
        call_sites: Vec::new(),
        origins: HashMap::new(),
        checked: HashSet::new(),
        binding_types: HashMap::new(),
        structs,
        enums: Vec::new(),
        generic_scope: owner.clone(),
        generic_declarations: Vec::new(),
        generic_parameters: BTreeSet::new(),
        assignment_outputs: HashMap::new(),
        value_types,
        literal_signatures: Vec::new(),
        dereference_points: HashMap::new(),
        index_points: HashMap::new(),
        array_elements: Box::new(HashMap::new()),
        cfg: ScalarReferenceCfg::new(owner),
        next_scope: 0,
        cursor: None,
    };
    (
        builder,
        ScalarPlace::Dereference {
            pointer: Box::new(pointer),
            span,
        },
        ty,
    )
}

fn reference(mutability: ScalarReferenceMutability, inner: ScalarType) -> ScalarType {
    ScalarType::CheckedReference {
        mutability,
        inner: Box::new(inner),
    }
}

fn scoped(ty: &ScalarType) -> BTreeMap<String, ScalarType> {
    BTreeMap::from([
        ("target".into(), ty.clone()),
        ("replacement".into(), ty.clone()),
        ("choose".into(), ScalarType::Bool),
    ])
}

fn produced(text: &str, layout: ScalarTargetLayout, project: bool) {
    let expected = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
    let (builder, place, ty) = pointer_builder(
        text,
        scoped(&expected),
        Vec::new(),
        layout,
        project,
        support_source("src/main.w"),
    );
    assert_eq!(ty, expected);
    assert_eq!(builder.indexed_move_type(&place), Ok(ScalarType::U32));
    assert!(builder.cfg.points.is_empty());
    assert!(builder.cfg.facts.is_empty());
    assert!(builder.names.is_empty());
    assert!(builder.call_sites.is_empty());
}

#[test]
fn compositional_assignment_result_support_pointer_type_assignment_program_native64() {
    produced(
        "(target = replacement)",
        ScalarTargetLayout::NATIVE64,
        false,
    );
}
#[test]
fn compositional_assignment_result_support_pointer_type_assignment_program_wasm32() {
    produced("(target = replacement)", ScalarTargetLayout::WASM32, false);
}
#[test]
fn compositional_assignment_result_support_pointer_type_assignment_project_native64() {
    produced("(target = replacement)", ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn compositional_assignment_result_support_pointer_type_assignment_project_wasm32() {
    produced("(target = replacement)", ScalarTargetLayout::WASM32, true);
}

#[test]
fn compositional_assignment_result_support_pointer_type_branch_program_native64() {
    produced(
        "if (choose) { target } else { replacement }",
        ScalarTargetLayout::NATIVE64,
        false,
    );
}
#[test]
fn compositional_assignment_result_support_pointer_type_branch_program_wasm32() {
    produced(
        "if (choose) { target } else { replacement }",
        ScalarTargetLayout::WASM32,
        false,
    );
}
#[test]
fn compositional_assignment_result_support_pointer_type_branch_project_native64() {
    produced(
        "if (choose) { target } else { replacement }",
        ScalarTargetLayout::NATIVE64,
        true,
    );
}
#[test]
fn compositional_assignment_result_support_pointer_type_branch_project_wasm32() {
    produced(
        "if (choose) { target } else { replacement }",
        ScalarTargetLayout::WASM32,
        true,
    );
}

#[test]
fn compositional_assignment_result_support_pointer_type_block_program_native64() {
    produced("{ target }", ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn compositional_assignment_result_support_pointer_type_block_program_wasm32() {
    produced("{ target }", ScalarTargetLayout::WASM32, false);
}
#[test]
fn compositional_assignment_result_support_pointer_type_block_project_native64() {
    produced("{ target }", ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn compositional_assignment_result_support_pointer_type_block_project_wasm32() {
    produced("{ target }", ScalarTargetLayout::WASM32, true);
}

fn categories(layout: ScalarTargetLayout, project: bool) {
    for pointer_type in [
        reference(ScalarReferenceMutability::Shared, ScalarType::U32),
        reference(ScalarReferenceMutability::Mutable, ScalarType::U32),
        ScalarType::RawPointer(Box::new(ScalarType::U32)),
    ] {
        let (builder, place, ty) = pointer_builder(
            "target",
            scoped(&pointer_type),
            Vec::new(),
            layout,
            project,
            support_source("src/main.w"),
        );
        assert_eq!(ty, pointer_type);
        assert_eq!(builder.indexed_move_type(&place), Ok(ScalarType::U32));
    }
    let raw = ScalarType::RawPointer(Box::new(ScalarType::U8));
    let (builder, place, ty) = pointer_builder(
        "core.pointer_cast<*?u32>(target)",
        scoped(&raw),
        Vec::new(),
        layout,
        project,
        support_source("src/main.w"),
    );
    assert_eq!(ty, ScalarType::RawPointer(Box::new(ScalarType::U32)));
    assert_eq!(builder.indexed_move_type(&place), Ok(ScalarType::U32));
    let checked = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
    let array = ScalarType::Array {
        element: Box::new(checked.clone()),
        length: 2,
        length_span: ByteSpan::new(0, 0),
        span: ByteSpan::new(0, 0),
    };
    let (builder, place, ty) = pointer_builder(
        "target[0]",
        scoped(&array),
        Vec::new(),
        layout,
        project,
        support_source("src/main.w"),
    );
    assert_eq!(ty, checked);
    assert_eq!(builder.indexed_move_type(&place), Ok(ScalarType::U32));
    let (builder, base, _) = pointer_builder(
        "target",
        scoped(&reference(
            ScalarReferenceMutability::Mutable,
            array.clone(),
        )),
        Vec::new(),
        layout,
        project,
        support_source("src/main.w"),
    );
    let projected = ScalarPlace::Index {
        base: Box::new(base),
        index: Box::new(ScalarExpression::Integer {
            value: BigInt::from(0),
            span: ByteSpan::new(0, 0),
        }),
        span: ByteSpan::new(0, 0),
        index_span: ByteSpan::new(0, 0),
    };
    assert_eq!(builder.indexed_move_type(&projected), Ok(checked));
    assert!(builder.cfg.points.is_empty());
    let source = support_source("src/main.w");
    let parsed = crate::parse_source(
        source.clone(),
        "%%start\nstruct Holder { *!u32 pointer; u32 value; }\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let node = parsed
        .result
        .root
        .descendants()
        .find(|node| node.kind() == SyntaxKind::StructDecl)
        .expect("struct declaration");
    let structure = derive_struct(
        node,
        ScalarStructId {
            source: source.clone(),
            index: 0,
        },
        layout,
    );
    let structure_type = ScalarType::Struct(structure.id.clone());
    let (builder, place, ty) = pointer_builder(
        "target.pointer",
        scoped(&structure_type),
        vec![structure.clone()],
        layout,
        project,
        source.clone(),
    );
    assert_eq!(
        ty,
        reference(ScalarReferenceMutability::Mutable, ScalarType::U32)
    );
    assert_eq!(builder.indexed_move_type(&place), Ok(ScalarType::U32));
    let (builder, base, _) = pointer_builder(
        "(target = replacement)",
        scoped(&reference(
            ScalarReferenceMutability::Mutable,
            structure_type,
        )),
        vec![structure.clone()],
        layout,
        project,
        source,
    );
    for field in &structure.fields {
        let place = ScalarPlace::Field {
            base: Box::new(base.clone()),
            field: ScalarFieldReference::Resolved(field.id.clone()),
            span: field.name_span,
        };
        assert_eq!(builder.indexed_move_type(&place), Ok(field.ty.clone()));
    }
    assert!(builder.cfg.points.is_empty());
    assert!(builder.cfg.facts.is_empty());
}

#[test]
fn compositional_assignment_result_support_pointer_type_categories_program_native64() {
    categories(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn compositional_assignment_result_support_pointer_type_categories_program_wasm32() {
    categories(ScalarTargetLayout::WASM32, false);
}
#[test]
fn compositional_assignment_result_support_pointer_type_categories_project_native64() {
    categories(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn compositional_assignment_result_support_pointer_type_categories_project_wasm32() {
    categories(ScalarTargetLayout::WASM32, true);
}

#[test]
fn compositional_assignment_result_support_pointer_type_equal_offsets_distinct_sources() {
    let pointer = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
    let (left, place, _) = pointer_builder(
        "target",
        scoped(&pointer),
        Vec::new(),
        ScalarTargetLayout::NATIVE64,
        false,
        support_source("src/left.w"),
    );
    let (right, _, _) = pointer_builder(
        "target",
        scoped(&ScalarType::RawPointer(Box::new(ScalarType::U64))),
        Vec::new(),
        ScalarTargetLayout::NATIVE64,
        false,
        support_source("src/right.w"),
    );
    let mut combined = left;
    combined
        .value_types
        .expressions
        .extend(right.value_types.expressions);
    assert_eq!(combined.indexed_move_type(&place), Ok(ScalarType::U32));
    combined.source = right.source;
    assert_eq!(combined.indexed_move_type(&place), Ok(ScalarType::U64));
}

#[test]
fn compositional_assignment_result_support_pointer_type_missing_output_fact() {
    let pointer = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
    let (mut builder, place, _) = pointer_builder(
        "target",
        scoped(&pointer),
        Vec::new(),
        ScalarTargetLayout::NATIVE64,
        false,
        support_source("src/main.w"),
    );
    builder.value_types.expressions.clear();
    let error = builder
        .indexed_move_type(&place)
        .expect_err("missing typed correspondence");
    assert!(
        matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidPointerTypeCorrespondence } if source_span == SourceSpan::new(builder.source, expression_span(match &place { ScalarPlace::Dereference { pointer, .. } => pointer, _ => unreachable!() })))
    );
}

#[test]
fn compositional_assignment_result_support_pointer_type_empty_output_sequence() {
    let pointer = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
    let (mut builder, place, _) = pointer_builder(
        "target",
        scoped(&pointer),
        Vec::new(),
        ScalarTargetLayout::NATIVE64,
        false,
        support_source("src/main.w"),
    );
    builder.value_types.expressions[0].outputs.outputs.clear();
    assert!(matches!(
        builder.indexed_move_type(&place),
        Err(ReferenceAnalysisError::MissingContext {
            reason: ReferenceAnalysisErrorReason::InvalidPointerTypeCorrespondence,
            ..
        })
    ));
}

#[test]
fn compositional_assignment_result_support_pointer_type_duplicate_output_fact() {
    let pointer = reference(ScalarReferenceMutability::Mutable, ScalarType::U32);
    let (mut builder, place, _) = pointer_builder(
        "target",
        scoped(&pointer),
        Vec::new(),
        ScalarTargetLayout::NATIVE64,
        false,
        support_source("src/main.w"),
    );
    builder
        .value_types
        .expressions
        .push(builder.value_types.expressions[0].clone());
    assert!(matches!(
        builder.indexed_move_type(&place),
        Err(ReferenceAnalysisError::MissingContext {
            reason: ReferenceAnalysisErrorReason::InvalidPointerTypeCorrespondence,
            ..
        })
    ));
}

#[test]
fn compositional_assignment_result_support_pointer_type_invalid_source_type_recovery() {
    let (builder, place, ty) = pointer_builder(
        "target",
        scoped(&ScalarType::Bool),
        Vec::new(),
        ScalarTargetLayout::NATIVE64,
        false,
        support_source("src/main.w"),
    );
    assert_eq!(ty, ScalarType::Bool);
    assert!(matches!(
        builder.indexed_move_type(&place),
        Err(ReferenceAnalysisError::MissingContext {
            reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin,
            ..
        })
    ));
}
