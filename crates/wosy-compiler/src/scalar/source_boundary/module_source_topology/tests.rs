use super::*;
use crate::parse_source;

const EMPTY: &str = "%%start\nu32() inspect = fn { 1 };\n%%end";
const NESTED: &str = "%%start\nstruct copy Reader { unit() read; }\nReader value = { .read = fn { unit() inner = fn { 1; }; inner(); }; };\nvalue.read();\n%%end";

fn original(text: &str, layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    let source = SourceIdentity::new("module-topology".into(), "app".into(), "src/main.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    if project {
        let result = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(single.program, Vec::new())], vec![source]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules.into_iter().next().expect("actual module endpoint").top_level_reference_cfg
    } else {
        single.program.top_level_reference_cfg
    }
}

fn augmentation(cfg: &ScalarReferenceCfg) -> &SourceModuleTopologyAugmentation {
    let SourceModuleTopologyState::AugmentedOriginal(augmented) = &evidence(cfg).expect("actual original invocation").topology else {
        panic!("actual original augmented stage")
    };
    augmented
}

#[test]
fn whole_connected_prefix_nested_suffix_and_canonical_root_are_exact() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(NESTED, layout, project);
            validate_published(&cfg).expect("published original baseline");
            let augmented = augmentation(&cfg);
            assert!(!augmented.suffix.is_empty(), "actual nested supplemental inventory");
            assert_eq!(cfg.initial_points[..augmented.connected.points.len()], augmented.connected.points);
            assert_eq!(cfg.name_resolution_context, augmented.connected.canonical_root);
            assert_eq!(*cfg.top_level, augmented.final_top_level);
            assert_eq!(cfg.initial_points, augmented.final_points);
            for event in &augmented.suffix {
                assert!(event.successors.is_empty());
                assert!(cfg.top_level.points.iter().any(|row| row.point == event.id
                    && row.execution == ScalarTopLevelPointExecution::SupplementalAnalysisOnly));
                assert!(cfg.holder_edges.iter().all(|row| row.from != event.id && row.to != event.id));
                assert!(cfg.holder_exits.iter().all(|row| row.from != event.id));
            }
            let saved = cfg.clone();
            let mut missing = cfg.clone();
            let SourceModuleResolverInputState::OriginalModuleInvocation(input) = &mut missing.module_resolver_input else { panic!("module input") };
            let SourceModuleTopologyState::AugmentedOriginal(actual) = &mut input.topology else { panic!("augmented stage") };
            actual.suffix.clear();
            assert!(validate_published(&missing).is_err());
            assert_eq!(cfg, saved);
            let mut wrong_root = cfg.clone();
            let NameResolutionContext::Complete(context) = &mut wrong_root.name_resolution_context else { panic!("canonical root") };
            let SourceNameResolutionRoot::Module { top_level, .. } = &mut context.root else { panic!("module lexical root") };
            *top_level = *cfg.top_level.clone();
            assert!(validate_published(&wrong_root).is_err());
            assert_eq!(cfg, saved);
        }
    }
}

#[test]
fn empty_suffix_still_requires_augmented_publication_and_serde_replay() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(EMPTY, layout, project);
            validate_published(&cfg).expect("empty suffix original baseline");
            let augmented = augmentation(&cfg);
            assert!(augmented.suffix.is_empty());
            assert!(augmented.suffix_environments.is_empty());
            let serialized = serde_json::to_vec(&cfg).expect("whole independent original serialization");
            let restored: ScalarReferenceCfg = serde_json::from_slice(&serialized).expect("independent original endpoint");
            validate_published(&restored).expect("independent Serde replay");
            validate_published(&restored).expect("repeated independent replay");
            assert_eq!(restored, cfg);
            let saved = cfg.clone();
            let mut downgraded = cfg.clone();
            let SourceModuleResolverInputState::OriginalModuleInvocation(input) = &mut downgraded.module_resolver_input else { panic!("original invocation") };
            input.topology = SourceModuleTopologyState::ConnectedConstruction(Box::new(augmented.connected.clone()));
            assert_eq!(validate_published(&downgraded), Err(ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone())));
            assert_eq!(cfg, saved);
        }
    }
}

#[test]
fn whole_placed_inventory_removal_rejects_nested_source_atomically() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(NESTED, layout, project);
            validate_published(&cfg).expect("placed capture source baseline");
            assert!(!augmentation(&cfg).connected.placed.is_empty());
            let saved = cfg.clone();
            let mut removed = cfg.clone();
            let SourceModuleResolverInputState::OriginalModuleInvocation(input) = &mut removed.module_resolver_input else { panic!("original input") };
            let SourceModuleTopologyState::AugmentedOriginal(actual) = &mut input.topology else { panic!("augmented original") };
            actual.connected.placed.clear();
            assert!(validate_published(&removed).is_err());
            assert_eq!(cfg, saved);
        }
    }
}
