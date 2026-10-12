use super::*;

#[test]
fn building_ordinary_null_records_all_deduplicated_events_without_typed_admission() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let identity = SourceIdentity::new("null-building".into(), "app".into(), "src/main.w".into(), "r1".into());
        let parsed = crate::parse_source(identity.clone(), "%%start\n*i32 value = null;\nvalue;\n%%end".into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let original = derive_scalar_program_with_layout(&parsed.result, layout);
        assert!(original.diagnostics.is_empty(), "{:?}", original.diagnostics);
        let expression = original.program.top_level_reference_cfg.source_null_constructors[0].expression.clone();
        let owner = ProducerCfgOwnerId::Module(identity.clone());
        let mut builder = ReferenceFlowBuilder {
            rebind_association_count: 0, rebind_associations: BTreeMap::new(),
            source: identity, function_span: ByteSpan::new(0, 0), assignment_context_span: ByteSpan::new(0, 0),
            names: BTreeMap::new(), global_bindings: BTreeMap::new(), module_names: HashSet::new(), call_sites: Vec::new(),
            origins: HashMap::new(), checked: HashSet::new(), binding_types: HashMap::new(),
            structs: Vec::new(), enums: Vec::new(), generic_scope: owner.clone(),
            generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(), assignment_outputs: HashMap::new(),
            value_types: assignment::SourceValueTypeFacts { assignments: Vec::new(), expressions: Vec::new() },
            literal_signatures: Vec::new(), dereference_points: HashMap::new(), index_points: HashMap::new(),
            array_elements: Box::new(HashMap::new()), cfg: ScalarReferenceCfg::new(owner), next_scope: 0, cursor: None,
        };
        for _ in 0..2 { builder.expression(&expression, ReferenceScopeId(0)).expect("ordinary Building constructor remains legal"); }
        assert_eq!(builder.cfg.facts.len(), 1);
        assert_eq!(builder.cfg.source_null_constructors.len(), 2);
        assert_eq!(builder.cfg.source_null_constructors[0].origin, builder.cfg.source_null_constructors[1].origin);
        assert_ne!(builder.cfg.source_null_constructors[0].event, builder.cfg.source_null_constructors[1].event);
        assert_eq!(builder.cfg.name_resolution_context, NameResolutionContext::Building);
        builder.cfg.initial_points = builder.cfg.points.clone();
        let saved = builder.cfg.clone();
        assert!(validate_null_constructors(&builder.cfg, &[]).is_err());
        assert_eq!(builder.cfg, saved);
    }
}
