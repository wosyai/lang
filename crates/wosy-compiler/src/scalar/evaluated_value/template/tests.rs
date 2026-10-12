use super::*;
use crate::scalar::reference::source_epoch::{SourceEpoch, SourceEpochError};

fn context() -> SourceCapturePolicyContext {
    let source = SourceIdentity::new("template".into(), "proof".into(), "main.w".into(), "r1".into());
    SourceCapturePolicyContext {
        structs: Vec::new(), enums: Vec::new(),
        generic_scope: ProducerCfgOwnerId::Function { source, declaration: ByteSpan::new(10, 90), body: ProducerFunctionBodyId::Declaration },
        generic_declarations: vec![ScalarGenericParameter { name: "T".into(), span: ByteSpan::new(2, 3) }],
        generic_parameters: BTreeSet::from(["T".into()]),
    }
}

fn named() -> ScalarType { ScalarType::Named { name: "T".into(), span: ByteSpan::new(20, 21) } }

fn intern(shapes: &mut Vec<ResolvedValueShape>, ty: &ScalarType, context: &SourceCapturePolicyContext) -> Result<ValueShapeId, ValueShapeError> {
    intern_value_shape(shapes, &context.generic_scope, ty, &context.structs, &context.enums, &context.generic_scope, &context.generic_declarations)
}

#[test]
fn template_declared_identity_nested_reuse_and_serde() {
    let context = context();
    let ty = ScalarType::RawPointer(Box::new(ScalarType::RawPointer(Box::new(named()))));
    let mut shapes = Vec::new();
    let root = intern(&mut shapes, &ty, &context).unwrap();
    assert_eq!(root.index, 0);
    assert_eq!(shapes.iter().map(|row| row.id.index).collect::<Vec<_>>(), vec![0, 1, 2]);
    let original = shapes.clone();
    assert_eq!(intern(&mut shapes, &ty, &context).unwrap(), root);
    assert_eq!(shapes, original);
    let leaf = shapes.iter().find(|row| row.ty == named()).unwrap();
    assert_eq!(leaf.payload, ValuePayloadShape::TemplateParameter { scope: context.generic_scope.clone(), declaration: declaration_span(&context.generic_scope, &context.generic_declarations, "T").unwrap() });
    let encoded = serde_json::to_string(&(shapes.clone(), context.clone())).unwrap();
    let decoded: (Vec<ResolvedValueShape>, SourceCapturePolicyContext) = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, (original, context));
}

#[test]
fn template_undeclared_qualified_error_are_atomic() {
    let context = context();
    let undeclared = ScalarType::Named { name: "Missing".into(), span: ByteSpan::new(20, 21) };
    let qualified = ScalarType::Qualified { receiver: "dep".into(), receiver_span: ByteSpan::new(1, 4), member: "T".into(), member_span: ByteSpan::new(5, 6), span: ByteSpan::new(1, 6) };
    for ty in [undeclared, qualified, ScalarType::Error, ScalarType::Named { name: "T".into(), span: ByteSpan::new(20, 20) }] {
        let mut shapes = Vec::new();
        intern(&mut shapes, &ScalarType::Bool, &context).unwrap();
        let original = shapes.clone();
        assert_eq!(intern(&mut shapes, &ty, &context), Err(ValueShapeError::UnresolvedType(ty)));
        assert_eq!(shapes, original);
    }
}

#[test]
fn template_crossed_reused_leaf_and_nested_edge_reject_atomically() {
    let context = context();
    let ty = ScalarType::RawPointer(Box::new(named()));
    let mut shapes = Vec::new();
    intern(&mut shapes, &ty, &context).unwrap();
    for defect in 0..4 {
        let mut candidate = shapes.clone();
        match defect {
            0 => candidate[1].payload = ValuePayloadShape::Plain(PlainValueKind::Bool),
            1 => {
                let ValuePayloadShape::TemplateParameter { declaration, .. } = &mut candidate[1].payload else { unreachable!(); };
                declaration.range = ByteSpan::new(4, 5);
            }
            2 => {
                let ValuePayloadShape::TemplateParameter { scope, .. } = &mut candidate[1].payload else { unreachable!(); };
                let ProducerCfgOwnerId::Function { declaration, .. } = scope else { unreachable!(); };
                *declaration = ByteSpan::new(100, 200);
            }
            3 => candidate[0].payload = ValuePayloadShape::Raw { pointee: candidate[0].id.clone() },
            _ => unreachable!(),
        }
        let original = candidate.clone();
        assert!(intern(&mut candidate, &ty, &context).is_err(), "defect={defect}");
        assert_eq!(candidate, original);
    }
}

#[test]
fn template_context_exact_set_cardinality_scope_and_declaration_checks() {
    let context = context();
    assert!(valid_context(&context));
    for defect in 0..5 {
        let mut candidate = context.clone();
        match defect {
            0 => candidate.generic_parameters.clear(),
            1 => { candidate.generic_parameters.insert("Other".into()); }
            2 => candidate.generic_declarations.push(candidate.generic_declarations[0].clone()),
            3 => candidate.generic_declarations[0].span = ByteSpan::new(2, 2),
            4 => {
                let ProducerCfgOwnerId::Function { source, .. } = candidate.generic_scope else { unreachable!(); };
                candidate.generic_scope = ProducerCfgOwnerId::Module(source);
            }
            _ => unreachable!(),
        }
        assert!(!valid_context(&candidate), "defect={defect}");
    }
}

fn parsed_module(text: &str) -> ScalarModule {
    let source = SourceIdentity::new("template".into(), "actual".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(source, text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validated = derive_scalar_program(&parsed.result);
    assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
    ScalarModule::from_program(validated.program, Vec::new())
}

fn collect_cfgs<'a>(cfg: &'a ScalarReferenceCfg, rows: &mut Vec<&'a ScalarReferenceCfg>) {
    rows.push(cfg);
    for child in &cfg.literal_cfgs { collect_cfgs(&child.reference_cfg, rows); }
}

fn contexts(module: &ScalarModule) -> Vec<SourceCapturePolicyContext> {
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).unwrap();
    ValueSchemaRemap::validate_construction_forest(&forest).unwrap();
    let mut contexts = Vec::new();
    for row in forest.sources {
        if !contexts.iter().any(|context: &SourceCapturePolicyContext| context.generic_scope == row.policy_context.generic_scope) { contexts.push(row.policy_context); }
    }
    contexts
}

#[test]
fn template_actual_parsed_nested_capture_epoch_and_serde() {
    let module = parsed_module("%%start\ngeneric T;\nunit(T) run = fn(item) { unit() outer = fn { unit() inner = fn { item; }; inner(); }; outer(); };\n%%end");
    let contexts = contexts(&module);
    let mut cfgs = Vec::new();
    collect_cfgs(&module.top_level_reference_cfg, &mut cfgs);
    for item in &module.items { if let ScalarItem::Function(function) = item { collect_cfgs(&function.reference_cfg, &mut cfgs); } }
    let epoch = SourceEpoch::freeze(&cfgs, &module.structs, &module.enums, &contexts).unwrap();
    assert_eq!(epoch.captures().len(), 2);
    assert!(epoch.shapes().iter().any(|row| matches!(row.payload, ValuePayloadShape::TemplateParameter { .. })));
    for shape in epoch.shapes() {
        let context = SourceEpoch::lexical_context(&shape.id.producer, &contexts).unwrap();
        assert!(valid_node(shape, context));
    }
    let decoded: Vec<ScalarReferenceCfg> = serde_json::from_str(&serde_json::to_string(&cfgs).unwrap()).unwrap();
    let decoded_cfgs = decoded.iter().collect::<Vec<_>>();
    assert_eq!(SourceEpoch::freeze(&decoded_cfgs, &module.structs, &module.enums, &contexts).unwrap(), epoch);
}

#[test]
fn template_epoch_undeclared_and_crossed_at_undemanded_node() {
    let context = context();
    let mut cfg = ScalarReferenceCfg::new(context.generic_scope.clone());
    intern(&mut cfg.evaluated_values.shapes, &named(), &context).unwrap();
    let contexts = vec![context.clone()];
    SourceEpoch::freeze(&[&cfg], &[], &[], &contexts).unwrap();
    for defect in 0..4 {
        let mut candidate = cfg.clone();
        match defect {
            0 => candidate.evaluated_values.shapes[0].ty = ScalarType::Named { name: "Missing".into(), span: ByteSpan::new(20, 21) },
            1 => candidate.evaluated_values.shapes[0].payload = ValuePayloadShape::Plain(PlainValueKind::U8),
            2 => {
                let ValuePayloadShape::TemplateParameter { declaration, .. } = &mut candidate.evaluated_values.shapes[0].payload else { unreachable!(); };
                declaration.range = ByteSpan::new(8, 9);
            }
            3 => {
                let ValuePayloadShape::TemplateParameter { scope, .. } = &mut candidate.evaluated_values.shapes[0].payload else { unreachable!(); };
                let ProducerCfgOwnerId::Function { body, .. } = scope else { unreachable!(); };
                *body = ProducerFunctionBodyId::Overload(1);
            }
            _ => unreachable!(),
        }
        let before = candidate.clone();
        assert!(matches!(SourceEpoch::freeze(&[&candidate], &[], &[], &contexts), Err(SourceEpochError::UnresolvedShapeType { .. })));
        assert_eq!(candidate, before);
    }
}

#[test]
fn template_same_name_distinct_use_and_original_scope_stay_distinct() {
    let context = context();
    let mut shapes = Vec::new();
    let first = intern(&mut shapes, &named(), &context).unwrap();
    let later_type = ScalarType::Named { name: "T".into(), span: ByteSpan::new(40, 41) };
    let later = intern(&mut shapes, &later_type, &context).unwrap();
    assert_ne!(first, later);
    assert_eq!(shapes[0].payload, shapes[1].payload);
    assert_eq!(shapes[1].ty, later_type);
    let mut foreign = context.clone();
    let ProducerCfgOwnerId::Function { body, .. } = &mut foreign.generic_scope else { unreachable!(); };
    *body = ProducerFunctionBodyId::Overload(1);
    let mut foreign_shapes = Vec::new();
    intern(&mut foreign_shapes, &named(), &foreign).unwrap();
    assert_ne!(shapes[0].payload, foreign_shapes[0].payload);
    assert!(!valid_node(&foreign_shapes[0], &context));
}

#[test]
fn template_actual_capture_enum_topology_adds_zero_payload_authority() {
    let module = parsed_module("%%start\nenum copy Status { ready; done; }\nunit(Status) run = fn(status) { unit() read = fn { status; }; read(); };\n%%end");
    let contexts = contexts(&module);
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) => Some(function), _ => None }).unwrap();
    let mut cfgs = Vec::new();
    collect_cfgs(&function.reference_cfg, &mut cfgs);
    SourceEpoch::freeze(&cfgs, &module.structs, &module.enums, &contexts).unwrap();
    assert!(matches!(SourceEpoch::freeze(&cfgs, &module.structs, &[], &contexts), Err(SourceEpochError::CaptureShapeConstruction { error: ValueShapeError::MissingEnum(_), .. })));
    let mut malformed = function.reference_cfg.clone();
    let context = SourceEpoch::lexical_context(&malformed.producer_owner, &contexts).unwrap();
    let child = intern_value_shape(&mut malformed.evaluated_values.shapes, &malformed.producer_owner, &ScalarType::Bool, &module.structs, &module.enums, &context.generic_scope, &context.generic_declarations).unwrap();
    let shape = malformed.evaluated_values.shapes.iter_mut().find(|row| matches!(row.payload, ValuePayloadShape::Enum { .. })).unwrap();
    let ValuePayloadShape::Enum { variants, .. } = &mut shape.payload else { unreachable!(); };
    variants[0].fields.push(child);
    let before = malformed.clone();
    let mut cfgs = Vec::new();
    collect_cfgs(&malformed, &mut cfgs);
    assert!(matches!(SourceEpoch::freeze(&cfgs, &module.structs, &module.enums, &contexts), Err(SourceEpochError::CaptureShapeConstruction { error: ValueShapeError::UnresolvedType(_), .. })));
    assert_eq!(malformed, before);
}

#[test]
fn template_actual_capture_reused_nominal_closure_and_erasure() {
    let module = parsed_module("%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; unit() read = fn { pair.item; }; read(); };\n%%end");
    let contexts = contexts(&module);
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).unwrap();
    let mut cfgs = Vec::new();
    collect_cfgs(&function.reference_cfg, &mut cfgs);
    SourceEpoch::freeze(&cfgs, &module.structs, &module.enums, &contexts).unwrap();
    assert!(matches!(SourceEpoch::freeze(&cfgs, &[], &module.enums, &contexts), Err(SourceEpochError::CaptureShapeConstruction { error: ValueShapeError::MissingStruct(_), .. })));
    let mut malformed = function.reference_cfg.clone();
    let shape = malformed.evaluated_values.shapes.iter_mut().find(|row| matches!(row.payload, ValuePayloadShape::Struct { .. })).unwrap();
    let ValuePayloadShape::Struct { fields, .. } = &mut shape.payload else { unreachable!(); };
    fields.clear();
    let before = malformed.clone();
    let mut cfgs = Vec::new();
    collect_cfgs(&malformed, &mut cfgs);
    assert!(matches!(SourceEpoch::freeze(&cfgs, &module.structs, &module.enums, &contexts), Err(SourceEpochError::CaptureShapeConstruction { error: ValueShapeError::UnresolvedType(_), .. })));
    assert_eq!(malformed, before);
    let mut changed_declarations = module.structs.clone();
    changed_declarations[0].fields[0].name = "forged".into();
    let mut cfgs = Vec::new();
    collect_cfgs(&function.reference_cfg, &mut cfgs);
    assert!(matches!(SourceEpoch::freeze(&cfgs, &changed_declarations, &module.enums, &contexts), Err(SourceEpochError::UnresolvedShapeType { .. })));
}

#[test]
fn template_nested_nominal_generic_reused_fields_are_required() {
    let mut context = context();
    let ProducerCfgOwnerId::Function { source, .. } = &context.generic_scope else { unreachable!(); };
    let structure = ScalarStructId { source: source.clone(), index: 1 };
    context.structs.push(ScalarStruct {
        id: structure.clone(), name: "Boxed".into(), name_span: ByteSpan::new(4, 9),
        fields: vec![ScalarStructField { id: ScalarStructFieldId { structure: structure.clone(), index: 0 }, name: "item".into(), name_span: ByteSpan::new(10, 14), ty: ScalarType::RawPointer(Box::new(named())), declaration_index: 0, offset: None, layout: None }],
        copy_policy: CopyPolicy::Deferred, copy_modifier_span: None, span: ByteSpan::new(4, 30), layout: None,
    });
    let ty = ScalarType::Struct(structure);
    let mut shapes = Vec::new();
    let root = intern(&mut shapes, &ty, &context).unwrap();
    assert_eq!(intern(&mut shapes, &ty, &context).unwrap(), root);
    for defect in 0..3 {
        let mut malformed = shapes.clone();
        match defect {
            0 => {
                let ValuePayloadShape::Struct { fields, .. } = &mut malformed[0].payload else { unreachable!(); };
                fields.clear();
            }
            1 => { malformed.retain(|row| row.ty != named()); }
            2 => {
                let ValuePayloadShape::Struct { fields, .. } = &mut malformed[0].payload else { unreachable!(); };
                fields[0].shape.index = 999;
            }
            _ => unreachable!(),
        }
        let before = malformed.clone();
        assert!(intern(&mut malformed, &ty, &context).is_err(), "defect={defect}");
        assert_eq!(malformed, before);
    }
}

#[test]
fn template_pure_nonidentity_remap_preserves_original_declaration_and_use() {
    let context = context();
    let mut shapes = Vec::new();
    let original_id = intern(&mut shapes, &named(), &context).unwrap();
    let imported = ProducerCfgOwnerId::Module(SourceIdentity::new("transport".into(), "arena".into(), "import.w".into(), "r2".into()));
    let imported_id = ValueShapeId { producer: imported.clone(), index: 99 };
    let remap = ValueSchemaRemap { producers: vec![(context.generic_scope.clone(), imported)], points: Vec::new(), shapes: vec![(original_id, imported_id.clone())], imports: Vec::new() };
    let transported = remap.resolved_shape(&shapes[0]).unwrap();
    assert_eq!(transported.id, imported_id);
    assert_eq!(transported.ty, shapes[0].ty);
    assert_eq!(transported.payload, shapes[0].payload);
    assert!(valid_node(&transported, &context));
    let mut reused = vec![transported];
    let original = reused.clone();
    assert_eq!(intern_value_shape(&mut reused, &imported_id.producer, &named(), &[], &[], &context.generic_scope, &context.generic_declarations).unwrap(), imported_id);
    assert_eq!(reused, original);
}

#[test]
fn template_pointer_array_runtime_callable_declared_leaf_incidence() {
    let context = context();
    let types = vec![
        ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Shared, inner: Box::new(named()) },
        ScalarType::Array { element: Box::new(named()), length: 2, length_span: ByteSpan::new(30, 31), span: ByteSpan::new(20, 32) },
        ScalarType::RuntimeArray { element: Box::new(named()), span: ByteSpan::new(20, 32) },
        ScalarType::Callable { parameters: vec![ScalarType::RawPointer(Box::new(named()))], outputs: ScalarOutputSequence { outputs: vec![ScalarOutput { ty: named(), span: ByteSpan::new(20, 21) }], span: ByteSpan::new(20, 21) } },
    ];
    for ty in types {
        let mut shapes = Vec::new();
        let id = intern(&mut shapes, &ty, &context).unwrap();
        validate_shapes(&shapes, &context.generic_scope, &context.generic_scope, &context.generic_declarations).unwrap();
        let before = shapes.clone();
        assert_eq!(intern(&mut shapes, &ty, &context).unwrap(), id);
        assert_eq!(shapes, before);
        let leaf = shapes.iter_mut().find(|row| row.ty == named()).unwrap();
        leaf.payload = ValuePayloadShape::Plain(PlainValueKind::U8);
        let malformed = shapes.clone();
        assert!(intern(&mut shapes, &ty, &context).is_err());
        assert_eq!(shapes, malformed);
    }
}

#[test]
fn template_epoch_missing_and_duplicate_external_context_reject() {
    let context = context();
    let mut cfg = ScalarReferenceCfg::new(context.generic_scope.clone());
    intern(&mut cfg.evaluated_values.shapes, &named(), &context).unwrap();
    for contexts in [Vec::new(), vec![context.clone(), context.clone()]] {
        let before = cfg.clone();
        assert!(matches!(SourceEpoch::freeze(&[&cfg], &[], &[], &contexts), Err(SourceEpochError::MissingLexicalOwner { .. })));
        assert_eq!(cfg, before);
    }
}

#[test]
fn template_finite_output_nominal_topology_preserves_complete_vector_without_declarations() {
    let module = parsed_module("%%start\nu8() run = fn { 1 };\n%%end");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) => Some(function), _ => None }).unwrap();
    let context = contexts(&module);
    let original = &function.reference_cfg;
    let output = original.evaluated_values.values.last().unwrap();
    let root = output.shape.clone();
    let target = ValueShapeId { producer: root.producer.clone(), index: 99999 };
    let structure = ScalarStructId { source: module.source.clone(), index: 123 };
    let enumeration = ScalarEnumId { source: module.source.clone(), index: 123 };
    for (ty, payload) in [
        (ScalarType::Struct(structure.clone()), ValuePayloadShape::Struct { structure: structure.clone(), fields: vec![ValueShapeField { field: ScalarStructFieldId { structure, index: 0 }, shape: target.clone() }] }),
        (ScalarType::Enum(enumeration.clone()), ValuePayloadShape::Enum { enumeration, variants: vec![ValueShapeVariant { ordinal: 7, fields: vec![target.clone()] }] }),
    ] {
        let mut cfg = original.clone();
        cfg.evaluated_values.values = vec![output.clone()];
        cfg.evaluated_values.shapes = vec![ResolvedValueShape { id: root.clone(), ty, payload }];
        assert_eq!(SourceEpoch::freeze(&[&cfg], &[], &[], &context), Err(SourceEpochError::MissingShapeReference { shape: root.clone(), target: target.clone() }));
        cfg.evaluated_values.shapes.push(ResolvedValueShape { id: target.clone(), ty: ScalarType::Bool, payload: ValuePayloadShape::Plain(PlainValueKind::Bool) });
        let epoch = SourceEpoch::freeze(&[&cfg], &[], &[], &context).unwrap();
        assert_eq!(epoch.shapes(), cfg.evaluated_values.shapes);
        assert_eq!(epoch.outputs()[0].shape, root);
        assert!(epoch.captures().is_empty());
    }
}
