use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nu32() inspect = fn { u32 value = 1; u32 replacement = 2; value = replacement };\n%%end";

fn program(layout: ScalarTargetLayout) -> ScalarProgram {
    let source = SourceIdentity::new("resolver-endpoint".into(), "app".into(), "src/main.w".into(), "r1".into());
    let parsed = parse_source(source, SOURCE.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    validation.program
}

fn original(root: &ScalarReferenceCfg) -> &SourceModuleResolverInputEvidence {
    let SourceModuleResolverInputState::OriginalModuleInvocation(evidence) = &root.module_resolver_input else {
        panic!("actual factory original invocation")
    };
    evidence
}

#[test]
fn fresh_program_project_and_repeated_project_preserve_prior_endpoints() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let program = program(layout);
        original(&program.top_level_reference_cfg).validate().expect("program original replay");
        let module = ScalarModule::from_program(program, Vec::new());
        let prior = module.clone();
        let context = SourceCapturePolicyContext {
            structs: module.structs.clone(), enums: module.enums.clone(),
            generic_scope: module.top_level_reference_cfg.producer_owner.clone(),
            generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(),
        };
        let resources = capture_table_sources(&module.top_level_reference_cfg, &context).expect("prior original resource endpoints");
        let saved_resources = resources.clone();
        let result = validate_scalar_project(ScalarProject::new(vec![module], vec![prior.source.clone()]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let current = &result.project.modules[0];
        let evidence = original(&current.top_level_reference_cfg);
        evidence.validate().expect("actual project input replay");
        assert_eq!(evidence.pre.source, prior.source);
        assert_eq!(evidence.pre.items, current.items.iter().map(ItemInput::freeze).collect::<Vec<_>>());
        assert_eq!(evidence.pre.modules.len(), 1);
        assert_eq!(evidence.pre.modules[0].namespace_bindings, prior.namespace_bindings);
        assert_eq!(current.target_layout, layout);
        assert_eq!(resources, saved_resources);
        assert_eq!(resources[0].resource_construction.original(), &prior.top_level_reference_cfg);
        let first_endpoint = current.top_level_reference_cfg.clone();
        let result = validate_scalar_project(result.project.clone());
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        original(&result.project.modules[0].top_level_reference_cfg).validate().expect("repeated factory replay");
        assert_eq!(first_endpoint, current.top_level_reference_cfg);
        assert_eq!(resources, saved_resources);
    }
}

#[test]
fn same_fresh_endpoint_second_install_is_atomic() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let program = program(layout);
        let module = ScalarModule::from_program(program, Vec::new());
        let modules = [module];
        let pre = freeze(&modules[0].source, &modules[0].items, &modules);
        let result = reference::resolve_module_call_sites(&modules[0].source, &modules[0].items, &modules);
        let evidence = finish(pre, &result, &modules);
        evidence.validate().expect("genuine invocation baseline");
        let (sites, captures) = result.expect("original canonical result");
        let mut root = ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(modules[0].source.clone()));
        evidence.clone().install(&mut root, &modules[0].source, &modules[0].items, &sites, &captures).expect("first fresh install");
        let saved = root.clone();
        let error = evidence.install(&mut root, &modules[0].source, &modules[0].items, &sites, &captures).expect_err("same endpoint duplicate");
        assert_eq!(error, failure(&modules[0].source));
        assert_eq!(root, saved);
    }
}

#[test]
fn isolated_repeated_serde_and_corrupt_epochs_leave_live_source_exact() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let module = ScalarModule::from_program(program(layout), Vec::new());
        let modules = [module];
        let saved = modules.clone();
        let pre = freeze(&modules[0].source, &modules[0].items, &modules);
        let result = reference::resolve_module_call_sites(&modules[0].source, &modules[0].items, &modules);
        let evidence = finish(pre, &result, &modules);
        let live_after_resolver = modules.clone();
        evidence.validate().expect("baseline isolated replay");
        evidence.validate().expect("independent repeated isolated replay");
        let serialized = serde_json::to_vec(&evidence).expect("typed original evidence serialization");
        let restored: SourceModuleResolverInputEvidence = serde_json::from_slice(&serialized).expect("independent original evidence deserialization");
        restored.validate().expect("independent Serde isolated replay");
        assert_eq!(restored, evidence);
        assert_eq!(modules, live_after_resolver);
        let mut bad_pre = evidence.clone();
        bad_pre.pre.modules.clear();
        assert_eq!(bad_pre.validate(), Err(failure(&modules[0].source)));
        let mut bad_post = evidence.clone();
        bad_post.post.clear();
        assert_eq!(bad_post.validate(), Err(failure(&modules[0].source)));
        let mut bad_result = evidence.clone();
        bad_result.result = Err(failure(&modules[0].source));
        assert_eq!(bad_result.validate(), Err(failure(&modules[0].source)));
        let (sites, captures) = result.expect("canonical invocation succeeded");
        let mut root = ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(modules[0].source.clone()));
        let before = root.clone();
        assert_eq!(bad_result.install(&mut root, &modules[0].source, &modules[0].items, &sites, &captures), Err(failure(&modules[0].source)));
        assert_eq!(root, before);
        assert_eq!(modules, live_after_resolver);
        assert_eq!(saved[0].top_level_reference_cfg, live_after_resolver[0].top_level_reference_cfg);
    }
}

#[test]
fn offside_program_clone_owns_independent_type_cells_and_pending_endpoint() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let live = program(layout);
        let saved = live.clone();
        let mut construction = live.clone();
        construction.top_level_reference_cfg = ScalarReferenceCfg::new(ProducerCfgOwnerId::Module(live.source.clone()));
        construction.resolved_literals.borrow_mut().clear();
        construction.resolved_assignment_types.borrow_mut().clear();
        construction.resolved_expression_outputs.borrow_mut().clear();
        construction.resolved_assignment_outputs.borrow_mut().clear();
        assert_eq!(construction.top_level_reference_cfg.module_resolver_input, SourceModuleResolverInputState::PendingModuleInvocation);
        assert_eq!(live, saved);
        original(&live.top_level_reference_cfg).validate().expect("prior endpoint remains original");
    }
}
