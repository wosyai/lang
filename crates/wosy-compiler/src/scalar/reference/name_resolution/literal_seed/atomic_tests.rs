use super::*;

fn original(layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    let source = SourceIdentity::new("uncaptured-arena".into(), "app".into(), "src/main.w".into(), "r1".into());
    let text = "%%start\ni32 first = 1;\n*i32 left = &first;\nunit() value = fn { 1; };\nvalue;\n%%end";
    let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let cfg = if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(single.program, Vec::new())], vec![source]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules.into_iter().next().expect("actual module source").top_level_reference_cfg
    } else { single.program.top_level_reference_cfg };
    let context = source_boundary::SourceCapturePolicyContext {
        structs: Vec::new(), enums: Vec::new(), generic_scope: cfg.producer_owner.clone(),
        generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(),
    };
    source_boundary::capture_table_sources(&cfg, &context).expect("genuine original seed/source baseline");
    cfg
}

#[test]
fn failed_literal_seed_publication_preserves_entire_parent_and_diagnostic_order() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let mut cfg = original(layout, project);
            let saved = cfg.clone();
            let mut children = cfg.literal_cfgs.clone();
            children[0].constructor_seed.id.event.point = CfgPointId(0);
            let errors = vec![
                ReferenceAnalysisError::MissingContext {
                    source_span: cfg.initial_points[0].source_span.clone(),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                },
                ReferenceAnalysisError::MissingContext {
                    source_span: children[0].literal.clone(),
                    reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                },
            ];
            let returned = publish(&mut cfg, children, errors.clone());
            assert_eq!(returned, errors);
            assert_eq!(cfg, saved);
        }
    }
}

#[test]
fn uncaptured_parent_definitions_are_copied_without_capture_ordinals() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(layout, project);
            let child = cfg.literal_cfgs.first().expect("actual original child");
            let seed = &child.constructor_seed;
            assert!(seed.captures.is_empty());
            assert!(seed.inputs.is_empty());
            assert!(!seed.initial_facts.is_empty());
            assert!(seed.inherited_facts.starts_with(&seed.initial_facts));
            assert!(child.reference_cfg.facts.starts_with(&seed.inherited_facts));
        }
    }
}
