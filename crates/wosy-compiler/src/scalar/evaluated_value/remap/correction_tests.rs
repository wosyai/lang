use super::*;

pub(super) fn source(revision: &str) -> ValueTableSource {
    let identity = SourceIdentity::new("c1-correction-proof".into(), "app".into(), "src/main.w".into(), revision.into());
    let parsed = crate::parse_source(identity.clone(), "%%start\nstruct copy Record { i32 value; u8[2] bytes; }\nstruct copy Other { i32 value; u8[2] bytes; }\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let program = validation.program;
    let producer = ProducerCfgOwnerId::Module(identity);
    let mut tables = EvaluatedValueTables::new();
    for declaration in &program.structs {
        intern_value_shape(&mut tables.shapes, &producer, &ScalarType::Struct(declaration.id.clone()), &program.structs, &program.enums, &producer, &[]).expect("actual canonical declaration graph");
    }
    ValueTableSource {
        resource_construction: capture_table_sources(&program.top_level_reference_cfg, &SourceCapturePolicyContext { structs: program.structs.clone(), enums: program.enums.clone(), generic_scope: producer.clone(), generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() }).expect("actual original parsed empty-resource CFG snapshot")[0].resource_construction.clone(),
        policy_context: SourceCapturePolicyContext { structs: program.structs, enums: program.enums, generic_scope: producer.clone(), generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() },
        producer, tables, capture_inventory: SourceCaptureInventory { phase: SourceCaptureConstructionPhase::Complete, required: Vec::new() },
        initial_points: Vec::new(), capture_points: Vec::new(), top_level: ScalarTopLevelCorrespondence::default(), children: Vec::new(),
    }
}

fn remap(source: &ValueTableSource, destination: &ValueTableSource) -> ValueSchemaRemap {
    assert_eq!(source.tables.shapes.len(), destination.tables.shapes.len());
    ValueSchemaRemap {
        producers: vec![(source.producer.clone(), destination.producer.clone())], points: Vec::new(),
        shapes: source.tables.shapes.iter().zip(&destination.tables.shapes).map(|(before, after)| (before.id.clone(), after.id.clone())).collect(), imports: Vec::new(),
    }
}

#[test]
fn explicit_qualified_shapes_validate_actual_canonical_fields() {
    let original = source("r1");
    let destination = source("r2");
    remap(&original, &destination).validate_qualified_shapes(&[original], &[destination]).expect("explicit complete qualified declaration graph");
}

#[test]
fn crossed_field_declaration_rejects_equal_field_indexes() {
    let original = source("r1");
    let mut destination = source("r2");
    let mapping = remap(&original, &destination);
    let other = destination.policy_context.structs[1].id.clone();
    let record = destination.policy_context.structs[0].id.clone();
    let shape = destination.tables.shapes.iter_mut().find(|row| row.ty == ScalarType::Struct(record.clone())).expect("actual Record shape");
    let id = original.tables.shapes.iter().find(|row| row.ty == ScalarType::Struct(original.policy_context.structs[0].id.clone())).expect("original Record shape").id.clone();
    let ValuePayloadShape::Struct { fields, .. } = &mut shape.payload else { panic!("struct payload"); };
    fields[0].field.structure = other;
    assert_eq!(mapping.validate_qualified_shapes(&[original], &[destination]), Err(ValueSchemaRemapError::MissingShape(id)));
}

#[test]
fn missing_field_shape_association_is_qualified_error() {
    let original = source("r1");
    let destination = source("r2");
    let mut mapping = remap(&original, &destination);
    let id = original.tables.shapes.iter().find(|row| row.ty == ScalarType::I32).expect("actual canonical field type").id.clone();
    mapping.shapes.retain(|(before, _)| before != &id);
    assert_eq!(mapping.validate_qualified_shapes(&[original], &[destination]), Err(ValueSchemaRemapError::MissingShape(id)));
}

#[test]
fn empty_selective_results_require_designated_exact_root() {
    let original = source("r1");
    let destination = source("r2");
    let mapping = remap(&original, &destination);
    let root = &original.producer;
    let sources = [original.clone()];
    let destinations = [destination.clone()];
    let forest = capture_source_tree_construction_forest(root, &sources).expect("original constructor forest");
    let destination_forest = capture_source_tree_construction_forest(&destination.producer, &destinations).expect("independent constructor forest");
    let witness = mapping.independent_boundary_destination(&destinations, &destination_forest).expect("independent destination evidence");
    assert!(mapping.source_use_graphs_into(root, &original.tables, &sources, &forest, &witness).expect("designated empty source-use root").graphs.is_empty());
    assert!(mapping.staged_loan_queries_into(root, &original.tables, &sources, &forest, &witness).expect("designated empty staged root").queries.is_empty());
    let error = Err(ValueSchemaRemapError::MissingProducer(root.clone()));
    assert_eq!(mapping.source_use_graphs_into(root, &original.tables, &[], &forest, &witness), error);
    assert_eq!(mapping.source_use_graphs_into(root, &original.tables, &[original.clone(), original.clone()], &forest, &witness), error);
    assert_eq!(mapping.source_use_graphs_into(root, &EvaluatedValueTables::new(), &sources, &forest, &witness), error);
    assert_eq!(mapping.staged_loan_queries_into(root, &original.tables, &[], &forest, &witness), Err(ValueSchemaRemapError::MissingProducer(root.clone())));
    assert_eq!(mapping.staged_loan_queries_into(root, &original.tables, &[original.clone(), original.clone()], &forest, &witness), Err(ValueSchemaRemapError::MissingProducer(root.clone())));
    assert_eq!(mapping.staged_loan_queries_into(root, &EvaluatedValueTables::new(), &sources, &forest, &witness), Err(ValueSchemaRemapError::MissingProducer(root.clone())));
    assert_eq!(mapping.source_use_graphs_into(&destination.producer, &original.tables, &sources, &forest, &witness), Err(ValueSchemaRemapError::MissingProducer(destination.producer.clone())));
}
