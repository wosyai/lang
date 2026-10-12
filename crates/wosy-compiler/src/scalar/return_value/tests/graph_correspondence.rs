use super::*;
mod resources;

pub(super) fn complete(
    original: &ScalarModule,
    target: &ScalarModule,
    old: &ScalarReferenceCfg,
    new: &ScalarReferenceCfg,
    points: Vec<ValuePointRemap>,
    shapes: Vec<(ValueShapeId, ValueShapeId)>,
    bindings: Vec<(ReferenceBindingId, ReferenceBindingId)>,
    types: &dyn Fn(&ScalarType, &ScalarType) -> bool,
) -> ValueSchemaRemap {
    fn collect<'a>(old: &'a ScalarReferenceCfg, new: &'a ScalarReferenceCfg, types: &dyn Fn(&ScalarType, &ScalarType) -> bool, pairs: &mut Vec<(&'a ScalarReferenceCfg, &'a ScalarReferenceCfg)>) {
        assert!(!pairs.iter().any(|(a, b)| a.producer_owner == old.producer_owner || b.producer_owner == new.producer_owner));
        pairs.push((old, new));
        assert_eq!(old.literal_cfgs.len(), new.literal_cfgs.len());
        for child in &old.literal_cfgs {
            let events = old.initial_points.iter().filter(|point| point.id == child.capture_point).collect::<Vec<_>>();
            let [event] = events.as_slice() else { panic!("actual original child Capture event"); };
            let CfgPointKind::Capture { signature, captures, .. } = &event.kind else { panic!("actual original Capture role"); };
            let candidates = new.literal_cfgs.iter().filter(|candidate| {
                let events = new.initial_points.iter().filter(|point| point.id == candidate.capture_point).collect::<Vec<_>>();
                let [receiving] = events.as_slice() else { return false; };
                let CfgPointKind::Capture { signature: receiving_signature, captures: receiving_captures, .. } = &receiving.kind else { return false; };
                event.source_span.range == receiving.source_span.range && child.literal.range == candidate.literal.range
                    && types(signature, receiving_signature) && captures.len() == receiving_captures.len()
                    && captures.iter().zip(receiving_captures).all(|(before, after)| before.name == after.name && before.use_span.range == after.use_span.range && types(&before.ty, &after.ty) && types(&before.place_type, &after.place_type))
                    && matches!(&candidate.reference_cfg.producer_owner, ProducerCfgOwnerId::Literal { enclosing, capture_point, literal }
                        if enclosing.as_ref() == &new.producer_owner && *capture_point == receiving.id && literal == &candidate.literal)
            }).collect::<Vec<_>>();
            let [receiving] = candidates.as_slice() else { panic!("unique qualified actual child constructor correspondence"); };
            collect(&child.reference_cfg, &receiving.reference_cfg, types, pairs);
        }
    }
    assert_eq!(original.source.project, target.source.project);
    assert_eq!(original.source.package, target.source.package);
    assert_eq!(original.source.path, target.source.path);
    assert_eq!(original.source.revision, "r1");
    assert_eq!(target.source.revision, "r2");
    let mut pairs = Vec::new();
    collect(old, new, types, &mut pairs);
    let mut remap = ValueSchemaRemap { producers: pairs.iter().map(|(a, b)| (a.producer_owner.clone(), b.producer_owner.clone())).collect(), points, shapes, imports: Vec::new() };
    for &(before, after) in &pairs {
        if before.producer_owner != old.producer_owner {
            assert_eq!(before.initial_points.len(), after.initial_points.len());
            for point in &before.initial_points {
                let candidates = after.initial_points.iter().filter(|candidate| {
                    point.source_span.range == candidate.source_span.range && std::mem::discriminant(&point.kind) == std::mem::discriminant(&candidate.kind)
                        && match (&point.kind, &candidate.kind) {
                            (CfgPointKind::Call { output: a, .. }, CfgPointKind::Call { output: b, .. }) | (CfgPointKind::ReturnOutput { output: a, .. }, CfgPointKind::ReturnOutput { output: b, .. }) => a == b,
                            (CfgPointKind::Capture { signature: a, captures: x, .. }, CfgPointKind::Capture { signature: b, captures: y, .. }) => types(a, b) && x.len() == y.len(),
                            _ => true,
                        }
                }).collect::<Vec<_>>();
                let [receiving] = candidates.as_slice() else { panic!("unique actual child event role and occurrence"); };
                remap.points.push(ValuePointRemap { producer: before.producer_owner.clone(), point: point.id, imported: receiving.id });
            }
            assert_eq!(before.evaluated_values.shapes.len(), after.evaluated_values.shapes.len());
            for shape in &before.evaluated_values.shapes {
                let candidates = after.evaluated_values.shapes.iter().filter(|candidate| types(&shape.ty, &candidate.ty)).collect::<Vec<_>>();
                let [receiving] = candidates.as_slice() else { panic!("unique independently typed child shape"); };
                remap.shapes.push((shape.id.clone(), receiving.id.clone()));
            }
        }
    }
    let source = capture_table_sources(old, &context(original, old)).expect("actual original shape closure");
    let destination = capture_table_sources(new, &context(target, new)).expect("actual receiving shape closure");
    for &(before, after) in &pairs {
        for point in &after.initial_points {
            assert_eq!(remap.points.iter().filter(|row| row.producer == before.producer_owner && row.imported == point.id).count(), 1);
        }
        for shape in &before.evaluated_values.shapes {
            let targets = after.evaluated_values.shapes.iter().filter(|row| row.id == remap.shape(&shape.id).unwrap()).collect::<Vec<_>>();
            let [receiving] = targets.as_slice() else { panic!("unique actual shape receiving row"); };
            resources::assert_shape(shape, receiving, &remap, types, &source, &destination);
        }
        let imported = resources::imports(original, target, before, after, &remap);
        if before.producer_owner == old.producer_owner {
            for pair in &bindings { assert!(imported.bindings.contains(pair)); }
        }
        remap.imports.push(imported);
    }
    remap
}

pub(super) fn context(module: &ScalarModule, cfg: &ScalarReferenceCfg) -> SourceCapturePolicyContext {
    let forest = forest(module);
    let sources = forest.sources.iter().filter(|row| row.producer == cfg.producer_owner).collect::<Vec<_>>();
    let [source] = sources.as_slice() else { panic!("unique actual canonical constructor scope"); };
    source.policy_context.clone()
}

pub(super) fn forest(module: &ScalarModule) -> SourceCaptureConstructionForest {
    capture_construction_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums,
        module.top_level_reference_cfg.source_capture_inventory.phase.clone(), &[]).expect("actual canonical post-worker module construction forest")
}

pub(super) fn mapped_root<'a>(source: &ProducerCfgOwnerId, original: &[ValueTableSource], mapped: &'a [RemappedValueTable]) -> &'a EvaluatedValueTables {
    assert_eq!(original.len(), mapped.len());
    for row in original {
        let matches = mapped.iter().filter(|candidate| candidate.source == row.producer).collect::<Vec<_>>();
        let [candidate] = matches.as_slice() else { panic!("unique retained mapped source closure"); };
        assert_eq!(candidate.policy_context, row.policy_context);
        assert_eq!(candidate.capture_inventory.phase, row.capture_inventory.phase);
        assert_eq!(candidate.capture_inventory.required.len(), row.capture_inventory.required.len());
        assert_eq!(candidate.children.len(), row.children.len());
        for link in &candidate.children {
            assert_eq!(mapped.iter().filter(|child| child.destination == link.child).count(), 1);
            assert_eq!(candidate.capture_inventory.required.iter().filter(|requirement| requirement.event == link.event && requirement.child == link.child).count(), 1);
        }
    }
    let matches = mapped.iter().filter(|candidate| &candidate.source == source).collect::<Vec<_>>();
    let [root] = matches.as_slice() else { panic!("unique retained original root"); };
    &root.tables
}
