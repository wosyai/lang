use super::*;

fn fixture(text: &str, layout: ScalarTargetLayout, project: bool) {
    let identity = SourceIdentity::new("c3".into(), "history-worker".into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (validation, construction) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    for pass in construction.passes.iter().filter(|pass| pass.stage == ReferenceTransferTestStage::InitialConstruction) {
        assert_eq!(pass.cfg_at_entry.source_capture_inventory.phase, SourceCaptureConstructionPhase::Building);
        source_boundary::validate_capture_preflight(&pass.cfg_at_entry, &pass.context.capture_input).expect("genuine Building pending input");
        for requirement in &pass.cfg_at_entry.source_capture_inventory.required {
            assert!(pass.cfg_at_entry.evaluated_values.source_capture_pending.iter().any(|pending| pending.event == requirement.event));
        }
    }
    let module = if project {
        let validated = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
        assert!(validated.diagnostics.is_empty(), "{:?}", validated.diagnostics);
        validated.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) };
    let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("actual run");
    let cfg = &function.reference_cfg;
    let forest = source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("retained original complete forest");
    let input = source_boundary::preflight_input(&cfg.producer_owner, &forest, BTreeMap::new()).expect("retained external actual input");
    let restored: ScalarReferenceCfg = serde_json::from_str(&serde_json::to_string(cfg).expect("serialize original CFG history")).expect("restore static history only");
    for history in [cfg.clone(), restored] {
        assert_eq!(history, *cfg);
        source_boundary::validate_capture_preflight(&history, &input).expect("original complete Clone/Serde input");
        let mut refreshed = history.clone();
        let mut trace = ReferenceValidationTestTrace::new();
        assert!(refresh_literal_cfgs(&mut refreshed, &[], &[], &input, &mut trace).is_empty(), "actual refresh retains original closure");
        assert_eq!(refreshed.source_capture_inventory, history.source_capture_inventory);
        assert_eq!(refreshed.initial_points, history.initial_points);
        assert_eq!(refreshed.evaluated_values.source_capture_productions, history.evaluated_values.source_capture_productions);
        source_boundary::validate_capture_preflight(&refreshed, &input).expect("current refreshed candidate against retained originals");
        assert_eq!(input.construction_forest, forest);
        for defect in 0..26 {
            let mut candidate = history.clone();
            let production = candidate.evaluated_values.source_capture_productions.first().expect("actual original Capture, including empty-input source").clone();
            if defect >= 15 && production.inputs.is_empty() { continue; }
            match defect {
                0 => candidate.evaluated_values.source_capture_productions.clear(),
                1 => candidate.evaluated_values.source_capture_productions.push(production.clone()),
                2 => candidate.source_capture_inventory.required.clear(),
                3 => { candidate.source_capture_inventory.required.clear(); candidate.evaluated_values.source_capture_productions.clear(); }
                4 => candidate.literal_cfgs.clear(),
                5 => candidate.evaluated_values.source_capture_productions[0].event.point = CfgPointId(usize::MAX),
                6 => candidate.evaluated_values.source_capture_productions[0].output.resolved.output += 1,
                7 => candidate.evaluated_values.source_capture_productions[0].signature = ScalarType::U32,
                8 => candidate.evaluated_values.source_capture_productions[0].child = cfg.producer_owner.clone(),
                9 => candidate.literal_cfgs[0].reference_cfg.evaluated_values.boundary_signatures.clear(),
                10 => candidate.evaluated_values.source_capture_productions[0].output.resolved.occurrence.range = ByteSpan::new(0, 0),
                11 => candidate.evaluated_values.source_capture_productions[0].output.resolved.output_projection.push(usize::MAX),
                12 => candidate.capture_points.clear(),
                13 => candidate.source_capture_inventory.required.push(candidate.source_capture_inventory.required[0].clone()),
                14 => candidate.points[production.event.point.0].kind = CfgPointKind::Entry,
                15 => candidate.evaluated_values.source_capture_productions[0].inputs.clear(),
                16 => candidate.evaluated_values.source_capture_productions[0].inputs.push(production.inputs[0].clone()),
                17 => candidate.evaluated_values.source_capture_productions[0].inputs[0].formal.port = usize::MAX,
                18 => candidate.evaluated_values.source_capture_productions[0].inputs[0].formal.owner = cfg.producer_owner.clone(),
                19 => candidate.evaluated_values.source_capture_productions[0].inputs[0].coordinate.index = usize::MAX,
                20 => candidate.evaluated_values.source_capture_productions[0].inputs[0].coordinate.owner = production.child.clone(),
                21 => candidate.evaluated_values.source_capture_productions[0].inputs[0].shape.index = usize::MAX,
                22 => candidate.evaluated_values.source_capture_productions[0].inputs[0].capture.ty = ScalarType::Error,
                23 => candidate.evaluated_values.source_capture_productions[0].inputs[0].policy = CopyPolicy::Deferred,
                24 => candidate.evaluated_values.source_capture_productions[0].inputs[0].capture.binding.declaration_span = ByteSpan::new(0, 0),
                25 => candidate.literal_cfgs[0].reference_cfg.evaluated_values.boundary_signatures[0].ports.clear(),
                _ => unreachable!(),
            }
            let expected = source_boundary::validate_capture_preflight(&candidate, &input).expect_err("independent retained-original guard rejects explicit history defect");
            assert!(matches!(&expected, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence } if source_span.source == module.source));
            let saved = (candidate.clone(), input.clone());
            let mut trace = ReferenceValidationTestTrace::new();
            let errors = transfer_reference_facts(&mut candidate, &BTreeMap::new(), &[], &[], &[], &input, ReferenceTransferTestRole::Function, &mut trace);
            assert_eq!(errors, vec![expected], "qualified worker priority defect={defect}");
            assert_eq!(trace.passes.len(), 1);
            assert!(trace.passes[0].observations.points.is_empty());
            assert!(trace.passes[0].observations.borrows.is_empty());
            assert_eq!((candidate.clone(), input.clone()), saved);
            let mut refresh_trace = ReferenceValidationTestTrace::new();
            assert!(!refresh_literal_cfgs(&mut candidate, &[], &[], &input, &mut refresh_trace).is_empty());
            assert_eq!((candidate, input.clone()), saved);
            assert!(refresh_trace.passes.is_empty());
        }
    }
}

fn matrix(layout: ScalarTargetLayout, project: bool) {
    for text in [
        "%%start\nunit() run = fn { unit() empty = fn { }; empty(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; unit() write = fn { item = 2; }; write(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; unit() outer = fn { unit() inner = fn { item = 2; }; inner(); }; outer(); };\n%%end",
        "%%start\nstruct copy Pair { u32 item; }\nunit() run = fn { Pair pair = { .item = 1; }; unit() read = fn { pair.item; }; read(); };\n%%end",
        "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *child = 2; }; write(); };\n%%end",
        "%%start\nunit(u64) run = fn(count) { u8[count] slots; u8() read = fn { slots[0] }; read(); };\n%%end",
        "%%start\nunit() run = fn { u8() first = fn { 1 }; u8() second = fn { first() }; second(); };\n%%end",
    ] { fixture(text, layout, project); }
}

#[test]
fn actual_history_worker_native64_program() { matrix(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn actual_history_worker_native64_project() { matrix(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn actual_history_worker_wasm32_program() { matrix(ScalarTargetLayout::WASM32, false); }
#[test]
fn actual_history_worker_wasm32_project() { matrix(ScalarTargetLayout::WASM32, true); }

#[test]
fn actual_selected_concrete_history_worker() {
    let text = "%%start\ngeneric T;\nunit(T) run = fn(item) { unit() read = fn { item; }; read(); };\nunit() invoke = fn { run<u32>(1); };\n%%end";
    for (layout, project) in [(ScalarTargetLayout::NATIVE64, false), (ScalarTargetLayout::NATIVE64, true), (ScalarTargetLayout::WASM32, false), (ScalarTargetLayout::WASM32, true)] {
        let identity = SourceIdentity::new("c3".into(), "concrete-history".into(), "main.w".into(), "r1".into());
        let parsed = crate::parse_source(identity.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let (validation, construction) = derive_scalar_program_observed_test(&parsed.result, layout);
        assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
        let trace = if project {
            let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![identity]));
            assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics); trace
        } else { construction };
        let pass = trace.passes.iter().find(|pass| matches!(pass.role, ReferenceTransferTestRole::SpecializedCopy { .. }) && !pass.cfg_at_entry.evaluated_values.source_capture_policy_instances.is_empty()).expect("actual selected concrete policy history");
        let input = &pass.context.capture_input;
        assert_eq!(input.substitutions.get("T"), Some(&ScalarType::U32));
        assert!(pass.cfg_at_entry.evaluated_values.source_capture_productions.iter().flat_map(|production| &production.inputs).any(|port| port.policy == CopyPolicy::Deferred));
        let restored: ScalarReferenceCfg = serde_json::from_str(&serde_json::to_string(&pass.cfg_at_entry).expect("serialize selected static policy history")).expect("restore selected history");
        for mut candidate in [pass.cfg_at_entry.clone(), restored] {
            assert_eq!(candidate, pass.cfg_at_entry);
            source_boundary::validate_capture_preflight(&candidate, input).expect("actual original selected concrete instance");
            for instance in &candidate.evaluated_values.source_capture_policy_instances { assert_eq!(instance.ty, ScalarType::U32); assert_eq!(instance.policy, CopyPolicy::Copy); }
            candidate.evaluated_values.source_capture_policy_instances.clear();
            let expected = source_boundary::validate_capture_preflight(&candidate, input).expect_err("required selected instance erasure");
            let saved = (candidate.clone(), input.clone());
            let mut trace = ReferenceValidationTestTrace::new();
            let errors = transfer_reference_facts(&mut candidate, &pass.context.deferred_policies, &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits, input, pass.role.clone(), &mut trace);
            assert_eq!(errors, vec![expected]);
            assert_eq!(trace.passes.len(), 1);
            assert!(trace.passes[0].observations.points.is_empty()); assert!(trace.passes[0].observations.borrows.is_empty());
            assert_eq!((candidate, input.clone()), saved);
        }
        for defect in 0..6 {
            let mut candidate = pass.cfg_at_entry.clone();
            let selected = candidate.evaluated_values.source_capture_policy_instances[0].clone();
            match defect {
                0 => candidate.evaluated_values.source_capture_policy_instances.push(selected.clone()),
                1 => candidate.evaluated_values.source_capture_policy_instances[0].event.point = CfgPointId(usize::MAX),
                2 => candidate.evaluated_values.source_capture_policy_instances[0].formal.port = usize::MAX,
                3 => candidate.evaluated_values.source_capture_policy_instances[0].ty = ScalarType::U8,
                4 => candidate.evaluated_values.source_capture_policy_instances[0].policy = CopyPolicy::Move,
                5 => candidate.evaluated_values.source_capture_policy_instances[0].substitutions.clear(),
                _ => unreachable!(),
            }
            let expected = source_boundary::validate_capture_preflight(&candidate, input).expect_err("actual selected concrete history field counterpart");
            let before = (candidate.clone(), input.clone());
            let mut trace = ReferenceValidationTestTrace::new();
            assert_eq!(transfer_reference_facts(&mut candidate, &pass.context.deferred_policies, &pass.context.contracts, &pass.context.literal_contracts, &pass.context.call_exits, input, pass.role.clone(), &mut trace), vec![expected]);
            assert_eq!(trace.passes.len(), 1);
            assert!(trace.passes[0].observations.points.is_empty());
            assert!(trace.passes[0].observations.borrows.is_empty());
            assert_eq!((candidate.clone(), input.clone()), before);
            let mut refresh_trace = ReferenceValidationTestTrace::new();
            assert!(!refresh_literal_cfgs(&mut candidate, &pass.context.contracts, &pass.context.literal_contracts, input, &mut refresh_trace).is_empty());
            assert_eq!((candidate, input.clone()), before);
            assert!(refresh_trace.passes.is_empty());
        }
    }
}
