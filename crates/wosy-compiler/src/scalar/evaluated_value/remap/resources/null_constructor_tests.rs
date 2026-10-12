use super::*;
use source_boundary::null_constructor::SourceConstructorOriginDerivation;

fn original(unit: &str, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let identity = SourceIdentity::new("null-qualified".into(), unit.into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), "%%start\n*i32() run = fn { null };\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("genuine complete original factory forest");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual original named Null body");
    let input = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("independent typed function source context");
    (capture_table_sources(&function.reference_cfg, &input.policy_context).expect("genuine original source extraction"), forest)
}

fn mapping(source: &SourceCaptureConstructionForest, destination: &SourceCaptureConstructionForest) -> ValueSchemaRemap {
    assert_eq!(source.sources.len(), destination.sources.len());
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
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
            let rows = after.tables.shapes.iter().filter(|candidate| candidate.ty == shape.ty).collect::<Vec<_>>();
            let [actual] = rows.as_slice() else { panic!("one independently parsed primitive shape counterpart"); };
            remap.shapes.push((shape.id.clone(), actual.id.clone()));
        }
        remap.imports.push(ValueImportCorrespondence { producer: before.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    let a = original::catalog(&source.sources).expect("genuine original source resource catalog");
    let b = original::catalog(&destination.sources).expect("genuine independent destination resource catalog");
    for import in &mut remap.imports {
        let owner = &remap.producers.iter().find(|pair| pair.0 == import.producer).expect("explicit original owner counterpart").1;
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
                _ => panic!("distinct original resource kinds require their own typed counterpart"),
            }
        }
    }
    remap
}

#[test]
fn qualified_null_full_selective_header_owned_and_repeated_original_transport() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let (source, forest) = original("source", layout, project);
            let (destination, target_forest) = original("destination", layout, project);
            let remap = mapping(&forest, &target_forest);
            let root = &source[0].producer;
            let saved = (source.clone(), forest.clone(), destination.clone(), target_forest.clone(), remap.clone());
            assert_eq!(source[0].tables.source_origin_derivations.len(), 1);
            let mapped = remap.tables_into(&source, &forest, &destination, &target_forest).expect("genuine independent full Null counterpart baseline");
            assert_eq!(mapped[0].tables.source_origin_derivations, destination[0].tables.source_origin_derivations);
            assert_eq!(mapped[0].resources.null_constructors.len(), 1);
            null_constructor::full(&remap, &source, &mapped, &mapped[0].resources).expect("owned full Null original inventory replay");
            let copied = remap.tables(&source, &forest).expect("original copied full Null family baseline");
            assert_eq!(copied[0].tables.source_origin_derivations, mapped[0].tables.source_origin_derivations);
            let witness = remap.independent_boundary_destination(&destination, &target_forest).expect("genuine independent original headers");
            let uses = remap.source_use_graphs_into(root, &source[0].tables, &source, &forest, &witness).expect("independent selective source-use Null originals baseline");
            let stages = remap.staged_loan_queries_into(root, &source[0].tables, &source, &forest, &witness).expect("independent selective staged Null originals baseline");
            null_constructor::owned(&remap, &source, &uses.resources).expect("whole original Null source retained on selective owned replay");
            null_constructor::owned(&remap, &source, &stages.resources).expect("whole original Null source retained on staged owned replay");
            let empty = witness.checked(&remap, &source, &forest, &BoundaryResourceDemand { roots: Vec::new() }).expect("genuine empty original resource selection");
            assert!(empty.resources.null_constructors.is_empty());
            null_constructor::owned(&remap, &source, &empty.resources).expect("whole original Null validation with zero selected refs");
            let mut missing = source.clone();
            missing[0].tables.source_origin_derivations.clear();
            let before = missing.clone();
            assert!(null_constructor::owned(&remap, &missing, &empty.resources).is_err());
            assert_eq!(missing, before);
            let mut extra = empty.resources.clone();
            extra.null_constructors = mapped[0].resources.null_constructors.clone();
            let extra_before = extra.clone();
            assert!(null_constructor::owned(&remap, &source, &extra).is_err());
            assert_eq!(extra, extra_before);
            let serialized = serde_json::to_string(&source[0].tables).expect("candidate qualified Null refs serialize");
            let decoded = serde_json::from_str::<EvaluatedValueTables>(&serialized).expect("candidate qualified Null refs decode");
            assert_eq!(decoded, source[0].tables);
            let mut restored = source.clone();
            restored[0].tables = decoded;
            assert_eq!(remap.tables_into(&restored, &forest, &destination, &target_forest).expect("Serde candidate with independently retained originals"), mapped);
            assert_eq!(remap.tables_into(&source, &forest, &destination, &target_forest).expect("repeated original transport"), mapped);
            assert_eq!((source, forest, destination, target_forest, remap), saved);
        }
    }
}

#[test]
fn qualified_null_missing_redirected_and_extra_candidate_refs_are_atomic() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let (source, forest) = original("source", layout, project);
            let (destination, target_forest) = original("destination", layout, project);
            let remap = mapping(&forest, &target_forest);
            let mapped = remap.tables_into(&source, &forest, &destination, &target_forest).expect("accepted independent source baseline before defects");
            let witness = remap.independent_boundary_destination(&destination, &target_forest).expect("original destination header baseline");
            for side in 0..2 {
                for defect in 0..5 {
                    let mut candidate = if side == 0 { source.clone() } else { destination.clone() };
                    match defect {
                        0 => candidate[0].tables.source_origin_derivations.clear(),
                        1 => { let extra = candidate[0].tables.source_origin_derivations[0].clone(); candidate[0].tables.source_origin_derivations.push(extra); },
                        2 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut candidate[0].tables.source_origin_derivations[0] else { panic!("genuine Null fixture") }; row.origin.origin = ReferenceOriginId(usize::MAX); },
                        3 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut candidate[0].tables.source_origin_derivations[0] else { panic!("genuine Null fixture") }; row.event.point = CfgPointId(usize::MAX); },
                        4 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut candidate[0].tables.source_origin_derivations[0] else { panic!("genuine Null fixture") }; row.origin.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())); },
                        _ => unreachable!(),
                    }
                    let saved = candidate.clone();
                    if side == 0 {
                        assert!(remap.tables_into(&candidate, &forest, &destination, &target_forest).is_err());
                        assert!(remap.source_use_graphs_into(&source[0].producer, &candidate[0].tables, &candidate, &forest, &witness).is_err());
                        assert!(remap.staged_loan_queries_into(&source[0].producer, &candidate[0].tables, &candidate, &forest, &witness).is_err());
                    } else { assert!(remap.tables_into(&source, &forest, &candidate, &target_forest).is_err()); }
                    assert_eq!(candidate, saved);
                }
            }
            let mut removed = mapped.clone();
            removed[0].tables.source_origin_derivations.clear();
            let saved = removed.clone();
            assert!(null_constructor::full(&remap, &source, &removed, &mapped[0].resources).is_err());
            assert_eq!(removed, saved);
            for defect in 0..4 {
                let mut resources = mapped[0].resources.clone();
                match defect {
                    0 => resources.null_constructors.clear(),
                    1 => { let extra = resources.null_constructors[0].clone(); resources.null_constructors.push(extra); },
                    2 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut resources.null_constructors[0].destination else { panic!("genuine Null fixture") }; row.event.point = CfgPointId(usize::MAX); },
                    3 => { let SourceConstructorOriginDerivation::NullExpression(row) = &mut resources.null_constructors[0].destination else { panic!("genuine Null fixture") }; row.origin.origin = ReferenceOriginId(usize::MAX); },
                    _ => unreachable!(),
                }
                let saved = resources.clone();
                assert!(null_constructor::owned(&remap, &source, &resources).is_err());
                assert_eq!(resources, saved);
            }
            let identity = mapping(&forest, &forest);
            let identity_result = identity.tables_into(&source, &forest, &source, &forest).expect("identity-mapped same original Null baseline");
            assert_eq!(identity_result[0].tables.source_origin_derivations, source[0].tables.source_origin_derivations);
            let shared = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
            let refs = null_constructor::selected(&shared, identity_result[0].resources.source_originals(), identity_result[0].resources.associations()).expect("actual SharedOriginal Null identity refs");
            assert_eq!(refs.len(), 1);
            assert_eq!(refs[0].source, refs[0].destination);
            null_constructor::selected_originals(&shared, identity_result[0].resources.source_originals(), identity_result[0].resources.source_originals(), &refs).expect("WHOLE SharedOriginal snapshot counterpart");
        }
    }
}

#[path = "null_selective_tests.rs"]
mod selective;
