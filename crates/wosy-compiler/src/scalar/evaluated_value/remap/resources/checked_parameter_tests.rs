use super::*;
use source_boundary::null_constructor::SourceConstructorOriginDerivation;

fn original(unit: &str, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let module = source_boundary::checked_parameter_constructor::tests::factory(unit, layout, project);
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("independent genuine original forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual named parameter body");
    let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("independent original source context");
    (capture_table_sources(&function.reference_cfg, &input.policy_context).expect("genuine complete original table sources"), forest)
}

fn mapping(source: &SourceCaptureConstructionForest, destination: &SourceCaptureConstructionForest) -> ValueSchemaRemap {
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    assert_eq!(source.sources.len(), destination.sources.len());
    for (before, after) in source.sources.iter().zip(&destination.sources) {
        assert_eq!(std::mem::discriminant(&before.producer), std::mem::discriminant(&after.producer));
        remap.producers.push((before.producer.clone(), after.producer.clone()));
        assert_eq!(before.initial_points.len(), after.initial_points.len());
        for (a, b) in before.initial_points.iter().zip(&after.initial_points) {
            assert_eq!(a.source_span.range, b.source_span.range);
            assert_eq!(std::mem::discriminant(&a.kind), std::mem::discriminant(&b.kind));
            remap.points.push(ValuePointRemap { producer: before.producer.clone(), point: a.id, imported: b.id });
        }
        for shape in &before.tables.shapes {
            let rows = after.tables.shapes.iter().filter(|row| row.ty == shape.ty).collect::<Vec<_>>();
            let [actual] = rows.as_slice() else { panic!("one genuine independent primitive type counterpart") };
            remap.shapes.push((shape.id.clone(), actual.id.clone()));
        }
        remap.imports.push(ValueImportCorrespondence { producer: before.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    let a = original::catalog(&source.sources).expect("genuine original source resource catalog");
    let b = original::catalog(&destination.sources).expect("genuine independent target resource catalog");
    for import in &mut remap.imports {
        let owner = &remap.producers.iter().find(|pair| pair.0 == import.producer).expect("explicit original counterpart").1;
        let before = a.vertices.iter().filter(|row| row.reference.owner == import.producer).collect::<Vec<_>>();
        let after = b.vertices.iter().filter(|row| row.reference.owner == *owner).collect::<Vec<_>>();
        assert_eq!(before.len(), after.len());
        for (a, b) in before.iter().zip(after) {
            match (&a.reference.key, &b.reference.key) {
                (BoundaryResourceKey::Origin(a), BoundaryResourceKey::Origin(b)) => { assert_eq!(a.0, import.origins.len()); import.origins.push(*b); },
                (BoundaryResourceKey::Binding(a), BoundaryResourceKey::Binding(b)) => import.bindings.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Place(a), BoundaryResourceKey::Place(b)) => import.places.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Loan(a), BoundaryResourceKey::Loan(b)) => import.loans.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Runtime(a), BoundaryResourceKey::Runtime(b)) => import.runtime_values.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Environment(a), BoundaryResourceKey::Environment(b)) => import.environments.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Backing(a), BoundaryResourceKey::Backing(b)) => import.backings.push((a.clone(), b.clone())),
                (BoundaryResourceKey::Holder(a), BoundaryResourceKey::Holder(b)) => import.holders.push((a.clone(), b.clone())),
                _ => panic!("exact typed resource counterparts required"),
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
        3..=8 => {
            let row = tables.source_origin_derivations.iter_mut().find_map(|row| match row { SourceConstructorOriginDerivation::CheckedParameter(row) => Some(row), SourceConstructorOriginDerivation::AllocationIdentityAddress(_) | SourceConstructorOriginDerivation::DirectStorageAddress(_) | SourceConstructorOriginDerivation::NullExpression(_) => None }).expect("genuine local parameter incidence");
            match defect {
                3 => row.origin.origin = ReferenceOriginId(usize::MAX),
                4 => row.event.point = CfgPointId(usize::MAX),
                5 => row.declaration.site = SourceNameDeclarationSite::Parameter { ordinal: usize::MAX },
                6 => row.binding.declaration_span = ByteSpan::new(0, 0),
                7 => row.origin.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                8 => row.declaration.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                _ => unreachable!(),
            }
        },
        _ => unreachable!(),
    }
}

#[test]
fn checked_parameter_mixed_full_selective_owned_and_serde_original_correspondence() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let (source, forest) = original("source", layout, project);
            let (destination, target_forest) = original("destination", layout, project);
            let remap = mapping(&forest, &target_forest);
            let root = &source[0].producer;
            let mapped = remap.tables_into(&source, &forest, &destination, &target_forest).expect("independent Mapped whole mixed creator baseline");
            let copied = remap.tables(&source, &forest).expect("copied whole mixed creator baseline");
            assert_eq!(mapped[0].tables.source_origin_derivations, copied[0].tables.source_origin_derivations);
            assert_eq!(mapped[0].resources.checked_parameters().iter().filter(|row| row.source.origin.owner == *root).count(), 2);
            let witness = remap.independent_boundary_destination(&destination, &target_forest).expect("independent original headers");
            let uses = remap.source_use_graphs_into(root, &source[0].tables, &source, &forest, &witness).expect("actual mixed source-use baseline");
            let stages = remap.staged_loan_queries_into(root, &source[0].tables, &source, &forest, &witness).expect("actual mixed staged baseline");
            let saved = (source.clone(), forest.clone(), destination.clone(), target_forest.clone(), remap.clone(), witness.clone(), mapped.clone(), uses.clone(), stages.clone());
            for side in 0..2 {
                for defect in 0..9 {
                    let mut candidate = if side == 0 { source.clone() } else { destination.clone() };
                    corrupt(&mut candidate[0].tables, defect);
                    let before = candidate.clone();
                    if side == 0 {
                        assert!(remap.tables_into(&candidate, &forest, &destination, &target_forest).is_err());
                        assert!(remap.source_use_graphs_into(root, &candidate[0].tables, &candidate, &forest, &witness).is_err());
                        assert!(remap.staged_loan_queries_into(root, &candidate[0].tables, &candidate, &forest, &witness).is_err());
                    } else { assert!(remap.tables_into(&source, &forest, &candidate, &target_forest).is_err()); }
                    assert_eq!(candidate, before);
                }
            }
            for cut in [BoundaryResourceDemandCut::SourceUses { root: root.clone() }, BoundaryResourceDemandCut::StagedLoans { root: root.clone() }] {
                let checked = witness.checked(&remap, &source, &forest, &demand::resource_demand_full(&source)).expect("genuine original header and resource closure");
                postvalidation::selective(&remap, root, &source, &mapped[0].tables, &checked.headers, &checked.correspondence, &cut, &checked.resources).expect("actual selective mapped mixed baseline");
                let headers = (checked.headers.clone(), checked.correspondence.clone(), checked.resources.clone());
                for defect in 0..9 {
                    let mut candidate = mapped[0].tables.clone();
                    corrupt(&mut candidate, defect);
                    let before = candidate.clone();
                    assert!(postvalidation::selective(&remap, root, &source, &candidate, &checked.headers, &checked.correspondence, &cut, &checked.resources).is_err());
                    assert_eq!(candidate, before);
                    assert_eq!((&checked.headers, &checked.correspondence, &checked.resources), (&headers.0, &headers.1, &headers.2));
                }
            }
            let mut missing = mapped[0].resources.clone();
            missing.checked_parameters.clear();
            let before = missing.clone();
            assert!(checked_parameter::owned(&remap, &source, &missing).is_err());
            assert_eq!(missing, before);
            let decoded = serde_json::from_str::<EvaluatedValueTables>(&serde_json::to_string(&source[0].tables).expect("mixed candidate serialize")).expect("mixed candidate deserialize");
            let mut restored = source.clone();
            restored[0].tables = decoded;
            assert_eq!(remap.tables_into(&restored, &forest, &destination, &target_forest).expect("Serde candidate with independent original source evidence"), mapped);
            let identity = mapping(&forest, &forest);
            let same = identity.tables_into(&source, &forest, &source, &forest).expect("identity-mapped original mixed baseline");
            let shared = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
            let refs = checked_parameter::selected(&shared, same[0].resources.source_originals(), same[0].resources.associations()).expect("actual WHOLE SharedOriginal parameter associations");
            assert!(refs.iter().all(|row| row.source == row.destination));
            checked_parameter::original_pairs(&shared, same[0].resources.source_originals(), same[0].resources.source_originals()).expect("whole SharedOriginal snapshot baseline");
            assert_eq!(remap.tables_into(&source, &forest, &destination, &target_forest).expect("repeated immutable original mixed baseline"), mapped);
            assert_eq!((source, forest, destination, target_forest, remap, witness, mapped, uses, stages), saved);
        }
    }
}

fn empty_original(unit: &str, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let identity = SourceIdentity::new("parameter-empty".into(), unit.into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), "%%start\nunit(*u8, *!u8, u8) run = fn(shared, mutable, value) { shared; mutable; value; null; };\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("genuine original empty-demand forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual unused checked parameter constructor");
    let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("independent source original empty-demand context");
    (capture_table_sources(&function.reference_cfg, &input.policy_context).expect("genuine empty-demand source extraction"), forest)
}

#[test]
fn checked_parameter_empty_selection_preserves_whole_mixed_metadata_and_atomicity() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let (source, forest) = empty_original("source", layout, project);
            let (destination, target_forest) = empty_original("destination", layout, project);
            let remap = mapping(&forest, &target_forest);
            let root = &source[0].producer;
            let witness = remap.independent_boundary_destination(&destination, &target_forest).expect("genuine independent empty-demand headers");
            let uses = remap.source_use_graphs_into(root, &source[0].tables, &source, &forest, &witness).expect("actual empty source-use parameter baseline");
            let stages = remap.staged_loan_queries_into(root, &source[0].tables, &source, &forest, &witness).expect("actual empty staged parameter baseline");
            let saved = (source.clone(), forest.clone(), destination.clone(), target_forest.clone(), remap.clone(), witness.clone(), uses.clone(), stages.clone());
            let checked = witness.checked(&remap, &source, &forest, &BoundaryResourceDemand { roots: Vec::new() }).expect("actual zero-demand original owned resources");
            assert!(checked.resources.checked_parameters().is_empty());
            checked_parameter::owned(&remap, &source, &checked.resources).expect("whole parameter source validation with zero selected associations");
            let targets = checked.headers.iter().filter(|row| row.producer == remap.boundary_producer(root).expect("genuine mapped owner")).collect::<Vec<_>>();
            let [target] = targets.as_slice() else { panic!("one genuine original mapped header") };
            let mapped = remap.map_table(root, &source[0].tables, &checked.correspondence, &target.tables).expect("whole mapped vector despite empty owned selection");
            assert_eq!(mapped.source_origin_derivations.len(), 3);
            let prior = (checked.headers.clone(), checked.correspondence.clone(), checked.resources.clone(), mapped.clone());
            for cut in [BoundaryResourceDemandCut::SourceUses { root: root.clone() }, BoundaryResourceDemandCut::StagedLoans { root: root.clone() }] {
                assert!(demand::table_roots(root, &source[0].tables, &cut).is_empty());
                postvalidation::selective(&remap, root, &source, &mapped, &checked.headers, &checked.correspondence, &cut, &checked.resources).expect("actual empty selective mixed metadata baseline before defects");
                for defect in 0..9 {
                    let mut candidate = mapped.clone();
                    corrupt(&mut candidate, defect);
                    let before = candidate.clone();
                    assert!(postvalidation::selective(&remap, root, &source, &candidate, &checked.headers, &checked.correspondence, &cut, &checked.resources).is_err());
                    assert_eq!(candidate, before);
                    assert_eq!((&checked.headers, &checked.correspondence, &checked.resources, &mapped), (&prior.0, &prior.1, &prior.2, &prior.3));
                    assert_eq!((&source, &forest, &destination, &target_forest, &remap, &witness, &uses, &stages), (&saved.0, &saved.1, &saved.2, &saved.3, &saved.4, &saved.5, &saved.6, &saved.7));
                }
            }
            let mut missing = source.clone();
            missing[0].tables.source_origin_derivations.clear();
            let before = missing.clone();
            assert!(checked_parameter::owned(&remap, &missing, &checked.resources).is_err());
            assert_eq!(missing, before);
            assert_eq!(remap.source_use_graphs_into(root, &source[0].tables, &source, &forest, &witness).expect("repeated empty source-use baseline"), uses);
            assert_eq!(remap.staged_loan_queries_into(root, &source[0].tables, &source, &forest, &witness).expect("repeated empty staged baseline"), stages);
        }
    }
}
