use super::*;

const MAKE: &str = "%%start\n(u8())() make = fn { u8() inner = fn { 1 }; inner };\nunit() run = fn { u8() spare = fn { 0 }; spare(); u8() first = make(); u8() probe = fn { first() }; probe(); };\n%%end";
const BRANCH: &str = "%%start\n(u8())() make_a = fn { u8() inner = fn { 1 }; inner };\n(u8())() make_b = fn { u8() inner = fn { 2 }; inner };\nunit(bool) run = fn(flag) { u8() spare = fn { 0 }; spare(); u8() target = if (flag) { make_a() } else { make_b() }; u8() alias = target; u8() probe = fn { alias() }; probe(); };\n%%end";

fn input(text: &str, layout: ScalarTargetLayout, project: bool) -> (Vec<ValueTableSource>, SourceCaptureConstructionForest) {
    let source = SourceIdentity::new("i2-callee-correction".into(), "owned".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validation = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source]));
        assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
        validation.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("real original forest before correction matrices");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual declared original caller");
    let context = source_boundary::preflight_input(&function.reference_cfg.producer_owner, &forest, BTreeMap::new()).expect("exact original policy context");
    (capture_table_sources(&function.reference_cfg, &context.policy_context).expect("whole selected original Capture tree"), forest)
}

fn identity(forest: &SourceCaptureConstructionForest) -> ValueSchemaRemap {
    let mut remap = ValueSchemaRemap { producers: Vec::new(), points: Vec::new(), shapes: Vec::new(), imports: Vec::new() };
    for row in &forest.sources {
        remap.producers.push((row.producer.clone(), row.producer.clone()));
        for point in &row.initial_points { remap.points.push(ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: point.id }); }
        for shape in &row.tables.shapes { remap.shapes.push((shape.id.clone(), shape.id.clone())); }
        remap.imports.push(ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: Vec::new(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() });
    }
    let catalog = original::catalog(&forest.sources).expect("genuine original whole provisional catalog");
    c3_key_builders::install(&mut remap, &catalog, &catalog, forest);
    remap
}

fn owned(text: &str, layout: ScalarTargetLayout, project: bool) {
    let (source, forest) = input(text, layout, project);
    let remap = identity(&forest);
    let saved = (source.clone(), forest.clone(), remap.clone());
    let witness = remap.independent_boundary_destination(&source, &forest).expect("actual original Independent witness");
    let (_, correspondence) = witness.checked_headers(&remap, &source, &forest).expect("original callee/header factory");
    let full = remap.tables_into(&source, &forest, &source, &forest).expect("full genuine original imported callee evidence");
    original_catalogue_negatives(&source, &forest, &remap);
    assert!(source.iter().any(|row| row.resource_construction.original().environment_imports.iter().any(|import| import.source_index != import.destination.index)));
    for row in &source {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &source, &forest, &witness).expect("source-use imported callee evidence");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &source, &forest, &witness).expect("staged imported callee evidence");
        for resources in [&full[0].resources, &uses.resources, &stages.resources] {
            header_counterpart::postvalidate(&remap, &source, &correspondence, resources).expect("repeat original qualified callee constructor paths on ALL owned cuts");
            postvalidation::capture_inputs(&remap, &source, &correspondence, resources).expect("repeat complete Capture helper on ALL owned cuts");
        }
        let cfg = row.resource_construction.original();
        for call in &cfg.points {
            let CfgPointKind::Call { target: target @ (ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. }), .. } = &call.kind else { continue; };
            let proof = paths(&forest, cfg, call, target).expect("qualified original callee constructor baseline");
            assert!(paired(&remap, &cfg.producer_owner, &proof, &proof).expect("actual owner-tagged whole path correspondence"));
            for defect in 0..4 {
                let mut candidate = cfg.clone();
                let import = candidate.environment_imports.iter().position(|entry| call.possible_callee_environments.contains(&entry.destination));
                match defect {
                    0 => { let Some(index) = import else { continue; }; candidate.environment_imports[index].source_index = usize::MAX; }
                    1 => { let Some(index) = import else { continue; }; candidate.environment_imports[index].table.owner = cfg.producer_owner.clone(); }
                    2 => { let Some(index) = import else { continue; }; candidate.environment_imports[index].destination_instance = CallableEnvironmentInstance::Capture; }
                    3 if !candidate.capture_input_path_facts.is_empty() => candidate.capture_input_path_facts[0].owner = cfg.producer_owner.clone(),
                    _ => continue,
                }
                let snapshot = candidate.clone();
                let result = paths(&forest, &candidate, call, target);
                assert!(result.is_err());
                assert_eq!(candidate, snapshot);
                assert_eq!(paths(&forest, cfg, call, target).expect("unchanged genuine original after rejected candidate"), proof);
            }
        }
    }
    let mut empty = source.clone(); let mut empty_forest = forest.clone();
    for row in &mut empty {
        row.tables.source_use_graphs.clear(); row.tables.staged_loan_queries.clear();
        empty_forest.sources.iter_mut().find(|actual| actual.producer == row.producer).expect("actual selected original carrier").tables = row.tables.clone();
    }
    let witness = remap.independent_boundary_destination(&empty, &empty_forest).expect("empty output cut retains original constructors");
    let (_, correspondence) = witness.checked_headers(&remap, &empty, &empty_forest).expect("empty owned cut header evidence");
    for row in &empty {
        let uses = remap.source_use_graphs_into(&row.producer, &row.tables, &empty, &empty_forest, &witness).expect("empty source-use original callee paths");
        let stages = remap.staged_loan_queries_into(&row.producer, &row.tables, &empty, &empty_forest, &witness).expect("empty staged original callee paths");
        assert!(uses.graphs.is_empty()); assert!(stages.queries.is_empty());
        for resources in [&uses.resources, &stages.resources] { header_counterpart::postvalidate(&remap, &empty, &correspondence, resources).expect("empty owned cut mandatory whole constructor proof"); }
    }
    assert_eq!((source, forest, remap), saved);
}

fn original_catalogue_negatives(source: &[ValueTableSource], forest: &SourceCaptureConstructionForest, remap: &ValueSchemaRemap) {
    let root = &source[0];
    let cfg = root.resource_construction.original();
    let calls = cfg.initial_points.iter().filter(|point| matches!(point.kind, CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { .. }, .. })).collect::<Vec<_>>();
    assert!(!calls.is_empty());
    for call in calls {
        let CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { source: target_source, declaration_span, .. }, .. } = &call.kind else { unreachable!(); };
        let index = cfg.call_signatures.iter().position(|signature| signature.source == *target_source && signature.declaration_span == *declaration_span).expect("original exact root signature key");
        for defect in 0..6 {
            let mut original = cfg.clone();
            match defect {
                0 => original.call_signatures[index].declaration_span = ByteSpan::new(0, 0),
                1 => original.call_signatures[index].parameters.push(ScalarType::Error),
                2 => original.call_signatures[index].outputs[0] = ScalarType::Error,
                3 => original.call_signatures[index].generic_parameters.push("wrong-original-key".into()),
                4 => { let duplicate = original.call_signatures[index].clone(); original.call_signatures.push(duplicate); }
                5 => {
                    let point = original.initial_points.iter_mut().find(|point| point.id == call.id).expect("whole original registered Call");
                    let CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { declaration_span, .. }, .. } = &mut point.kind else { unreachable!(); };
                    *declaration_span = ByteSpan::new(0, 0);
                    let kind = point.kind.clone();
                    original.points.iter_mut().find(|point| point.id == call.id).expect("actual original Call instance").kind = kind;
                }
                _ => unreachable!(),
            }
            let candidate = capture_table_sources(&original, &root.policy_context).expect("fresh corrupted whole original carriers with preserved producer topology");
            let mut candidate_forest = forest.clone();
            for row in &candidate { *candidate_forest.sources.iter_mut().find(|row_before| row_before.producer == row.producer).expect("exact original carrier location") = row.clone(); }
            let saved = (candidate.clone(), candidate_forest.clone(), source.to_vec(), forest.clone(), remap.clone());
            for (before, before_forest, after, after_forest) in [(source, forest, candidate.as_slice(), &candidate_forest), (candidate.as_slice(), &candidate_forest, source, forest)] {
                assert!(remap.boundary_arena_correspondence(before, before_forest, after, after_forest).is_err());
            }
            assert_eq!((candidate, candidate_forest, source.to_vec(), forest.clone(), remap.clone()), saved);
        }
    }
    let mut candidate = forest.clone();
    let row = candidate.sources.iter_mut().find(|row| row.producer == root.producer).expect("actual original root");
    let child = row.children[0].clone(); row.children.push(child);
    let saved = candidate.clone();
    assert!(remap.boundary_arena_correspondence(source, forest, source, &candidate).is_err());
    assert!(remap.boundary_arena_correspondence(source, &candidate, source, forest).is_err());
    assert_eq!(candidate, saved);
}

#[test]
fn imported_reindexed_alias_native64_program() { for text in [MAKE, BRANCH] { owned(text, ScalarTargetLayout::NATIVE64, false); } }
#[test]
fn imported_reindexed_alias_native64_project() { for text in [MAKE, BRANCH] { owned(text, ScalarTargetLayout::NATIVE64, true); } }
#[test]
fn imported_reindexed_alias_wasm32_program() { for text in [MAKE, BRANCH] { owned(text, ScalarTargetLayout::WASM32, false); } }
#[test]
fn imported_reindexed_alias_wasm32_project() { for text in [MAKE, BRANCH] { owned(text, ScalarTargetLayout::WASM32, true); } }
