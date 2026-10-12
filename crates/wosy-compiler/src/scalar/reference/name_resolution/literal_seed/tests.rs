use super::*;

fn original(text: &str, layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    let identity = SourceIdentity::new("original-seed".into(), "app".into(), "src/main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(single.program, Vec::new())], vec![identity]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules.into_iter().next().expect("actual original module").top_level_reference_cfg
    } else { single.program.top_level_reference_cfg }
}

fn context(cfg: &ScalarReferenceCfg) -> source_boundary::SourceCapturePolicyContext {
    source_boundary::SourceCapturePolicyContext {
        structs: Vec::new(), enums: Vec::new(), generic_scope: cfg.producer_owner.clone(),
        generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(),
    }
}

#[test]
fn original_constructor_receipt_retains_two_actual_nonempty_capture_inputs() {
    let text = "%%start\nstruct copy Holder { (*i32, *i32)() take; }\ni32 first = 1;\ni32 second = 2;\n*i32 left = &first;\n*i32 right = &second;\nHolder container = { .take = fn() { left, right }; };\ncontainer;\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(text, layout, project);
            source_boundary::capture_table_sources(&cfg, &context(&cfg)).expect("original source extraction baseline");
            let child = cfg.literal_cfgs.first().expect("actual constructed original child");
            let seed = &child.constructor_seed;
            assert_eq!(seed.id.event.owner, cfg.producer_owner);
            assert_eq!(seed.id.event.point, child.capture_point);
            assert_eq!(seed.id.child, child.reference_cfg.producer_owner);
            assert_eq!(seed.captures, child.captures);
            assert!(matches!(seed.parent_source, source_boundary::SourceLiteralSeedParentSource::Module(_)));
            assert!(seed.inherited_facts.starts_with(&seed.initial_facts));
            for name in ["left", "right"] {
                let capture = seed.captures.iter().find(|capture| capture.name == name).expect("genuine original capture declaration");
                let input = seed.inputs.iter().find(|input| input.binding == capture.binding).expect("actual consumed input");
                assert!(!input.origins.is_empty());
                for id in &input.origins { assert!(seed.inherited_facts.get(id.0).is_some()); }
            }
            assert_eq!(cfg.evaluated_values.source_literal_constructor_seeds, vec![seed.id.clone()]);
        }
    }
}

#[test]
fn original_empty_capture_receipt_is_explicit_and_required_source_refs_are_atomic() {
    let text = "%%start\nunit() value = fn { 1; };\nvalue;\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(text, layout, project);
            let context = context(&cfg);
            source_boundary::capture_table_sources(&cfg, &context).expect("original empty-capture source baseline");
            let seed = &cfg.literal_cfgs.first().expect("actual empty capture child").constructor_seed;
            assert!(seed.captures.is_empty());
            assert!(seed.inputs.is_empty());
            let mut missing = cfg.clone();
            missing.evaluated_values.source_literal_constructor_seeds.clear();
            let before = missing.clone();
            assert!(source_boundary::capture_table_sources(&missing, &context).is_err());
            assert_eq!(missing, before);
            let mut redirected = cfg.clone();
            redirected.literal_cfgs[0].constructor_seed.id.event.point = CfgPointId(0);
            let before = redirected.clone();
            assert!(source_boundary::capture_table_sources(&redirected, &context).is_err());
            assert_eq!(redirected, before);
        }
    }
}
