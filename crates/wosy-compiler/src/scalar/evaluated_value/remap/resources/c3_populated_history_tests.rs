use super::*;

const SOURCE: &str = "%%start\nunit() run = fn { u32 item = 1; unit() write = fn { item = 2; }; write(); };\n%%end";

fn original(identity: SourceIdentity, layout: ScalarTargetLayout, project: bool) {
    let parsed = crate::parse_source(identity.clone(), SOURCE.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("actual complete module and function constructors");
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual run declaration");
    let cfg = &function.reference_cfg;
    let input = source_boundary::preflight_input(&cfg.producer_owner, &forest, BTreeMap::new()).expect("external retained original preflight input");
    source_boundary::validate_capture_preflight(cfg, &input).expect("actual complete populated source");
    let sources = capture_table_sources(cfg, &input.policy_context).expect("selected actual parent and entire literal tree");
    let tuples = original_capture_tuples(&sources).expect("actual populated whole input tuples");
    assert!(tuples.iter().any(|tuple| !tuple.production.inputs.is_empty()));
    assert!(sources.iter().all(|row| row.capture_inventory.phase == SourceCaptureConstructionPhase::Complete && row.tables.source_capture_pending.is_empty()));
    assert!(forest.sources.len() > sources.len(), "original module spectator retained outside selected tree");
    let catalog = original::catalog(&sources).expect("actual typed original resource constructors");
    let mut remap = ValueSchemaRemap {
        producers: sources.iter().map(|row| (row.producer.clone(), row.producer.clone())).collect(),
        points: sources.iter().flat_map(|row| row.initial_points.iter().map(|point| ValuePointRemap { producer: row.producer.clone(), point: point.id, imported: point.id })).collect(),
        shapes: sources.iter().flat_map(|row| row.tables.shapes.iter().map(|shape| (shape.id.clone(), shape.id.clone()))).collect(), imports: Vec::new(),
    };
    for row in &sources {
        let mut import = ValueImportCorrespondence { producer: row.producer.clone(), bindings: Vec::new(), places: Vec::new(), origins: row.resource_construction.original().facts.iter().enumerate().map(|(index, _)| ReferenceOriginId(index)).collect(), loans: Vec::new(), runtime_values: Vec::new(), environments: Vec::new(), backings: Vec::new(), holders: Vec::new() };
        for vertex in catalog.vertices.iter().filter(|vertex| vertex.reference.owner == row.producer) {
            match &vertex.reference.key {
                BoundaryResourceKey::Origin(_) => {}
                BoundaryResourceKey::Binding(id) => import.bindings.push((id.clone(), id.clone())),
                BoundaryResourceKey::Place(id) => import.places.push((id.clone(), id.clone())),
                BoundaryResourceKey::Loan(id) => import.loans.push((id.clone(), id.clone())),
                BoundaryResourceKey::Runtime(id) => import.runtime_values.push((id.clone(), id.clone())),
                BoundaryResourceKey::Environment(id) => import.environments.push((id.clone(), id.clone())),
                BoundaryResourceKey::Backing(id) => import.backings.push((id.clone(), id.clone())),
                BoundaryResourceKey::Holder(id) => import.holders.push((id.clone(), id.clone())),
            }
        }
        remap.imports.push(import);
    }
    for independent in [false, true] {
        let witness = if independent { remap.independent_boundary_destination(&sources, &forest) } else { remap.stage_boundary_sources(&sources, &forest) }.expect("actual populated witness");
        let saved = (sources.clone(), forest.clone(), remap.clone(), witness.clone());
        let result = if independent { remap.tables_into(&sources, &forest, &sources, &forest) } else { remap.tables(&sources, &forest) }.expect("actual identity whole graph transport");
        assert_eq!(result.len(), sources.len());
        for carrier in &sources {
            let matches = result.iter().filter(|row| row.source == carrier.producer).collect::<Vec<_>>();
            let [mapped] = matches.as_slice() else { panic!("unique source-tagged returned result"); };
            assert_eq!(mapped.destination, carrier.producer);
            assert_eq!(mapped.tables, carrier.tables);
            assert_eq!(mapped.capture_inventory, carrier.capture_inventory);
            assert_eq!(mapped.children, carrier.children);
            assert_eq!(mapped.policy_context, carrier.policy_context);
            assert_eq!(mapped.resources.capture_inputs.iter().map(|pair| pair.source.clone()).collect::<Vec<_>>(), tuples);
            assert!(mapped.resources.capture_inputs.iter().all(|pair| pair.source == pair.destination));
            for child in &mapped.children { assert_eq!(result.iter().filter(|row| row.destination == child.child).count(), 1); }
            let uses = remap.source_use_graphs_into(&carrier.producer, &carrier.tables, &sources, &forest, &witness).expect("genuinely rooted selective source use");
            let stages = remap.staged_loan_queries_into(&carrier.producer, &carrier.tables, &sources, &forest, &witness).expect("genuinely rooted selective stages");
            assert_eq!(uses.graphs, carrier.tables.source_use_graphs);
            assert_eq!(stages.queries, carrier.tables.staged_loan_queries);
            for resources in [&uses.resources, &stages.resources] {
                assert_eq!(resources.capture_inputs.iter().map(|pair| pair.source.clone()).collect::<Vec<_>>(), tuples);
                assert!(resources.capture_inputs.iter().all(|pair| pair.source == pair.destination));
            }
        }
        assert_eq!((sources.clone(), forest.clone(), remap.clone(), witness.clone()), saved);
    }
    let restored: ScalarReferenceCfg = serde_json::from_str(&serde_json::to_string(cfg).expect("serialize original static history")).expect("restore static history only");
    for history in [cfg.clone(), restored] {
        assert_eq!(history, *cfg);
        let saved = (history.clone(), input.clone());
        source_boundary::validate_capture_preflight(&history, &input).expect("history validated with external ORIGINAL forest, zero serialized evidence");
        assert_eq!((history.clone(), input.clone()), saved);
        let mut malformed = history;
        malformed.evaluated_values.source_capture_productions.clear();
        let saved = (malformed.clone(), input.clone());
        let errors = reference::retransfer_evaluated_borrow_test(&mut malformed, &input);
        assert!(!errors.is_empty());
        assert!(errors.iter().all(|error| matches!(error, ReferenceAnalysisError::MissingContext { reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence, .. })));
        assert_eq!((malformed.clone(), input.clone()), saved);
        assert!(source_boundary::validate_capture_preflight(&malformed, &input).is_err());
        assert_eq!((malformed, input.clone()), saved);
    }
    let target_identity = SourceIdentity::new("c3".into(), "independent-target".into(), "main.w".into(), "r1".into());
    let target_parsed = crate::parse_source(target_identity.clone(), SOURCE.into(), &[]);
    assert!(target_parsed.diagnostics.is_empty());
    let (target_validation, _) = derive_scalar_program_observed_test(&target_parsed.result, layout);
    assert!(target_validation.diagnostics.is_empty(), "{:?}", target_validation.diagnostics);
    let target_module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(target_validation.program, Vec::new())], vec![target_identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(target_validation.program, Vec::new()) };
    let target_forest = source_boundary::module_preflight_forest(&target_module.source, &target_module.items, &target_module.top_level_reference_cfg, &target_module.structs, &target_module.enums, SourceCaptureConstructionPhase::Complete).expect("separately constructed destination original");
    let target_function = target_module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual destination run");
    let target_input = source_boundary::preflight_input(&target_function.reference_cfg.producer_owner, &target_forest, BTreeMap::new()).expect("destination original external input");
    let target_sources = capture_table_sources(&target_function.reference_cfg, &target_input.policy_context).expect("actual destination literal tree");
    let target_tuples = original_capture_tuples(&target_sources).expect("actual destination populated tuples");
    let [source_tuple] = tuples.as_slice() else { panic!("one actual original Capture"); };
    let [target_tuple] = target_tuples.as_slice() else { panic!("one actual destination Capture"); };
    assert!(!source_tuple.production.inputs.is_empty());
    assert_eq!(source_tuple.production.signature, target_tuple.production.signature);
    assert_eq!(source_tuple.production.output.resolved.occurrence.range, target_tuple.production.output.resolved.occurrence.range);
    assert_ne!(source_tuple.production.output.resolved.occurrence.source, target_tuple.production.output.resolved.occurrence.source);
    let boundary_remap = ValueSchemaRemap {
        producers: vec![(source_tuple.production.event.owner.clone(), target_tuple.production.event.owner.clone())],
        points: vec![ValuePointRemap { producer: source_tuple.production.event.owner.clone(), point: source_tuple.production.event.point, imported: target_tuple.production.event.point }], shapes: Vec::new(), imports: Vec::new(),
    };
    let saved = (sources.clone(), forest.clone(), target_sources.clone(), target_forest.clone(), boundary_remap.clone());
    let mapped_output = boundary_remap.value(&source_tuple.production.output).expect("isolated original-occurrence mapper boundary, zero productive correspondence authority");
    assert_eq!(mapped_output.resolved.producer, target_tuple.production.output.resolved.producer);
    assert_eq!(mapped_output.point, target_tuple.production.output.point);
    assert_eq!(mapped_output.resolved.occurrence, source_tuple.production.output.resolved.occurrence);
    assert_ne!(mapped_output, target_tuple.production.output);
    assert_eq!((sources, forest, target_sources, target_forest, boundary_remap), saved);
}

#[test]
fn populated_identity_history_occurrence_boundary_native64_program() { original(SourceIdentity::new("c3".into(), "original".into(), "main.w".into(), "r1".into()), ScalarTargetLayout::NATIVE64, false); }
#[test]
fn populated_identity_history_occurrence_boundary_native64_project() { original(SourceIdentity::new("c3".into(), "original".into(), "main.w".into(), "r1".into()), ScalarTargetLayout::NATIVE64, true); }
#[test]
fn populated_identity_history_occurrence_boundary_wasm32_program() { original(SourceIdentity::new("c3".into(), "original".into(), "main.w".into(), "r1".into()), ScalarTargetLayout::WASM32, false); }
#[test]
fn populated_identity_history_occurrence_boundary_wasm32_project() { original(SourceIdentity::new("c3".into(), "original".into(), "main.w".into(), "r1".into()), ScalarTargetLayout::WASM32, true); }
