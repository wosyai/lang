use super::*;

fn mapping(source: &[ValueTableSource]) -> ValueSchemaRemap {
    ValueSchemaRemap {
        producers: source.iter().map(|row| (row.producer.clone(), row.producer.clone())).collect(),
        points: source.iter().flat_map(|row| row.initial_points.iter().map(|point| ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: point.id })).collect(),
        shapes: source.iter().flat_map(|row| row.tables.shapes.iter().map(|shape| (shape.id.clone(), shape.id.clone()))).collect(),
        imports: Vec::new(),
    }
}

#[test]
fn ownerless_forests_reject_both_phases_before_factory_or_full_publication() {
    let sources = Vec::new();
    let remap = mapping(&sources);
    let saved_remap = remap.clone();
    let rooted_sources = [correction_tests::source("r1")];
    let rooted_forest = capture_source_tree_construction_forest(&rooted_sources[0].producer, &rooted_sources).expect("actual independent rooted forest");
    let saved_rooted = (rooted_sources.clone(), rooted_forest.clone());
    for phase in [SourceCaptureConstructionPhase::Building, SourceCaptureConstructionPhase::Complete] {
        let forest = SourceCaptureConstructionForest { phase, roots: Vec::new(), sources: Vec::new(), pending: Vec::new() };
        let saved_forest = forest.clone();
        assert_eq!(ValueSchemaRemap::validate_construction_forest(&forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.validate_forest_sources(&sources, &forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.stage_boundary_sources(&sources, &forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.independent_boundary_destination(&sources, &forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.tables(&sources, &forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.tables_into(&sources, &forest, &sources, &forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.tables_into(&sources, &forest, &rooted_sources, &rooted_forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(remap.tables_into(&rooted_sources, &rooted_forest, &sources, &forest), Err(ValueSchemaRemapError::EmptyConstructionForest));
        assert_eq!(forest, saved_forest);
        assert!(sources.is_empty());
        assert_eq!(remap, saved_remap);
        assert_eq!((rooted_sources.clone(), rooted_forest.clone()), saved_rooted);
    }
}

#[test]
fn empty_transport_rejects_against_actual_rooted_forest_on_all_full_routes() {
    let sources = [correction_tests::source("r1")];
    let remap = mapping(&sources);
    let original_forest = capture_source_tree_construction_forest(&sources[0].producer, &sources).expect("actual original typed constructor forest");
    for phase in [SourceCaptureConstructionPhase::Building, SourceCaptureConstructionPhase::Complete] {
        let mut forest = original_forest.clone();
        forest.phase = phase;
        let saved = (sources.clone(), forest.clone(), remap.clone());
        ValueSchemaRemap::validate_construction_forest(&forest).expect("actual nonempty forest remains valid");
        let witness = remap.stage_boundary_sources(&sources, &forest).expect("actual nonempty copied evidence");
        let saved_witness = witness.clone();
        assert_eq!(remap.validate_forest_sources(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!(remap.stage_boundary_sources(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!(remap.independent_boundary_destination(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!(remap.tables(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!(remap.tables_into(&[], &forest, &sources, &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!(remap.tables_into(&sources, &forest, &[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!(witness.checked(&remap, &[], &forest, &resources::demand::resource_demand_full(&[])), Err(ValueSchemaRemapError::EmptyTransportSources));
        assert_eq!((sources.clone(), forest.clone(), remap.clone()), saved);
        assert_eq!(witness, saved_witness);
    }
}

#[test]
fn retained_root_errors_precede_empty_transport_error() {
    let sources = [correction_tests::source("r1")];
    let remap = mapping(&sources);
    let mut forest = capture_source_tree_construction_forest(&sources[0].producer, &sources).expect("actual original typed constructor forest");
    forest.sources.clear();
    let saved = (sources.clone(), forest.clone(), remap.clone());
    let error = ValueSchemaRemapError::MissingProducer(sources[0].producer.clone());
    assert_eq!(ValueSchemaRemap::validate_construction_forest(&forest), Err(error.clone()));
    assert_eq!(remap.validate_forest_sources(&[], &forest), Err(error.clone()));
    assert_eq!(remap.stage_boundary_sources(&[], &forest), Err(error.clone()));
    assert_eq!(remap.independent_boundary_destination(&[], &forest), Err(error.clone()));
    assert_eq!(remap.tables(&[], &forest), Err(error.clone()));
    assert_eq!(remap.tables_into(&[], &forest, &[], &forest), Err(error));
    assert_eq!((sources.clone(), forest.clone(), remap.clone()), saved);
}

#[test]
fn genuine_pending_root_allows_standalone_building_validation_only() {
    let source = correction_tests::source("r1");
    let identity = source.policy_context.structs[0].id.source.clone();
    let parsed = crate::parse_source(identity.clone(), "%%start\nunit() pending = fn {};\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let syntax = parsed.result.root.descendants().find(|node| node.kind() == SyntaxKind::FunctionDecl).expect("actual pending function declaration");
    let declaration = derive_function(&syntax, &identity);
    let owner = declaration.reference_cfg.producer_owner.clone();
    assert!(declaration.reference_cfg.initial_points.is_empty());
    let forest = SourceCaptureConstructionForest {
        phase: SourceCaptureConstructionPhase::Building,
        roots: vec![SourceCaptureForestRoot { owner: owner.clone(), declaration: SourceSpan::new(identity, declaration.span) }],
        sources: Vec::new(), pending: vec![owner.clone()],
    };
    let remap = mapping(&[]);
    let saved = (forest.clone(), remap.clone());
    ValueSchemaRemap::validate_construction_forest(&forest).expect("independently required actual pending constructor");
    assert_eq!(remap.validate_forest_sources(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
    assert_eq!(remap.stage_boundary_sources(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
    assert_eq!(remap.independent_boundary_destination(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
    assert_eq!(remap.tables(&[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
    assert_eq!(remap.tables_into(&[], &forest, &[], &forest), Err(ValueSchemaRemapError::EmptyTransportSources));
    assert_eq!((forest.clone(), remap.clone()), saved);
    let mut complete = forest.clone();
    complete.phase = SourceCaptureConstructionPhase::Complete;
    assert_eq!(ValueSchemaRemap::validate_construction_forest(&complete), Err(ValueSchemaRemapError::MissingProducer(owner)));
}

#[test]
fn selective_root_precedence_survives_ownerless_raw_domain_guards() {
    let sources = [correction_tests::source("r1")];
    let root = &sources[0].producer;
    let remap = mapping(&sources);
    let forest = capture_source_tree_construction_forest(root, &sources).expect("actual original typed constructor forest");
    let witness = remap.stage_boundary_sources(&sources, &forest).expect("actual copied destination evidence");
    let empty_forest = SourceCaptureConstructionForest { phase: SourceCaptureConstructionPhase::Complete, roots: Vec::new(), sources: Vec::new(), pending: Vec::new() };
    let saved = (sources.clone(), forest.clone(), empty_forest.clone(), remap.clone(), witness.clone());
    assert_eq!(remap.source_use_graphs_into(root, &sources[0].tables, &[], &empty_forest, &witness), Err(ValueSchemaRemapError::MissingProducer(root.clone())));
    assert_eq!(remap.staged_loan_queries_into(root, &sources[0].tables, &[], &empty_forest, &witness), Err(ValueSchemaRemapError::MissingProducer(root.clone())));
    assert!(remap.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness).expect("real rooted empty graph result").graphs.is_empty());
    assert!(remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &witness).expect("real rooted empty query result").queries.is_empty());
    assert_eq!((sources.clone(), forest.clone(), empty_forest.clone(), remap.clone(), witness.clone()), saved);
}

#[test]
fn copied_witness_rejects_actual_source_mismatch_without_publication() {
    let sources = [correction_tests::source("r1")];
    let remap = mapping(&sources);
    let forest = capture_source_tree_construction_forest(&sources[0].producer, &sources).expect("original actual constructor forest");
    let witness = remap.stage_boundary_sources(&sources, &forest).expect("original actual copied evidence");
    let other = [correction_tests::source("r2")];
    let other_forest = capture_source_tree_construction_forest(&other[0].producer, &other).expect("other actual constructor forest");
    let saved = (sources.clone(), forest.clone(), other.clone(), other_forest.clone(), remap.clone(), witness.clone());
    let error = ValueSchemaRemapError::MissingProducer(other[0].producer.clone());
    assert_eq!(witness.checked(&remap, &other, &other_forest, &resources::demand::resource_demand_full(&other)), Err(error.clone()));
    assert_eq!(remap.source_use_graphs_into(&other[0].producer, &other[0].tables, &other, &other_forest, &witness), Err(error.clone()));
    assert_eq!(remap.staged_loan_queries_into(&other[0].producer, &other[0].tables, &other, &other_forest, &witness), Err(error));
    assert_eq!((sources.clone(), forest.clone(), other.clone(), other_forest.clone(), remap.clone(), witness.clone()), saved);
}

#[test]
fn copied_witness_rejects_changed_valid_forest_without_publication() {
    let sources = [correction_tests::source("r1")];
    let root = &sources[0].producer;
    let remap = mapping(&sources);
    let forest = capture_source_tree_construction_forest(root, &sources).expect("actual constructor forest");
    let witness = remap.stage_boundary_sources(&sources, &forest).expect("actual copied evidence");
    let mut changed = forest.clone();
    changed.phase = SourceCaptureConstructionPhase::Building;
    ValueSchemaRemap::validate_construction_forest(&changed).expect("changed forest is independently valid");
    let saved = (sources.clone(), forest.clone(), changed.clone(), remap.clone(), witness.clone());
    let error = ValueSchemaRemapError::MissingProducer(root.clone());
    assert_eq!(witness.checked(&remap, &sources, &changed, &resources::demand::resource_demand_full(&sources)), Err(error.clone()));
    assert_eq!(remap.source_use_graphs_into(root, &sources[0].tables, &sources, &changed, &witness), Err(error.clone()));
    assert_eq!(remap.staged_loan_queries_into(root, &sources[0].tables, &sources, &changed, &witness), Err(error));
    assert_eq!((sources.clone(), forest.clone(), changed.clone(), remap.clone(), witness.clone()), saved);
}

#[test]
fn copied_witness_rejects_changed_actual_remap_without_publication() {
    let sources = [correction_tests::source("r1")];
    let root = &sources[0].producer;
    let remap = mapping(&sources);
    let forest = capture_source_tree_construction_forest(root, &sources).expect("actual constructor forest");
    let witness = remap.stage_boundary_sources(&sources, &forest).expect("actual copied evidence");
    let mut changed = remap.clone();
    changed.producers[0].1 = correction_tests::source("r2").producer;
    let saved = (sources.clone(), forest.clone(), remap.clone(), changed.clone(), witness.clone());
    let error = ValueSchemaRemapError::MissingProducer(root.clone());
    assert_eq!(witness.checked(&changed, &sources, &forest, &resources::demand::resource_demand_full(&sources)), Err(error.clone()));
    assert_eq!(changed.source_use_graphs_into(root, &sources[0].tables, &sources, &forest, &witness), Err(error.clone()));
    assert_eq!(changed.staged_loan_queries_into(root, &sources[0].tables, &sources, &forest, &witness), Err(error));
    assert_eq!((sources.clone(), forest.clone(), remap.clone(), changed.clone(), witness.clone()), saved);
}
