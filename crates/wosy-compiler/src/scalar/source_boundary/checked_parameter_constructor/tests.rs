use super::*;

pub(in crate::scalar) fn factory(unit: &str, layout: ScalarTargetLayout, project: bool) -> ScalarModule {
    let source = SourceIdentity::new("checked-parameter".into(), unit.into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(source.clone(), "%%start\nunit(*u8, *!u8, u8) run = fn(shared, mutable, value) { shared; mutable; value; null; unit(*u8) nested = fn(local) { local; null; }; nested; };\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) }
}

fn corrupt(cfg: &mut ScalarReferenceCfg, defect: usize) {
    let parameter = expected(cfg).expect("genuine checked parameter baseline before mutation")[0].clone();
    let install = cfg.checked_installs.iter().position(|row| row.point == parameter.event.point).expect("genuine original Parameter install");
    match defect {
        0 => cfg.checked_installs.clear(),
        1 => { let extra = cfg.checked_installs[install].clone(); cfg.checked_installs.push(extra); },
        2 => { let ReferenceCheckedInstallInput::Parameter { origin, .. } = &mut cfg.checked_installs[install].input else { panic!("Parameter fixture") }; *origin = ReferenceOriginId(usize::MAX); },
        3 => {
            let extra = cfg.facts[parameter.origin.origin.0].clone();
            let id = ReferenceOriginId(cfg.facts.len());
            cfg.facts.push(extra);
            let ReferenceCheckedInstallInput::Parameter { origin, .. } = &mut cfg.checked_installs[install].input else { panic!("Parameter fixture") };
            *origin = id;
            cfg.initial_points[parameter.event.point.0].possible_origins = Some(BTreeSet::from([id]));
        },
        4 => { let ReferenceCheckedInstallInput::Parameter { binding, .. } = &mut cfg.checked_installs[install].input else { panic!("Parameter fixture") }; binding.declaration_span = ByteSpan::new(0, 0); },
        5..=17 => {
            let ReferenceOrigin::BorrowedFrom { parameter, loan } = &mut cfg.facts[parameter.origin.origin.0] else { panic!("BorrowedFrom fixture") };
            match defect {
                5 => loan.origin_place.place = ScalarPlace::Name { name: "other".into(), span: loan.creation_span },
                6 => loan.mode = ScalarReferenceMutability::Mutable,
                7 => loan.parent = Some(Box::new(loan.clone())),
                8 => loan.origin_place.projections.push(ReferencePlaceProjection::Dereference(Box::new(loan.clone()))),
                9 => loan.creation_source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                10 => loan.creation_span = ByteSpan::new(0, 0),
                11 => loan.origin_place.source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                12 => loan.origin_place.function_span = ByteSpan::new(0, 0),
                13 => loan.origin_place.declaration_span = ByteSpan::new(0, 0),
                14 => loan.origin_place.block_span = ByteSpan::new(0, 0),
                15 => loan.origin_place.binding.kind = ReferenceBindingKind::Declared,
                16 => *parameter = usize::MAX,
                17 => loan.origin_place.place = ScalarPlace::Name { name: "shared".into(), span: ByteSpan::new(0, 0) },
                _ => unreachable!(),
            }
        },
        18 => cfg.initial_points[parameter.event.point.0].possible_origins = Some(BTreeSet::new()),
        19 => cfg.initial_points[parameter.event.point.0].scope = ReferenceScopeId(usize::MAX),
        20 => cfg.evaluated_values.source_origin_derivations.reverse(),
        21 => cfg.evaluated_values.source_origin_derivations.clear(),
        22 => { let extra = cfg.evaluated_values.source_origin_derivations[0].clone(); cfg.evaluated_values.source_origin_derivations.push(extra); },
        23 => {
            let NameResolutionContext::Complete(context) = &mut cfg.name_resolution_context else { panic!("Complete fixture") };
            match &mut context.root {
                SourceNameResolutionRoot::Named(root) => root.parameters[0] = "other".into(),
                SourceNameResolutionRoot::Literal { parameters, .. } => parameters[0] = "other".into(),
                SourceNameResolutionRoot::Module { .. } => unreachable!(),
            }
        },
        24 => {
            let NameResolutionContext::Complete(context) = &mut cfg.name_resolution_context else { panic!("Complete fixture") };
            let signature = match &mut context.root {
                SourceNameResolutionRoot::Named(root) => &mut root.signature,
                SourceNameResolutionRoot::Literal { signature, .. } => signature,
                SourceNameResolutionRoot::Module { .. } => unreachable!(),
            };
            let ScalarType::Callable { parameters, .. } = signature else { panic!("Callable fixture") };
            parameters[0] = ScalarType::U8;
        },
        _ => unreachable!(),
    }
}

#[test]
fn checked_parameter_original_full_formula_mixed_inventory_and_inherited_separation() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                let module = factory(endpoint, layout, project);
                assert!(expected(&module.top_level_reference_cfg).expect("module has zero parameter creators").is_empty());
                let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None }).expect("genuine named constructor");
                let parent = &function.reference_cfg;
                assert_eq!(expected(parent).expect("shared and mutable source baseline").len(), 2);
                assert_eq!(parent.reference_parameters.len(), 3);
                let child = parent.literal_cfgs[0].reference_cfg.as_ref();
                assert_eq!(expected(child).expect("literal local checked parameter baseline").len(), 1);
                assert_eq!(&child.facts[..parent.facts.len()], parent.facts.as_slice());
                assert!(parent.literal_cfgs[0].constructor_seed.parent_seed_origins.iter().any(|row| row.capture_uses.is_empty()));
                for cfg in [parent, child] {
                    reference::name_resolution::validate_null_constructors(cfg, &module.structs).expect("independent mixed source baseline");
                    let vector = &cfg.evaluated_values.source_origin_derivations;
                    assert!(vector.windows(2).all(|pair| pair[0].event().point < pair[1].event().point));
                    assert!(vector.iter().all(|row| row.origin().owner == cfg.producer_owner));
                    let encoded = serde_json::to_string(cfg).expect("whole immutable original CFG serializes");
                    let restored = serde_json::from_str::<ScalarReferenceCfg>(&encoded).expect("whole original CFG decodes");
                    assert_eq!(restored, *cfg);
                    reference::name_resolution::validate_null_constructors(&restored, &module.structs).expect("Serde independent source replay");
                    for defect in 0..25 {
                        let mut candidate = cfg.clone();
                        corrupt(&mut candidate, defect);
                        let saved = candidate.clone();
                        assert!(reference::name_resolution::validate_null_constructors(&candidate, &module.structs).is_err());
                        assert_eq!(candidate, saved);
                        reference::name_resolution::validate_null_constructors(cfg, &module.structs).expect("repeat unchanged original baseline after defect");
                    }
                }
            }
        }
    }
}
