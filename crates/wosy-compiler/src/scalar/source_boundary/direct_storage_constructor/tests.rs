use super::*;
use crate::scalar::reference::conditional_output_production;

pub(in crate::scalar) const SOURCES: [&str; 13] = [
    "%%start\nunit() shared_root = fn { u8 value = 1; *u8 view = &value; view; };\nunit() mutable_root = fn { u8 value = 2; *!u8 view = &!value; view; };\n%%end",
    "%%start\nunit(u8) scalar_parameter = fn(value) { *u8 view = &value; view; };\n%%end",
    "%%start\nstruct copy Packet { u8 value; }\nunit() shared_field = fn { Packet packet = { .value = 1; }; *u8 view = &packet.value; view; };\nunit() mutable_field = fn { Packet packet = { .value = 2; }; *!u8 view = &!packet.value; view; };\n%%end",
    "%%start\nunit() shared_index = fn { u8[2] bytes = [1, 2]; u64 index = 0; *u8 view = &bytes[index]; view; };\nunit() mutable_index = fn { u8[2] bytes = [1, 2]; u64 index = 0; *!u8 view = &!bytes[index]; view; };\n%%end",
    "%%start\nunit() outer = fn { u8 value = 1; unit() nested = fn { *u8 view = &value; view; }; nested(); };\n%%end",
    "%%start\nunit(*u8) mixed = fn(input) { input; u8 value = 1; *u8 view = &value; view; *u8 empty = null; empty; };\n%%end",
    "%%start\nunit() repeated = fn { u8 value = 1; *u8 left = &value; left; *u8 right = &value; right; };\n%%end",
    "%%start\nunit() through = fn { u8 value = 1; *u8 parent = &value; *u8 child = &(*parent); child; };\n%%end",
    "%%start\nunit(u64, *?u8) update = fn(count, raw) { u8[count] bytes; *u8 first = &bytes[0]; first; unsafe { core.invalidate(&bytes); core.rebind(&bytes, raw, count); } };\n%%end",
    "%%start\nstruct copy Holder { (*i32, *i32)() take; }\ni32 first = 1;\ni32 second = 2;\n*i32 left = &first;\n*i32 right = &second;\nHolder container = { .take = fn() { left, right }; };\ncontainer;\n%%end",
    "%%start\nunit(*u8, *u8) consume = fn(first, second) { first; second; };\nu64(*u8) index_of = fn(pointer) { pointer; 0 };\nunit() arguments = fn { u8 value = 1; u8[2] bytes = [1, 2]; consume(&value, &bytes[0]); *u8 view = &bytes[index_of(&value)]; view; };\n%%end",
    "%%start\nunit() block_index = fn { u8[2] bytes = [1, 2]; *u8 view = &bytes[{ u64 offset = 0; offset }]; view; };\n%%end",
    "%%start\nunit(bool) conditional_index = fn(choose) { u8[2] bytes = [1, 2]; *u8 view = &bytes[if (choose) { 0 } else { 1 }]; view; };\n%%end",
];

pub(in crate::scalar) fn factory(unit: &str, text: &str, layout: ScalarTargetLayout, project: bool) -> ScalarModule {
    let source = SourceIdentity::new("direct-original".into(), unit.into(), "main.w".into(), "r1".into());
    let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:?}", validation.diagnostics);
    if project {
        let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules[0].clone()
    } else { ScalarModule::from_program(validation.program, Vec::new()) }
}

pub(in crate::scalar) fn forest(module: &ScalarModule) -> SourceCaptureConstructionForest {
    source_boundary::module_preflight_forest(&module.source, &module.items, &module.top_level_reference_cfg, &module.structs, &module.enums, SourceCaptureConstructionPhase::Complete).expect("genuine complete independent original forest")
}

fn corrupt(cfg: &mut ScalarReferenceCfg, structs: &[ScalarStruct], defect: usize) {
    let row = expected(cfg, structs).expect("independent direct source baseline before corruption")[0].clone();
    match defect {
        0..=13 => {
            let ReferenceOrigin::Fresh { allocation, loan } = &mut cfg.facts[row.origin.origin.0] else { panic!("genuine Fresh direct fixture") };
            match defect {
                0 => allocation.source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                1 => allocation.function_span = ByteSpan::new(0, 0),
                2 => allocation.declaration_span = ByteSpan::new(0, 0),
                3 => allocation.creation_span = ByteSpan::new(0, 0),
                4 => loan.mode = if loan.mode == ScalarReferenceMutability::Shared { ScalarReferenceMutability::Mutable } else { ScalarReferenceMutability::Shared },
                5 => loan.parent = Some(Box::new(loan.clone())),
                6 => loan.creation_span = ByteSpan::new(0, 0),
                7 => loan.creation_source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                8 => loan.origin_place.function_span = ByteSpan::new(1, 1),
                9 => loan.origin_place.binding.declaration_span = ByteSpan::new(0, 0),
                10 => loan.origin_place.block_span = ByteSpan::new(1, 1),
                11 => loan.origin_place.projections.push(ReferencePlaceProjection::Index(ScalarExpression::Integer { value: 0.into(), span: ByteSpan::new(0, 0) })),
                12 => loan.origin_place.place = ScalarPlace::Name { name: "redirect".into(), span: ByteSpan::new(0, 0) },
                13 => loan.origin_place.source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                _ => unreachable!(),
            }
        },
        14 => cfg.initial_points[row.event.point.0].possible_origins = Some(BTreeSet::new()),
        15 => cfg.initial_points[row.event.point.0].id = CfgPointId(usize::MAX),
        16 => cfg.initial_points[row.event.point.0].scope = ReferenceScopeId(usize::MAX),
        17 => cfg.initial_points[row.event.point.0].source_span.range = ByteSpan::new(0, 0),
        18 => cfg.evaluated_values.source_origin_derivations.clear(),
        19 => { let extra = cfg.evaluated_values.source_origin_derivations[0].clone(); cfg.evaluated_values.source_origin_derivations.push(extra); },
        20..=22 => {
            let row = cfg.evaluated_values.source_origin_derivations.iter_mut().find_map(|row| match row { null_constructor::SourceConstructorOriginDerivation::DirectStorageAddress(row) => Some(row), null_constructor::SourceConstructorOriginDerivation::AllocationIdentityAddress(_) | null_constructor::SourceConstructorOriginDerivation::NullExpression(_) | null_constructor::SourceConstructorOriginDerivation::CheckedParameter(_) => None }).expect("actual direct candidate incidence");
            match defect {
                20 => row.origin.origin = ReferenceOriginId(usize::MAX),
                21 => row.event.point = CfgPointId(usize::MAX),
                22 => row.origin.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                _ => unreachable!(),
            }
        },
        23 => { let CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { origin, .. } } = &mut cfg.initial_points[row.event.point.0].kind else { panic!("direct fixture") }; *origin = ReferenceOriginId(usize::MAX); },
        24 => { let extra = cfg.facts[row.origin.origin.0].clone(); let id = ReferenceOriginId(cfg.facts.len()); cfg.facts.push(extra); let CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { origin, .. } } = &mut cfg.initial_points[row.event.point.0].kind else { panic!("direct fixture") }; *origin = id; cfg.initial_points[row.event.point.0].possible_origins = Some(BTreeSet::from([id])); },
        _ => unreachable!(),
    }
}

#[test]
fn direct_storage_thirteen_original_groups_full_formula_and_candidate_atomicity() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                for (group, source) in SOURCES.iter().enumerate() {
                    let module = factory(endpoint, source, layout, project);
                    let forest = forest(&module);
                    let total = forest.sources.iter().map(|row| expected(row.resource_construction.original(), row.resource_construction.structs()).expect("independent direct whole source admission").len()).sum::<usize>();
                    assert_eq!(total, [2, 1, 2, 2, 1, 1, 2, 1, 2, 2, 4, 1, 1][group]);
                    for source in &forest.sources {
                        let cfg = source.resource_construction.original();
                        let structs = source.resource_construction.structs();
                        reference::name_resolution::validate_null_constructors(cfg, structs).expect("all3families baseline before mutation");
                        let rows = expected(cfg, structs).expect("whole source required direct inventory");
                        assert!(rows.iter().all(|row| row.origin.owner == cfg.producer_owner));
                        if rows.is_empty() { continue; }
                        let encoded = serde_json::to_string(cfg).expect("original CFG serialized separately from immutable struct policy");
                        let restored = serde_json::from_str::<ScalarReferenceCfg>(&encoded).expect("candidate CFG decoded");
                        reference::name_resolution::validate_null_constructors(&restored, structs).expect("Serde candidate requires independently retained original struct policy");
                        assert_eq!(restored, *cfg);
                        for defect in 0..25 {
                            let mut candidate = cfg.clone();
                            corrupt(&mut candidate, structs, defect);
                            let saved = candidate.clone();
                            assert!(reference::name_resolution::validate_null_constructors(&candidate, structs).is_err());
                            assert_eq!(candidate, saved);
                            reference::name_resolution::validate_null_constructors(cfg, structs).expect("repeated independent original after defect");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn direct_storage_control_original_anchor_direction_and_defects() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                for text in &SOURCES[11..] {
                    let module = factory(endpoint, text, layout, project);
                    let original = forest(&module);
                    let source = original.sources.iter().find(|source| source.resource_construction.original().evaluated_values.productions.iter().any(|row| matches!(row.source, OutputProductionSource::Block { .. } | OutputProductionSource::Conditional { .. }))).expect("actual original control production");
                    let cfg = source.resource_construction.original();
                    let ordinal = cfg.evaluated_values.productions.iter().position(|row| matches!(row.source, OutputProductionSource::Block { .. } | OutputProductionSource::Conditional { .. })).expect("genuine indexed control");
                    let production = &cfg.evaluated_values.productions[ordinal];
                    conditional_output_production::validate_original_control_anchor(cfg, production).expect("actual INITIAL canonical anchor baseline");
                    if let OutputProductionSource::Block { scope_exit, .. } = &production.source {
                        assert!(scope_exit.0 < production.evaluation_point.0);
                        assert!(cfg.initial_points[scope_exit.0].successors.contains(&production.evaluation_point));
                    }
                    for defect in 0..8 {
                        let mut candidate = cfg.clone();
                        let row = &mut candidate.evaluated_values.productions[ordinal];
                        match defect {
                            0 => row.evaluation_point = CfgPointId(usize::MAX),
                            1 => row.occurrence.range = ByteSpan::new(0, 0),
                            2 => row.output_projection.push(usize::MAX),
                            3 => candidate.initial_points[row.evaluation_point.0].id = CfgPointId(usize::MAX),
                            4 => candidate.initial_points[row.evaluation_point.0].kind = CfgPointKind::ScopeExit,
                            5..=7 => match &mut row.source {
                                OutputProductionSource::Block { scope_exit, inputs, .. } => match defect {
                                    5 => *scope_exit = row.evaluation_point,
                                    6 => candidate.initial_points[scope_exit.0].successors.clear(),
                                    7 => inputs[0].point = CfgPointId(usize::MAX),
                                    _ => unreachable!(),
                                },
                                OutputProductionSource::Conditional { branch, inputs } => match defect {
                                    5 => *branch = row.evaluation_point,
                                    6 => inputs[0].predecessor = inputs[1].predecessor,
                                    7 => inputs[0].outputs[0].point = CfgPointId(usize::MAX),
                                    _ => unreachable!(),
                                },
                                _ => unreachable!(),
                            },
                            _ => unreachable!(),
                        }
                        let saved = candidate.clone();
                        assert!(conditional_output_production::validate_original_control_anchor(&candidate, &candidate.evaluated_values.productions[ordinal]).is_err());
                        assert_eq!(candidate, saved);
                        assert_eq!(cfg, source.resource_construction.original());
                    }
                }
            }
        }
    }
}

#[test]
fn direct_storage_original_visible_shadow_b0002_remains_rejected() {
    let text = "%%start\nunit() run = fn { u32 item = 1; *u32 selected = &item; unsafe { u32 inner = 2; *u32 selected = &inner; *selected; }; *selected; };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let source = SourceIdentity::new("direct-shadow".into(), "negative".into(), "main.w".into(), "r1".into());
            let parsed = crate::parse_source(source.clone(), text.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let validation = derive_scalar_program_with_layout(&parsed.result, layout);
            let (module, diagnostics) = if project {
                let result = validate_scalar_project(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source.clone()]));
                (result.project.modules[0].clone(), result.diagnostics)
            } else { (ScalarModule::from_program(validation.program, Vec::new()), validation.diagnostics) };
            assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "B0002" && diagnostic.labels.iter().any(|label| label.kind == crate::DiagnosticLabelKind::Primary && label.span == SourceSpan::new(source.clone(), ByteSpan::new(87, 110)))));
            let function = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) => Some(function), _ => None }).expect("original rejected named source");
            assert!(function.reference_cfg.initial_points.is_empty());
            assert!(function.reference_cfg.points.is_empty());
            assert!(function.reference_cfg.checked_bindings.is_empty());
            assert!(function.reference_cfg.checked_installs.is_empty());
            assert!(function.reference_cfg.borrow_evaluations.is_empty());
        }
    }
}

#[test]
fn direct_storage_original_index_requests_and_nominal_policy_require_true_declarations() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                for group in [2, 3, 8, 10, 11, 12] {
                    let module = factory(endpoint, SOURCES[group], layout, project);
                    let original = forest(&module);
                    for source in &original.sources {
                        let cfg = source.resource_construction.original();
                        let structs = source.resource_construction.structs();
                        let rows = expected(cfg, structs).expect("true source index/field baseline");
                        for row in rows {
                            let point = &cfg.initial_points[row.event.point.0];
                            let CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { indices, place, .. } } = &point.kind else { panic!("genuine direct source fixture") };
                            if place.projections.iter().any(|projection| matches!(projection, ReferencePlaceProjection::Field(_))) {
                                for defect in 0..3 {
                                    let mut candidate = structs.to_vec();
                                    match defect {
                                        0 => candidate.clear(),
                                        1 => { let extra = candidate[0].clone(); candidate.push(extra); },
                                        2 => candidate[0].fields.clear(),
                                        _ => unreachable!(),
                                    }
                                    let before = candidate.clone();
                                    assert!(expected(cfg, &candidate).is_err());
                                    assert_eq!(candidate, before);
                                }
                            }
                            if indices.is_empty() { continue; }
                            for defect in 0..5 {
                                let mut candidate = cfg.clone();
                                let CfgPointKind::Borrow { request: ReferenceBorrowRequest::DirectStorage { indices, .. } } = &mut candidate.initial_points[row.event.point.0].kind else { panic!("genuine direct source fixture") };
                                match defect {
                                    0 => indices.clear(),
                                    1 => { let extra = indices[0].clone(); indices.push(extra); },
                                    2 => indices[0].projection = usize::MAX,
                                    3 => indices[0].value_point = row.event.point,
                                    4 => {
                                        let value_point = indices[0].value_point;
                                        candidate.initial_points[value_point.0].successors.clear();
                                    },
                                    _ => unreachable!(),
                                }
                                let before = candidate.clone();
                                assert!(expected(&candidate, structs).is_err());
                                assert_eq!(candidate, before);
                                assert_eq!(cfg, source.resource_construction.original());
                            }
                        }
                    }
                }
            }
        }
    }
}
