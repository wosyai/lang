use super::*;

#[test]
fn building_allocation_identity_records_all_compatible_dedup_creator_incidences() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let module = source_boundary::allocation_identity_constructor::tests::factory("building", source_boundary::allocation_identity_constructor::tests::SOURCES[2], layout, false);
        let original = &module.top_level_reference_cfg;
        let point = original.initial_points.iter().find(|point| matches!(point.kind, CfgPointKind::AllocationIdentityAddress { .. })).expect("actual parsed original allocation address");
        let CfgPointKind::AllocationIdentityAddress { request } = &point.kind else { unreachable!() };
        let binding = request.candidate.place.binding.clone();
        let ScalarPlace::Name { name, .. } = reference_root(&request.candidate.place.place) else { unreachable!() };
        let expression = ScalarExpression::CheckedAddress { mutability: request.candidate.loan.mode, place: request.candidate.place.place.clone(), span: point.source_span.range };
        let owner = original.producer_owner.clone();
        let mut builder = ReferenceFlowBuilder {
            rebind_association_count: 0, rebind_associations: BTreeMap::new(),
            source: module.source.clone(), function_span: ByteSpan::new(0, 0), assignment_context_span: ByteSpan::new(0, 0),
            names: BTreeMap::from([(name.clone(), binding)]), global_bindings: BTreeMap::new(), module_names: HashSet::new(), call_sites: Vec::new(),
            origins: HashMap::new(), checked: HashSet::new(), binding_types: HashMap::new(),
            structs: module.structs.clone(), enums: module.enums.clone(), generic_scope: owner.clone(),
            generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(), assignment_outputs: HashMap::new(),
            value_types: assignment::SourceValueTypeFacts { assignments: Vec::new(), expressions: Vec::new() },
            literal_signatures: Vec::new(), dereference_points: HashMap::new(), index_points: HashMap::new(),
            array_elements: Box::new(HashMap::new()), cfg: ScalarReferenceCfg::new(owner), next_scope: 0, cursor: None,
        };
        for ordinal in 0..2 { builder.allocation_identity_address(&expression, ReferenceRebindAssociationId(ordinal), ReferenceScopeId(0)).expect("actual writer records while Building"); }
        assert_eq!(builder.cfg.name_resolution_context, NameResolutionContext::Building);
        assert_eq!(builder.cfg.facts.len(), 1);
        assert_eq!(builder.cfg.evaluated_values.source_origin_derivations.len(), 2);
        let rows = &builder.cfg.evaluated_values.source_origin_derivations;
        assert_eq!(rows[0].origin(), rows[1].origin());
        assert_ne!(rows[0].event(), rows[1].event());
        for (ordinal, row) in rows.iter().enumerate() {
            let source_boundary::null_constructor::SourceConstructorOriginDerivation::AllocationIdentityAddress(row) = row else { panic!("genuine allocation writer") };
            assert_eq!(row.association.association, ReferenceRebindAssociationId(ordinal));
            assert_eq!(row.association.owner, builder.cfg.producer_owner);
            assert_eq!(builder.cfg.points[row.event.point.0].possible_origins, Some(BTreeSet::new()));
        }
        builder.cfg.initial_points = builder.cfg.points.clone();
        let saved = builder.cfg.clone();
        assert!(validate_null_constructors(&builder.cfg, &module.structs).is_err());
        assert_eq!(builder.cfg, saved);
    }
}
