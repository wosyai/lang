use super::*;

fn fixture(text: &str, layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("c2".into(), "capture".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, construction) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).unwrap();
    assert!(forest.pending.is_empty());
    let associations = ValueSchemaRemap::validate_construction_forest(&forest).unwrap();
    assert!(!associations.is_empty());
    let module_input = source_boundary::preflight_input(&module.top_level_reference_cfg.producer_owner, &forest, BTreeMap::new()).unwrap();
    source_boundary::validate_capture_preflight(&module.top_level_reference_cfg, &module_input).unwrap();
    let mut erased_productive = module_input.clone();
    erased_productive.construction_forest.sources.retain(|row| matches!(row.producer, ProducerCfgOwnerId::Module(_)));
    let original_module = module.top_level_reference_cfg.clone();
    assert!(source_boundary::validate_capture_preflight(&original_module, &erased_productive).is_err());
    assert_eq!(module.top_level_reference_cfg, original_module);
    for pass in construction.passes.iter().filter(|pass| pass.stage == ReferenceTransferTestStage::InitialConstruction) {
        assert_eq!(pass.cfg_at_entry.source_capture_inventory.phase, SourceCaptureConstructionPhase::Building);
        source_boundary::validate_capture_preflight(&pass.cfg_at_entry, &pass.context.capture_input).unwrap();
        if pass.role == ReferenceTransferTestRole::TopLevel {
            assert_eq!(pass.context.capture_input.construction_forest.phase, SourceCaptureConstructionPhase::Building);
            assert!(!pass.context.capture_input.construction_forest.pending.is_empty());
        }
        for requirement in &pass.cfg_at_entry.source_capture_inventory.required {
            assert!(pass.cfg_at_entry.evaluated_values.source_capture_pending.iter().any(|row| row.event == requirement.event));
            assert!(pass.cfg_at_entry.evaluated_values.source_capture_productions.is_empty());
        }
    }
    for row in &forest.sources {
        assert_eq!(row.capture_inventory.phase, SourceCaptureConstructionPhase::Complete);
        assert!(row.tables.source_capture_pending.is_empty());
        assert_eq!(row.capture_inventory.required.len(), row.tables.source_capture_productions.len());
        for required in &row.capture_inventory.required {
            let production = row.tables.source_capture_productions.iter().find(|production| production.event == required.event).unwrap();
            assert_eq!(production.output, required.output);
            assert_eq!(production.signature, required.signature);
            assert_eq!(production.child, required.child);
            assert_eq!(production.inputs.iter().map(|port| port.capture.clone()).collect::<Vec<_>>(), required.captures);
            for port in &production.inputs {
                assert_eq!(port.policy, copy_policy(&port.capture.ty, &row.policy_context.structs, &row.policy_context.enums, &row.policy_context.generic_parameters).unwrap());
            }
        }
    }
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).unwrap();
    let cfg = &function.reference_cfg;
    let input = source_boundary::preflight_input(&cfg.producer_owner, &forest, BTreeMap::new()).unwrap();
    source_boundary::validate_capture_preflight(cfg, &input).unwrap();
    if cfg.source_capture_inventory.required.is_empty() { return; }
    for defect in 0..16 {
        let mut candidate = cfg.clone();
        let mut retained = input.clone();
        match defect {
            0 => candidate.evaluated_values.source_capture_productions.clear(),
            1 => candidate.source_capture_inventory.required.clear(),
            2 => candidate.literal_cfgs.clear(),
            3 => {
                candidate.evaluated_values.source_capture_productions.clear();
                candidate.source_capture_inventory.required.clear();
            }
            4 => retained.construction_forest.sources.retain(|row| row.producer != cfg.producer_owner),
            5 => candidate.literal_cfgs[0].reference_cfg.points.retain(|point| !matches!(point.kind, CfgPointKind::Entry)),
            6 => candidate.evaluated_values.source_capture_productions.push(candidate.evaluated_values.source_capture_productions[0].clone()),
            7 => candidate.literal_cfgs[0].capture_point = CfgPointId(usize::MAX),
            8 => retained.construction_forest.sources.retain(|row| row.producer != cfg.literal_cfgs[0].reference_cfg.producer_owner),
            9 => retained.construction_forest.roots.clear(),
            10 => { retained.policy_context.generic_parameters.insert("foreign".into()); }
            11 => {
                if candidate.evaluated_values.source_capture_productions[0].inputs.is_empty() {
                    candidate.evaluated_values.source_capture_productions.clear();
                } else { candidate.evaluated_values.source_capture_productions[0].inputs.clear(); }
            }
            12 => {
                if candidate.evaluated_values.source_capture_productions[0].inputs.is_empty() {
                    candidate.evaluated_values.source_capture_productions.clear();
                } else { candidate.evaluated_values.source_capture_productions[0].inputs[0].formal.port = usize::MAX; }
            }
            13 => candidate.literal_cfgs[0].reference_cfg.evaluated_values.boundary_signatures.clear(),
            14 => {
                let point = candidate.source_capture_inventory.required[0].event.point;
                candidate.points[point.0].kind = CfgPointKind::Entry;
            }
            15 => candidate.source_capture_inventory.required.push(candidate.source_capture_inventory.required[0].clone()),
            _ => unreachable!(),
        }
        let original_candidate = candidate.clone();
        let original_input = retained.clone();
        let mut trace = ReferenceValidationTestTrace::new();
        let errors = transfer_reference_facts(&mut candidate, &BTreeMap::new(), &[], &[], &[], &retained, ReferenceTransferTestRole::Function, &mut trace);
        assert!(!errors.is_empty(), "defect={defect}");
        assert!(errors.iter().all(|error| matches!(error, ReferenceAnalysisError::MissingContext { reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence, .. })));
        for error in &errors {
            let ReferenceAnalysisError::MissingContext { source_span, .. } = error else { unreachable!(); };
            assert_eq!(source_span.source, module.source);
            assert!(forest.sources.iter().flat_map(|row| &row.initial_points).any(|point| point.source_span == *source_span));
        }
        assert_eq!(candidate, original_candidate);
        assert_eq!(retained, original_input);
        assert_eq!(trace.passes.len(), 1);
        assert!(trace.passes[0].observations.points.is_empty());
        assert!(trace.passes[0].observations.borrows.is_empty());
        let before_refresh = candidate.clone();
        let mut refresh_trace = ReferenceValidationTestTrace::new();
        assert!(!refresh_literal_cfgs(&mut candidate, &[], &[], &retained, &mut refresh_trace).is_empty());
        assert_eq!(candidate, before_refresh);
        assert!(refresh_trace.passes.is_empty());
    }
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    for text in [
        "%%start\nunit() run = fn { unit() empty = fn { }; empty(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; unit() write = fn { item = 2; }; write(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *child = 2; }; write(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; unit() outer = fn { unit() inner = fn { item = 2; }; inner(); }; outer(); };\n%%end",
        "%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; unit() read = fn { pair.item; }; read(); };\n%%end",
        "%%start\nunit(u64) run = fn(count) { u8[count] slots; u8() read = fn { slots[0] }; read(); };\n%%end",
        "%%start\nunit() run = fn { u8() first = fn { 1 }; u8() second = fn { first() }; second(); };\n%%end",
    ] { fixture(text, layout, project); }
}

#[test]
fn c2_actual_capture_native64_program() { matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn c2_actual_capture_native64_project() { matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn c2_actual_capture_wasm32_program() { matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn c2_actual_capture_wasm32_project() { matrix(ScalarTargetLayout::WASM32, true); }

#[test]
fn c2_actual_selected_concrete_capture_instance_erasure() {
    let text = "%%start\ngeneric T;\nunit(T) run = fn(item) { unit() read = fn { item; }; read(); };\nunit() invoke = fn { run<u32>(1); };\n%%end";
    for (layout, project) in [
        (ScalarTargetLayout::NATIVE64, false), (ScalarTargetLayout::NATIVE64, true),
        (ScalarTargetLayout::WASM32, false), (ScalarTargetLayout::WASM32, true),
    ] {
        let source = SourceIdentity::new("c2".into(), "concrete".into(), "main.w".into(), "r1".into());
        let parsed = crate::parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let (validated, construction_trace) = derive_scalar_program_observed_test(&parsed.result, layout);
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        let trace = if project {
            let project = ScalarProject::new(vec![ScalarModule::from_program(validated.program, Vec::new())], vec![source]);
            let (validated, trace) = validate_scalar_project_observed_test(project);
            assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
            trace
        } else { construction_trace };
        let pass = trace.passes.iter().find(|pass| matches!(pass.role, ReferenceTransferTestRole::SpecializedCopy { .. }) && !pass.cfg_at_entry.evaluated_values.source_capture_policy_instances.is_empty()).unwrap();
        let input = &pass.context.capture_input;
        assert_eq!(input.substitutions.get("T"), Some(&ScalarType::U32));
        source_boundary::validate_capture_preflight(&pass.cfg_at_entry, input).unwrap();
        let mut malformed = pass.cfg_at_entry.clone();
        malformed.evaluated_values.source_capture_policy_instances.clear();
        let before = malformed.clone();
        let original_input = input.clone();
        let mut negative_trace = ReferenceValidationTestTrace::new();
        let errors = transfer_reference_facts(&mut malformed, &pass.context.deferred_policies, &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits, input, pass.role.clone(), &mut negative_trace);
        assert!(!errors.is_empty());
        assert_eq!(malformed, before);
        assert_eq!(*input, original_input);
        assert_eq!(negative_trace.passes.len(), 1);
        assert!(negative_trace.passes[0].observations.points.is_empty());
        assert!(negative_trace.passes[0].observations.borrows.is_empty());
    }
}
