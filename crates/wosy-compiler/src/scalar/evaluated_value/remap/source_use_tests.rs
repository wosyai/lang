use super::*;

#[test]
fn source_use_remap_replaces_every_qualified_graph_incidence() {
    let cfg = crate::scalar::common_loan::tests::cfg("%%start\nunit() inspect = fn { u32 value = 1; *!u32 root = &!value; *!u32 child = &!(*root); *child; *root; };\n%%end", ScalarTargetLayout::NATIVE64, false);
    let owner = cfg.producer_owner.clone();
    let identity = SourceIdentity::new(
        "source-use-import".into(),
        "app".into(),
        "src/import.w".into(),
        "r1".into(),
    );
    let imported = ProducerCfgOwnerId::Module(identity.clone());
    let remap = ValueSchemaRemap {
        producers: vec![(owner.clone(), imported.clone())],
        points: cfg
            .initial_points
            .iter()
            .map(|point| ValuePointRemap {
                producer: owner.clone(),
                point: point.id,
                imported: CfgPointId(point.id.0 + 17),
            })
            .collect(),
        shapes: cfg
            .evaluated_values
            .shapes
            .iter()
            .map(|shape| {
                (
                    shape.id.clone(),
                    ValueShapeId {
                        producer: imported.clone(),
                        index: shape.id.index + 23,
                    },
                )
            })
            .collect(),
        imports: vec![ValueImportCorrespondence {
            producer: owner.clone(),
            bindings: cfg
                .evaluated_values
                .source_binding_shapes
                .iter()
                .map(|fact| {
                    let mut binding = fact.binding.clone();
                    binding.source = identity.clone();
                    (fact.binding.clone(), binding)
                })
                .collect(),
            places: Vec::new(),
            origins: Vec::new(),
            loans: Vec::new(),
            runtime_values: Vec::new(),
            environments: Vec::new(),
            backings: Vec::new(),
            holders: Vec::new(),
        }],
    };
    let context = SourceCapturePolicyContext { structs: Vec::new(), enums: Vec::new(), generic_scope: owner.clone(), generic_declarations: Vec::new(), generic_parameters: BTreeSet::new() };
    let sources = capture_table_sources(&cfg, &context).expect("original source-use constructor closure");
    let forest = capture_root_construction_forest(&cfg, &context).expect("actual original source-use construction forest");
    let destination = remap.stage_boundary_sources(&sources, &forest).expect("actual selective source-use headers");
    let mapped = remap
        .source_use_graphs_into(&owner, &cfg.evaluated_values, &sources, &forest, &destination)
        .expect("total qualified graph remap");
    let [graph] = mapped.graphs.as_slice() else {
        panic!("one source epoch");
    };
    let original = &cfg.evaluated_values.source_use_graphs[0];
    assert_eq!(graph.id.owner, imported);
    assert_eq!(graph.epoch.owner, imported);
    for (original, mapped) in original.nodes.iter().zip(&graph.nodes) {
        assert_eq!(mapped.id.contributor.owner, imported);
        assert_eq!(mapped.id.monitor.owner, imported);
        assert_eq!(mapped.configuration.owner, imported);
        assert_eq!(mapped.event.point.0, original.event.point.0 + 17);
        assert_eq!(mapped.frame.owner, imported);
        for support in &mapped.supports {
            assert_eq!(support.coordinate.owner, imported);
        }
    }
    for (original, mapped) in original.uses.iter().zip(&graph.uses) {
        assert_eq!(
            mapped.event,
            remap.source_event(&original.event).expect("event")
        );
        assert_eq!(mapped.support.node.contributor.owner, imported);
        assert_eq!(mapped.support.coordinate.owner, imported);
        assert_eq!(mapped.equation.input.owner, imported);
        assert_eq!(mapped.equation.output.owner, imported);
        assert_eq!(mapped.source_span, original.source_span);
    }
    for (original, mapped) in original.reachable.iter().zip(&graph.reachable) {
        assert_eq!(mapped.direct_use, original.direct_use);
        assert_eq!(mapped.predecessors, original.predecessors);
        assert_eq!(mapped.support.node.contributor.owner, imported);
        assert_eq!(mapped.support.node.monitor.owner, imported);
    }
}
