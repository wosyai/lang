use super::*;
use crate::parse_source;

fn original(layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    let source = SourceIdentity::new("stage-consumer".into(), "app".into(), "src/main.w".into(), "r1".into());
    let text = "%%start\nstruct copy Reader { unit() read; }\nReader value = { .read = fn { unit() inner = fn { 1; }; inner(); }; };\nvalue.read();\n%%end";
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(single.program, Vec::new())], vec![source]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules.into_iter().next().expect("actual module original").top_level_reference_cfg
    } else { single.program.top_level_reference_cfg }
}

#[test]
fn genuine_connected_capture_resolves_through_whole_augmented_original() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(layout, project);
            validate_published(&cfg).expect("whole augmented baseline");
            let saved = cfg.clone();
            let (stage, connected) = constructor_source(&cfg).expect("genuine retained connected source");
            assert_eq!(stage.stage, SourceModuleOriginalStage::ConnectedConstruction);
            assert_eq!(stage.invocation.owner, cfg.producer_owner);
            let point = connected.points.iter().find(|point| matches!(point.kind, CfgPointKind::Capture { .. })).expect("actual productive connected Capture");
            let event = SourceEventId { owner: cfg.producer_owner.clone(), point: point.id };
            let (resolved_stage, resolved) = constructor_capture(&cfg, &event).expect("connected constructor source endpoint");
            assert_eq!(resolved_stage, stage);
            assert_eq!(resolved, point);
            let SourceModuleTopologyState::AugmentedOriginal(augmented) = &evidence(&cfg).expect("module input").topology else { panic!("augmented original") };
            let supplemental = augmented.suffix.iter().find(|point| matches!(point.kind, CfgPointKind::Capture { .. })).expect("actual nested supplemental Capture");
            let bad = SourceEventId { owner: cfg.producer_owner.clone(), point: supplemental.id };
            assert!(constructor_capture(&cfg, &bad).is_err());
            let bad = SourceEventId { owner: ProducerCfgOwnerId::Module(SourceIdentity::new("other".into(), "app".into(), "src/main.w".into(), "r1".into())), point: point.id };
            assert!(constructor_capture(&cfg, &bad).is_err());
            assert_eq!(cfg, saved);
        }
    }
}

#[test]
fn published_observation_cannot_be_consumed_as_initial_constructor_parent() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(layout, project);
            validate_published(&cfg).expect("authentic original baseline");
            let saved = cfg.clone();
            assert!(constructor_handoff(&cfg, &cfg).is_err());
            assert_eq!(cfg, saved);
        }
    }
}
