use super::*;

fn original(captured: bool, layout: ScalarTargetLayout, project: bool) -> ScalarReferenceCfg {
    let source = SourceIdentity::new("parentseed-domain".into(), "app".into(), "src/main.w".into(), "r1".into());
    let text = if captured {
        "%%start\nstruct copy Holder { (*i32, *i32)() take; }\ni32 first = 1;\ni32 second = 2;\n*i32 left = &first;\n*i32 right = &second;\nHolder container = { .take = fn() { left, right }; };\ncontainer;\n%%end"
    } else {
        "%%start\ni32 first = 1;\n*i32 left = &first;\nunit() value = fn { 1; };\nvalue;\n%%end"
    };
    let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let cfg = if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(single.program, Vec::new())], vec![source]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules.into_iter().next().expect("actual original module").top_level_reference_cfg
    } else { single.program.top_level_reference_cfg };
    let child = cfg.literal_cfgs.first().expect("actual original constructor");
    validate_original(&cfg, child).expect("genuine original whole-arena/use baseline");
    cfg
}

#[test]
fn every_uncaptured_inherited_definition_has_one_exact_empty_use_parentseed() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(false, layout, project);
            let seed = &cfg.literal_cfgs[0].constructor_seed;
            assert!(!seed.inherited_facts.is_empty());
            assert_eq!(seed.parent_seed_origins.len(), seed.inherited_facts.len());
            for (index, row) in seed.parent_seed_origins.iter().enumerate() {
                assert_eq!(row.parent.owner, cfg.producer_owner);
                assert_eq!(row.child.owner, seed.id.child);
                assert_eq!(row.parent.origin, ReferenceOriginId(index));
                assert_eq!(row.child.origin, row.parent.origin);
                assert_eq!(row.definition, seed.inherited_facts[index]);
                assert!(row.capture_uses.is_empty());
            }
        }
    }
}

#[test]
fn whole_inheritance_and_fabricated_uncaptured_use_defects_are_atomic() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(false, layout, project);
            let literal = &cfg.literal_cfgs[0];
            let mut missing = literal.clone();
            missing.constructor_seed.parent_seed_origins.clear();
            let saved = missing.clone();
            assert!(validate_original(&cfg, &missing).is_err());
            assert_eq!(missing, saved);
            let mut fabricated = literal.clone();
            let row = &mut fabricated.constructor_seed.parent_seed_origins[0];
            let ReferenceOrigin::Fresh { loan, .. } = &row.definition else { panic!("genuine direct-address creator") };
            row.capture_uses.push(SourceParentSeedCaptureUse {
                capture_ordinal: 0, binding: loan.origin_place.binding.clone(), input_ordinal: 0, origin: row.parent.clone(),
            });
            let saved = fabricated.clone();
            assert!(validate_original(&cfg, &fabricated).is_err());
            assert_eq!(fabricated, saved);
        }
    }
}

#[test]
fn captured_uses_retain_exact_original_declaration_input_and_origin_associations() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let cfg = original(true, layout, project);
            let literal = &cfg.literal_cfgs[0];
            let seed = &literal.constructor_seed;
            let index = seed.parent_seed_origins.iter().position(|row| !row.capture_uses.is_empty()).expect("actual inherited capture use");
            for row in &seed.parent_seed_origins {
                for usage in &row.capture_uses {
                    assert_eq!(usage.binding, seed.captures[usage.capture_ordinal].binding);
                    assert_eq!(usage.binding, seed.inputs[usage.input_ordinal].binding);
                    assert_eq!(usage.origin, row.parent);
                    assert!(seed.inputs[usage.input_ordinal].origins.contains(&row.parent.origin));
                }
            }
            let mut missing = literal.clone();
            missing.constructor_seed.parent_seed_origins[index].capture_uses.clear();
            let saved = missing.clone();
            assert!(validate_original(&cfg, &missing).is_err());
            assert_eq!(missing, saved);
            let mut redirected = literal.clone();
            redirected.constructor_seed.parent_seed_origins[index].capture_uses[0].input_ordinal = seed.inputs.len();
            let saved = redirected.clone();
            assert!(validate_original(&cfg, &redirected).is_err());
            assert_eq!(redirected, saved);
        }
    }
}
