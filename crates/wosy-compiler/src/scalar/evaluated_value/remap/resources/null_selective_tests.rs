use super::*;

#[test]
fn qualified_null_selective_mapped_metadata_requires_whole_original_vector() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let (source, forest) = original("source", layout, project);
            let (destination, target_forest) = original("destination", layout, project);
            let remap = mapping(&forest, &target_forest);
            let root = &source[0].producer;
            let witness = remap.independent_boundary_destination(&destination, &target_forest).expect("independent original destination headers");
            let uses = remap.source_use_graphs_into(root, &source[0].tables, &source, &forest, &witness).expect("actual source-use API baseline with whole Null metadata");
            let stages = remap.staged_loan_queries_into(root, &source[0].tables, &source, &forest, &witness).expect("actual staged API baseline with whole Null metadata");
            let saved = (source.clone(), forest.clone(), destination.clone(), target_forest.clone(), remap.clone(), witness.clone(), uses.clone(), stages.clone());
            for cut in [BoundaryResourceDemandCut::SourceUses { root: root.clone() }, BoundaryResourceDemandCut::StagedLoans { root: root.clone() }] {
                for empty in [false, true] {
                    let demand = if empty {
                        assert!(demand::table_roots(root, &source[0].tables, &cut).is_empty());
                        BoundaryResourceDemand { roots: Vec::new() }
                    } else {
                        match &cut {
                            BoundaryResourceDemandCut::SourceUses { .. } => demand::resource_demand_uses(root, &source[0].tables, &source),
                            BoundaryResourceDemandCut::StagedLoans { .. } => demand::resource_demand_stages(root, &source[0].tables, &source),
                            BoundaryResourceDemandCut::Full => unreachable!(),
                        }
                    };
                    let checked = witness.checked(&remap, &source, &forest, &demand).expect("genuine selected or empty original resources");
                    let owner = remap.boundary_producer(root).expect("actual mapped carrier owner");
                    let targets = checked.headers.iter().filter(|row| row.producer == owner).collect::<Vec<_>>();
                    let [target] = targets.as_slice() else { panic!("one independently supplied original carrier header"); };
                    let mapped = remap.map_table(root, &source[0].tables, &checked.correspondence, &target.tables).expect("actual mapper carries whole Null metadata");
                    assert_eq!(mapped.source_origin_derivations, destination[0].tables.source_origin_derivations);
                    assert_eq!(mapped.source_origin_derivations.len(), 1);
                    if empty { assert!(checked.resources.null_constructors().is_empty()); }
                    postvalidation::selective(&remap, root, &source, &mapped, &checked.headers, &checked.correspondence, &cut, &checked.resources).expect("actual selective postvalidator accepted baseline before defects");
                    let prior = (checked.headers.clone(), checked.correspondence.clone(), checked.resources.clone(), mapped.clone());
                    for defect in 0..5 {
                        let mut candidate = mapped.clone();
                        match defect {
                            0 => candidate.source_origin_derivations.clear(),
                            1 => { let extra = candidate.source_origin_derivations[0].clone(); candidate.source_origin_derivations.push(extra); },
                            2 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut candidate.source_origin_derivations[0] else { panic!("genuine Null fixture") }; row.origin.origin = ReferenceOriginId(usize::MAX); },
                            3 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut candidate.source_origin_derivations[0] else { panic!("genuine Null fixture") }; row.event.point = CfgPointId(usize::MAX); },
                            4 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut candidate.source_origin_derivations[0] else { panic!("genuine Null fixture") }; row.origin.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())); },
                            _ => unreachable!(),
                        }
                        let before = candidate.clone();
                        assert_eq!(postvalidation::selective(&remap, root, &source, &candidate, &checked.headers, &checked.correspondence, &cut, &checked.resources), Err(ValueSchemaRemapError::MissingProducer(root.clone())));
                        assert_eq!(candidate, before);
                        assert_eq!((&checked.headers, &checked.correspondence, &checked.resources, &mapped), (&prior.0, &prior.1, &prior.2, &prior.3));
                        assert_eq!((&source, &forest, &destination, &target_forest, &remap, &witness, &uses, &stages), (&saved.0, &saved.1, &saved.2, &saved.3, &saved.4, &saved.5, &saved.6, &saved.7));
                    }
                    postvalidation::selective(&remap, root, &source, &mapped, &checked.headers, &checked.correspondence, &cut, &checked.resources).expect("accepted original baseline after every failed mutation");
                }
            }
            assert_eq!(remap.source_use_graphs_into(root, &source[0].tables, &source, &forest, &witness).expect("repeated source-use API baseline"), uses);
            assert_eq!(remap.staged_loan_queries_into(root, &source[0].tables, &source, &forest, &witness).expect("repeated staged API baseline"), stages);
        }
    }
}
