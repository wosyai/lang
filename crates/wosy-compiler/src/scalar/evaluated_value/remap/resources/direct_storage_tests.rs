use super::*;
use source_boundary::direct_storage_constructor::tests::{factory, forest, SOURCES};
use source_boundary::null_constructor::SourceConstructorOriginDerivation;

fn mapping(before: &SourceCaptureConstructionForest, after: &SourceCaptureConstructionForest) -> ValueSchemaRemap {
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    assert_eq!(before.sources.len(), after.sources.len());
    for (a, b) in before.sources.iter().zip(&after.sources) {
        assert_eq!(std::mem::discriminant(&a.producer), std::mem::discriminant(&b.producer));
        remap.producers.push((a.producer.clone(), b.producer.clone()));
        assert_eq!(a.initial_points.len(), b.initial_points.len());
        for (a_point, b_point) in a.initial_points.iter().zip(&b.initial_points) {
            assert_eq!(a_point.source_span.range, b_point.source_span.range);
            assert_eq!(std::mem::discriminant(&a_point.kind), std::mem::discriminant(&b_point.kind));
            remap.points.push(ValuePointRemap { producer: a.producer.clone(), point: a_point.id, imported: b_point.id });
        }
        assert_eq!(a.tables.shapes.len(), b.tables.shapes.len());
        for (a, b) in a.tables.shapes.iter().zip(&b.tables.shapes) { remap.shapes.push((a.id.clone(), b.id.clone())); }
        remap.imports.push(ValueImportCorrespondence { producer: a.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    let a = original::catalog(&before.sources).expect("actual independently admitted source resources");
    let b = original::catalog(&after.sources).expect("actual independently admitted destination resources");
    for import in &mut remap.imports {
        let owner = &remap.producers.iter().find(|pair| pair.0 == import.producer).expect("actual paired original owner").1;
        let sources = a.vertices.iter().filter(|row| row.reference.owner == import.producer).collect::<Vec<_>>();
        let targets = b.vertices.iter().filter(|row| row.reference.owner == *owner).collect::<Vec<_>>();
        assert_eq!(sources.len(), targets.len());
        for (a, b) in sources.iter().zip(targets) {
            match (&a.reference.key, &b.reference.key) {
                (BoundaryResourceKey::Origin(a), BoundaryResourceKey::Origin(b)) => { assert_eq!(a.0, import.origins.len()); import.origins.push(*b); },
                (BoundaryResourceKey::Binding(a), BoundaryResourceKey::Binding(b)) => import.bindings.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Place(a), BoundaryResourceKey::Place(b)) => import.places.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Loan(a), BoundaryResourceKey::Loan(b)) => import.loans.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Runtime(a), BoundaryResourceKey::Runtime(b)) => import.runtime_values.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Environment(a), BoundaryResourceKey::Environment(b)) => import.environments.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Backing(a), BoundaryResourceKey::Backing(b)) => import.backings.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Holder(a), BoundaryResourceKey::Holder(b)) => import.holders.push((a.clone(), b.clone())),
                _ => panic!("genuine exact typed original catalog roles"),
            }
        }
    }
    remap
}

fn corrupt(tables: &mut EvaluatedValueTables, defect: usize) {
    match defect {
        0 => tables.source_origin_derivations.clear(),
        1 => { let extra = tables.source_origin_derivations[0].clone(); tables.source_origin_derivations.push(extra); },
        2 => tables.source_origin_derivations.reverse(),
        3..=5 => {
            let row = tables.source_origin_derivations.iter_mut().find_map(|row| match row { SourceConstructorOriginDerivation::DirectStorageAddress(row) => Some(row), SourceConstructorOriginDerivation::AllocationIdentityAddress(_) | SourceConstructorOriginDerivation::CheckedParameter(_) | SourceConstructorOriginDerivation::NullExpression(_) => None }).expect("actual direct incidence");
            match defect {
                3 => row.origin.origin = ReferenceOriginId(usize::MAX),
                4 => row.event.point = CfgPointId(usize::MAX),
                5 => row.event.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                _ => unreachable!(),
            }
        },
        _ => unreachable!(),
    }
}

#[test]
fn direct_storage_full_all_thirteen_group_original_transport_owned_and_serde() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for text in SOURCES {
                let a = forest(&factory("source", text, layout, project));
                let b = forest(&factory("destination", text, layout, project));
                let source = &a.sources;
                let destination = &b.sources;
                let remap = mapping(&a, &b);
                let mapped = remap.tables_into(source, &a, destination, &b).expect("genuine independent full direct baseline including nominal and index originals");
                let copied = remap.tables(source, &a).expect("genuine copied direct baseline");
                let expected = source.iter().map(|source| source_boundary::direct_storage_constructor::expected(source.resource_construction.original(), source.resource_construction.structs()).expect("independent original required incidence vector").len()).sum::<usize>();
                assert_eq!(mapped[0].resources.direct_storage().len(), expected);
                assert_eq!(copied[0].resources.direct_storage().len(), expected);
                let witness = remap.independent_boundary_destination(destination, &b).expect("genuine independent original headers");
                let saved = (a.clone(), b.clone(), remap.clone(), witness.clone(), mapped.clone(), copied.clone());
                for (ordinal, carrier) in source.iter().enumerate() {
                    if source_boundary::direct_storage_constructor::expected(carrier.resource_construction.original(), carrier.resource_construction.structs()).expect("source baseline").is_empty() { continue; }
                    let root = &carrier.producer;
                    let uses = remap.source_use_graphs_into(root, &carrier.tables, source, &a, &witness).expect("actual direct source-use baseline");
                    let stages = remap.staged_loan_queries_into(root, &carrier.tables, source, &a, &witness).expect("actual direct staged baseline");
                    for side in 0..2 {
                        for defect in [0, 1, 3, 4, 5] {
                            let mut candidate = if side == 0 { source.clone() } else { destination.clone() };
                            corrupt(&mut candidate[ordinal].tables, defect);
                            let before = candidate.clone();
                            if side == 0 {
                                assert!(remap.tables_into(&candidate, &a, destination, &b).is_err());
                                assert!(remap.source_use_graphs_into(root, &candidate[ordinal].tables, &candidate, &a, &witness).is_err());
                                assert!(remap.staged_loan_queries_into(root, &candidate[ordinal].tables, &candidate, &a, &witness).is_err());
                            } else { assert!(remap.tables_into(source, &a, &candidate, &b).is_err()); }
                            assert_eq!(candidate, before);
                        }
                    }
                    let checked = witness.checked(&remap, source, &a, &BoundaryResourceDemand { roots: Vec::new() }).expect("empty selected original demand retains whole source direct evidence");
                    assert!(checked.resources.direct_storage().is_empty());
                    direct_storage::owned(&remap, source, &checked.resources).expect("whole direct originals even with zero selected associations");
                    let tables = &mapped[ordinal].tables;
                    for cut in [BoundaryResourceDemandCut::SourceUses { root: root.clone() }, BoundaryResourceDemandCut::StagedLoans { root: root.clone() }] {
                        let selected = witness.checked(&remap, source, &a, &demand::resource_demand_full(source)).expect("actual independent selected witness");
                        postvalidation::selective(&remap, root, source, tables, &selected.headers, &selected.correspondence, &cut, &selected.resources).expect("actual direct BOTHselective baseline");
                        for defect in [0, 1, 3, 4, 5] {
                            let mut candidate = tables.clone();
                            corrupt(&mut candidate, defect);
                            let before = candidate.clone();
                            assert!(postvalidation::selective(&remap, root, source, &candidate, &selected.headers, &selected.correspondence, &cut, &selected.resources).is_err());
                            assert_eq!(candidate, before);
                        }
                    }
                    let decoded = serde_json::from_str::<EvaluatedValueTables>(&serde_json::to_string(&carrier.tables).expect("direct candidate Serde encode")).expect("direct candidate Serde decode");
                    let mut restored = source.clone();
                    restored[ordinal].tables = decoded;
                    assert_eq!(remap.tables_into(&restored, &a, destination, &b).expect("Serde candidate authenticated against separate whole source originals"), mapped);
                    assert_eq!(remap.source_use_graphs_into(root, &carrier.tables, source, &a, &witness).expect("repeat source-use baseline"), uses);
                    assert_eq!(remap.staged_loan_queries_into(root, &carrier.tables, source, &a, &witness).expect("repeat staged baseline"), stages);
                }
                let mut missing = mapped[0].resources.clone();
                missing.direct_storage.clear();
                let before = missing.clone();
                assert!(direct_storage::owned(&remap, source, &missing).is_err());
                assert_eq!(missing, before);
                let identity = mapping(&a, &a);
                let same = identity.tables_into(source, &a, source, &a).expect("actual independent identity baseline");
                let shared = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
                let selected = direct_storage::selected(&shared, same[0].resources.source_originals(), same[0].resources.associations()).expect("whole SharedOriginal direct incidence baseline");
                assert!(selected.iter().all(|pair| pair.source == pair.destination));
                direct_storage::original_pairs(&shared, same[0].resources.source_originals(), same[0].resources.source_originals(), same[0].resources.input_types()).expect("WHOLE original snapshot equality includes true structs");
                assert_eq!(remap.tables_into(source, &a, destination, &b).expect("repeated full original direct baseline"), mapped);
                assert_eq!((a, b, remap, witness, mapped, copied), saved);
            }
        }
    }
}

#[test]
fn direct_storage_nominal_control_owned_and_empty_original_atomicity() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for group in [2, 5, 11, 12] {
                let a = forest(&factory("source", SOURCES[group], layout, project));
                let b = forest(&factory("destination", SOURCES[group], layout, project));
                let remap = mapping(&a, &b);
                let mapped = remap.tables_into(&a.sources, &a, &b.sources, &b).expect("genuine nominal/control/mixed full baseline");
                let witness = remap.independent_boundary_destination(&b.sources, &b).expect("genuine independent policy and control headers");
                let saved = (a.clone(), b.clone(), remap.clone(), mapped.clone(), witness.clone());
                for side in 0..2 {
                    let source = if side == 0 { &a.sources } else { &b.sources };
                    for (ordinal, carrier) in source.iter().enumerate() {
                        if group == 2 {
                            let mut candidate = source.clone();
                            candidate[ordinal].policy_context.structs.clear();
                            let before = candidate.clone();
                            if side == 0 { assert!(remap.tables_into(&candidate, &a, &b.sources, &b).is_err()); }
                            else { assert!(remap.tables_into(&a.sources, &a, &candidate, &b).is_err()); }
                            assert_eq!(candidate, before);
                        }
                        for (production, row) in carrier.tables.productions.iter().enumerate() {
                            if !matches!(row.source, OutputProductionSource::Block { .. } | OutputProductionSource::Conditional { .. }) { continue; }
                            for defect in 0..5 {
                                let mut candidate = source.clone();
                                let row = &mut candidate[ordinal].tables.productions[production];
                                match defect {
                                    0 => row.evaluation_point = CfgPointId(usize::MAX),
                                    1 => row.occurrence.range = ByteSpan::new(0, 0),
                                    2 => row.output_projection.push(usize::MAX),
                                    3 => row.outputs[0].point = CfgPointId(usize::MAX),
                                    4 => match &mut row.source {
                                        OutputProductionSource::Block { scope_exit, .. } => *scope_exit = row.evaluation_point,
                                        OutputProductionSource::Conditional { inputs, .. } => inputs[0].predecessor = inputs[1].predecessor,
                                        _ => unreachable!(),
                                    },
                                    _ => unreachable!(),
                                }
                                let before = candidate.clone();
                                if side == 0 { assert!(remap.tables_into(&candidate, &a, &b.sources, &b).is_err()); }
                                else { assert!(remap.tables_into(&a.sources, &a, &candidate, &b).is_err()); }
                                assert_eq!(candidate, before);
                            }
                        }
                    }
                }
                for defect in 0..5 {
                    let mut candidate = mapped[0].resources.clone();
                    match defect {
                        0 => candidate.direct_storage.clear(),
                        1 => { let extra = candidate.direct_storage[0].clone(); candidate.direct_storage.push(extra); },
                        2 => candidate.direct_storage[0].destination.origin.origin = ReferenceOriginId(usize::MAX),
                        3 => candidate.direct_storage[0].destination.event.point = CfgPointId(usize::MAX),
                        4 => candidate.direct_storage[0].source.origin.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                        _ => unreachable!(),
                    }
                    let before = candidate.clone();
                    assert!(direct_storage::owned(&remap, &a.sources, &candidate).is_err());
                    assert_eq!(candidate, before);
                }
                let empty = witness.checked(&remap, &a.sources, &a, &BoundaryResourceDemand { roots: Vec::new() }).expect("independent originals with zero selected direct associations");
                assert!(empty.resources.direct_storage().is_empty());
                direct_storage::owned(&remap, &a.sources, &empty.resources).expect("empty selection retains mandatory whole independent originals");
                for carrier in &a.sources {
                    if source_boundary::direct_storage_constructor::expected(carrier.resource_construction.original(), carrier.resource_construction.structs()).expect("genuine direct original").is_empty() { continue; }
                    let root = &carrier.producer;
                    let target = empty.headers.iter().find(|header| header.producer == remap.boundary_producer(root).expect("genuine mapped root")).expect("actual destination header");
                    let tables = remap.map_table(root, &carrier.tables, &empty.correspondence, &target.tables).expect("whole creator metadata carries separately from selected resource associations");
                    checked_parameter::metadata(&remap, carrier, &tables).expect("empty owned selection whole3family metadata baseline");
                    for defect in [0, 1, 3, 4, 5] {
                        let mut candidate = tables.clone();
                        corrupt(&mut candidate, defect);
                        let before = candidate.clone();
                        assert!(checked_parameter::metadata(&remap, carrier, &candidate).is_err());
                        assert_eq!(candidate, before);
                    }
                }
                assert_eq!((a, b, remap, mapped, witness), saved);
            }
        }
    }
}
