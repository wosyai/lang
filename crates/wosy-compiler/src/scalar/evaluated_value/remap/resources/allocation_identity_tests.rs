use super::*;
use source_boundary::allocation_identity_constructor::tests::{factory, forest, SOURCES};
use source_boundary::null_constructor::SourceConstructorOriginDerivation;

fn mapping(before: &SourceCaptureConstructionForest, after: &SourceCaptureConstructionForest) -> ValueSchemaRemap {
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    assert_eq!(before.sources.len(), after.sources.len());
    for (a, b) in before.sources.iter().zip(&after.sources) {
        assert_eq!(std::mem::discriminant(&a.producer), std::mem::discriminant(&b.producer));
        remap.producers.push((a.producer.clone(), b.producer.clone()));
        assert_eq!(a.initial_points.len(), b.initial_points.len());
        for (x, y) in a.initial_points.iter().zip(&b.initial_points) {
            assert_eq!(x.source_span.range, y.source_span.range);
            assert_eq!(std::mem::discriminant(&x.kind), std::mem::discriminant(&y.kind));
            remap.points.push(ValuePointRemap { producer: a.producer.clone(), point: x.id, imported: y.id });
        }
        assert_eq!(a.tables.shapes.len(), b.tables.shapes.len());
        for (x, y) in a.tables.shapes.iter().zip(&b.tables.shapes) { remap.shapes.push((x.id.clone(), y.id.clone())); }
        remap.imports.push(ValueImportCorrespondence { producer: a.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    let source = original::catalog(&before.sources).expect("genuine source catalog");
    let destination = original::catalog(&after.sources).expect("genuine destination catalog");
    for import in &mut remap.imports {
        let target = &remap.producers.iter().find(|pair| pair.0 == import.producer).expect("genuine source-destination owner").1;
        let a = source.vertices.iter().filter(|row| row.reference.owner == import.producer).collect::<Vec<_>>();
        let b = destination.vertices.iter().filter(|row| row.reference.owner == *target).collect::<Vec<_>>();
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            match (&a.reference.key, &b.reference.key) {
                (BoundaryResourceKey::Origin(x), BoundaryResourceKey::Origin(y)) => { assert_eq!(x.0, import.origins.len()); import.origins.push(*y); },
                (BoundaryResourceKey::Binding(x), BoundaryResourceKey::Binding(y)) => import.bindings.push((x.clone(), y.clone())),
                (BoundaryResourceKey::Place(x), BoundaryResourceKey::Place(y)) => import.places.push((x.clone(), y.clone())),
                (BoundaryResourceKey::Loan(x), BoundaryResourceKey::Loan(y)) => import.loans.push((x.clone(), y.clone())),
                (BoundaryResourceKey::Runtime(x), BoundaryResourceKey::Runtime(y)) => import.runtime_values.push((x.clone(), y.clone())),
                (BoundaryResourceKey::Environment(x), BoundaryResourceKey::Environment(y)) => import.environments.push((x.clone(), y.clone())),
                (BoundaryResourceKey::Backing(x), BoundaryResourceKey::Backing(y)) => import.backings.push((x.clone(), y.clone())),
                (BoundaryResourceKey::Holder(x), BoundaryResourceKey::Holder(y)) => import.holders.push((x.clone(), y.clone())),
                _ => panic!("actual complete typed resource roles"),
            }
        }
    }
    remap
}

fn corrupt(tables: &mut EvaluatedValueTables, defect: usize) {
    match defect {
        0 => tables.source_origin_derivations.clear(),
        1 => { let extra = tables.source_origin_derivations.iter().find(|row| matches!(row, SourceConstructorOriginDerivation::AllocationIdentityAddress(_))).expect("actual fourth incidence").clone(); tables.source_origin_derivations.push(extra); },
        2..=6 => {
            let row = tables.source_origin_derivations.iter_mut().find_map(|row| match row {
                SourceConstructorOriginDerivation::AllocationIdentityAddress(row) => Some(row),
                SourceConstructorOriginDerivation::NullExpression(_) | SourceConstructorOriginDerivation::CheckedParameter(_) | SourceConstructorOriginDerivation::DirectStorageAddress(_) => None,
            }).expect("actual fourth incidence");
            match defect {
                2 => row.origin.origin = ReferenceOriginId(usize::MAX),
                3 => row.event.point = CfgPointId(usize::MAX),
                4 => row.association.association = ReferenceRebindAssociationId(usize::MAX),
                5 => row.association.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                6 => row.origin.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                _ => unreachable!(),
            }
        },
        _ => unreachable!(),
    }
}

#[test]
fn allocation_identity_all_five_full_copied_selective_staged_empty_owned_serde_atomicity() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for text in SOURCES {
                let a = forest(&factory("source", text, layout, project));
                let b = forest(&factory("destination", text, layout, project));
                let remap = mapping(&a, &b);
                let mapped = remap.tables_into(&a.sources, &a, &b.sources, &b).expect("genuine independent complete fourth creator baseline");
                let copied = remap.tables(&a.sources, &a).expect("genuine copied complete fourth baseline");
                let witness = remap.independent_boundary_destination(&b.sources, &b).expect("whole original destination headers");
                let count = a.sources.iter().map(|source| source_boundary::allocation_identity_constructor::expected(source.resource_construction.original(), source.resource_construction.structs()).expect("independent whole ordinary Rebind inventory").len()).sum::<usize>();
                assert_eq!(mapped[0].resources.allocation_identity().len(), count);
                assert_eq!(copied[0].resources.allocation_identity().len(), count);
                let saved = (a.clone(), b.clone(), remap.clone(), witness.clone(), mapped.clone(), copied.clone());
                let checked = witness.checked(&remap, &a.sources, &a, &demand::resource_demand_full(&a.sources)).expect("full original independent selected witness");
                let empty = witness.checked(&remap, &a.sources, &a, &BoundaryResourceDemand { roots: Vec::new() }).expect("zero demand retains full original fourth-family evidence");
                assert!(empty.resources.allocation_identity().is_empty());
                allocation_identity::owned(&remap, &a.sources, &empty.resources).expect("empty selection source evidence mandatory");
                for (ordinal, source) in a.sources.iter().enumerate() {
                    if source_boundary::allocation_identity_constructor::expected(source.resource_construction.original(), source.resource_construction.structs()).expect("genuine original source").is_empty() { continue; }
                    let root = &source.producer;
                    let uses = remap.source_use_graphs_into(root, &source.tables, &a.sources, &a, &witness).expect("actual source-use selective fourth baseline");
                    let stages = remap.staged_loan_queries_into(root, &source.tables, &a.sources, &a, &witness).expect("actual staged selective fourth baseline");
                    for side in 0..2 {
                        for defect in 0..7 {
                            let mut candidate = if side == 0 { a.sources.clone() } else { b.sources.clone() };
                            corrupt(&mut candidate[ordinal].tables, defect);
                            let before = candidate.clone();
                            if side == 0 {
                                assert!(remap.tables_into(&candidate, &a, &b.sources, &b).is_err());
                                assert!(remap.tables(&candidate, &a).is_err());
                                assert!(remap.source_use_graphs_into(root, &candidate[ordinal].tables, &candidate, &a, &witness).is_err());
                                assert!(remap.staged_loan_queries_into(root, &candidate[ordinal].tables, &candidate, &a, &witness).is_err());
                            } else { assert!(remap.tables_into(&a.sources, &a, &candidate, &b).is_err()); }
                            assert_eq!(candidate, before);
                        }
                    }
                    for cut in [BoundaryResourceDemandCut::SourceUses { root: root.clone() }, BoundaryResourceDemandCut::StagedLoans { root: root.clone() }] {
                        postvalidation::selective(&remap, root, &a.sources, &mapped[ordinal].tables, &checked.headers, &checked.correspondence, &cut, &checked.resources).expect("actual BOTH selective full metadata baseline");
                        for defect in 0..7 {
                            let mut candidate = mapped[ordinal].tables.clone();
                            corrupt(&mut candidate, defect);
                            let before = candidate.clone();
                            assert!(postvalidation::selective(&remap, root, &a.sources, &candidate, &checked.headers, &checked.correspondence, &cut, &checked.resources).is_err());
                            assert_eq!(candidate, before);
                        }
                    }
                    let target = empty.headers.iter().find(|header| header.producer == remap.boundary_producer(root).expect("genuine mapped owner")).expect("actual empty selected header");
                    let table = remap.map_table(root, &source.tables, &empty.correspondence, &target.tables).expect("whole fourth metadata with zero selected origins");
                    checked_parameter::metadata(&remap, source, &table).expect("empty full4family metadata baseline");
                    for defect in 0..7 {
                        let mut candidate = table.clone();
                        corrupt(&mut candidate, defect);
                        let before = candidate.clone();
                        assert!(checked_parameter::metadata(&remap, source, &candidate).is_err());
                        assert_eq!(candidate, before);
                    }
                    let decoded = serde_json::from_str::<EvaluatedValueTables>(&serde_json::to_string(&source.tables).expect("candidate metadata Serde")).expect("candidate metadata decode");
                    let mut candidate = a.sources.clone();
                    candidate[ordinal].tables = decoded;
                    assert_eq!(remap.tables_into(&candidate, &a, &b.sources, &b).expect("Serde requires separate true originals"), mapped);
                    assert_eq!(remap.source_use_graphs_into(root, &source.tables, &a.sources, &a, &witness).expect("repeated selective"), uses);
                    assert_eq!(remap.staged_loan_queries_into(root, &source.tables, &a.sources, &a, &witness).expect("repeated staged"), stages);
                }
                for defect in 0..5 {
                    let mut candidate = mapped[0].resources.clone();
                    match defect {
                        0 => candidate.allocation_identity.clear(),
                        1 => { let extra = candidate.allocation_identity[0].clone(); candidate.allocation_identity.push(extra); },
                        2 => candidate.allocation_identity[0].destination.association.association = ReferenceRebindAssociationId(usize::MAX),
                        3 => candidate.allocation_identity[0].source.event.point = CfgPointId(usize::MAX),
                        4 => candidate.allocation_identity[0].destination.association.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                        _ => unreachable!(),
                    }
                    let before = candidate.clone();
                    assert!(allocation_identity::owned(&remap, &a.sources, &candidate).is_err());
                    assert_eq!(candidate, before);
                }
                assert_eq!(remap.tables_into(&a.sources, &a, &b.sources, &b).expect("repeated full fourth baseline"), mapped);
                assert_eq!((a, b, remap, witness, mapped, copied), saved);
            }
        }
    }
}

#[test]
fn allocation_identity_standalone_dependency_reservation_and_publication_fail_atomically() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for text in SOURCES {
                let a = forest(&factory("source", text, layout, project));
                let b = forest(&factory("destination", text, layout, project));
                let remap = mapping(&a, &b);
                let witness = remap.independent_boundary_destination(&b.sources, &b).expect("genuine independent destination witness");
                let demand = demand::resource_demand_full(&a.sources);
                let checked = witness.checked(&remap, &a.sources, &a, &demand).expect("actual full dependency reservation baseline");
                let closure = original::closure(&demand, &a).expect("genuine dependency closure");
                let reservations = reservation::ResourceReservations::original(&closure);
                let values = closure.vertices.iter().map(|row| row.value.clone()).collect::<Vec<_>>();
                reservations.fill(&closure, &values).expect("actual original reservation fill baseline");
                let saved = (a.clone(), b.clone(), remap.clone(), witness.clone(), checked.clone());
                let mut incomplete = values.clone();
                incomplete.pop().expect("genuine nonempty allocation resource closure");
                let before = incomplete.clone();
                assert!(reservations.fill(&closure, &incomplete).is_err());
                assert_eq!(incomplete, before);
                let mut missing = checked.resources.clone();
                missing.associations.clear();
                let before = missing.clone();
                assert!(allocation::validate_edges(&closure, &missing.arenas, &missing.associations).is_err());
                assert!(null_constructor::owned(&remap, &a.sources, &missing).is_err());
                assert_eq!(missing, before);
                let mapped = remap.tables_into(&a.sources, &a, &b.sources, &b).expect("actual published tables baseline");
                let mut candidate = mapped.clone();
                candidate[0].resources.allocation_identity.clear();
                let before = candidate.clone();
                assert!(postvalidation::full(&remap, &a.sources, &candidate, &checked.correspondence, &checked.resources).is_err());
                assert_eq!(candidate, before);
                assert_eq!((a, b, remap, witness, checked), saved);
            }
        }
    }
}

#[test]
fn allocation_identity_real_shared_original_checked_resource_witness_retains_whole_creator() {
    let text = "%%start\nu64 count = 2; *?u8 raw = null; u8[count] bytes; unsafe { core.rebind(&bytes, raw, count); }; bytes[0];\nunit() run = fn { u8 value = 1; value; };\nrun();\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let module = factory("shared", text, layout, project);
            let original = forest(&module);
            let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual selected named root");
            let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &original, BTreeMap::new()).expect("actual independent source preflight");
            let source = capture_table_sources(&function.reference_cfg, &input.policy_context).expect("actual selected named subtree");
            let mut remap = mapping(&original, &original);
            remap.producers.retain(|pair| source.iter().any(|row| row.producer == pair.0));
            remap.points.retain(|point| source.iter().any(|row| row.producer == point.producer));
            remap.shapes.retain(|pair| source.iter().any(|row| row.producer == pair.0.producer));
            remap.imports.retain(|import| source.iter().any(|row| row.producer == import.producer));
            let module_source = original.sources.iter().find(|source| source.producer == module.top_level_reference_cfg.producer_owner).expect("genuine external module snapshot");
            let creator = source_boundary::allocation_identity_constructor::expected(module_source.resource_construction.original(), module_source.resource_construction.structs()).expect("independent shared module original")[0].clone();
            assert!(!remap.producers.iter().any(|pair| pair.0 == creator.origin.owner));
            let witness = remap.independent_boundary_destination(&source, &original).expect("actual Independent SharedOriginal resource route");
            let demand = BoundaryResourceDemand { roots: vec![BoundaryResourceRef { owner: creator.origin.owner.clone(), key: BoundaryResourceKey::Origin(creator.origin.origin) }] };
            let checked = witness.checked(&remap, &source, &original, &demand).expect("real SharedOriginal fourth-family demand");
            assert_eq!(checked.resources.allocation_identity().len(), 1);
            assert_eq!(checked.resources.allocation_identity()[0].source, creator);
            assert_eq!(checked.resources.allocation_identity()[0].destination, creator);
            assert!(checked.resources.source_originals().contains(&module_source.resource_construction));
            allocation_identity::owned(&remap, &source, &checked.resources).expect("whole real SharedOriginal source authenticated through actual owned route");
            let saved = (source.clone(), original.clone(), remap.clone(), witness.clone(), checked.clone());
            let mut missing = checked.resources.clone();
            missing.allocation_identity.clear();
            let before = missing.clone();
            assert!(allocation_identity::owned(&remap, &source, &missing).is_err());
            assert_eq!(missing, before);
            let empty = witness.checked(&remap, &source, &original, &BoundaryResourceDemand { roots: Vec::new() }).expect("real checked empty source demand");
            assert!(empty.resources.allocation_identity().is_empty());
            allocation_identity::owned(&remap, &source, &empty.resources).expect("empty owned real originals");
            assert_eq!((source, original, remap, witness, checked), saved);
        }
    }
}

#[test]
fn allocation_identity_both_original_operand_types_and_tuple_remap_redirects_fail_atomically() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let a = forest(&factory("source", SOURCES[1], layout, project));
            let b = forest(&factory("destination", SOURCES[1], layout, project));
            let remap = mapping(&a, &b);
            let mapped = remap.tables_into(&a.sources, &a, &b.sources, &b).expect("actual independently typed operand baseline");
            let saved = (a.clone(), b.clone(), remap.clone(), mapped.clone());
            for (ordinal, source) in a.sources.iter().enumerate() {
                let cfg = source.resource_construction.original();
                if cfg.rebind_associations.is_empty() { continue; }
                let association = &cfg.rebind_associations[0];
                for point in [association.raw_point, association.length_point, association.address_point, association.effect_point, association.call_point] {
                    let mut candidate = remap.clone();
                    candidate.points.iter_mut().find(|mapping| mapping.producer == source.producer && mapping.point == point).expect("genuine original tuplepoint map").imported = association.address_point;
                    if point == association.address_point { candidate.points.iter_mut().find(|mapping| mapping.producer == source.producer && mapping.point == point).expect("genuine address map").imported = association.call_point; }
                    let before = candidate.clone();
                    assert!(candidate.tables_into(&a.sources, &a, &b.sources, &b).is_err());
                    assert_eq!(candidate, before);
                }
                for side in 0..2 {
                    for operand in [association.raw_point, association.length_point] {
                        let mut candidate = if side == 0 { a.sources.clone() } else { b.sources.clone() };
                        let fact = candidate[ordinal].tables.values.iter().find(|value| value.id.point == operand).expect("actual original typed raw/length value");
                        let shape = fact.shape.clone();
                        candidate[ordinal].tables.shapes.iter_mut().find(|value| value.id == shape).expect("actual original operand type shape").ty = ScalarType::Bool;
                        let before = candidate.clone();
                        if side == 0 { assert!(remap.tables_into(&candidate, &a, &b.sources, &b).is_err()); }
                        else { assert!(remap.tables_into(&a.sources, &a, &candidate, &b).is_err()); }
                        assert_eq!(candidate, before);
                    }
                }
            }
            assert_eq!((a, b, remap, mapped), saved);
        }
    }
}
