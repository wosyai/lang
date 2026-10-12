use super::*;
pub(in crate::scalar) use super::super::direct_storage_constructor::tests::{factory, forest};

pub(in crate::scalar) const SOURCES: [&str; 5] = [
    "%%start\nunit(u64, *?u8) update = fn(count, raw) { u8[count] bytes; *u8 first = &bytes[0]; first; unsafe { core.invalidate(&bytes); core.rebind(&bytes, raw, count); } };\n%%end",
    "%%start\nunit(u64, *?u8) update = fn(count, raw) { u8[count] bytes; u8[count] other; unsafe { core.invalidate(&bytes); core.rebind(&bytes, raw, count); }; bytes[0]; other[0]; };\n%%end",
    "%%start\nu64 count = 2; *?u8 raw = null; u8[count] bytes; unsafe { core.rebind(&bytes, raw, count); }; bytes[0];\n%%end",
    "%%start\nunit(u64, *?u8) outer = fn(count, raw) { u8[count] bytes; unit() nested = fn { unsafe { core.rebind(&bytes, raw, count); }; bytes[0]; }; nested(); };\n%%end",
    "%%start\nunit(u64, *?u8) nested_reservation = fn(count, raw) { u8[count] bytes; u8[count] other; unsafe { core.rebind(&bytes[{ core.rebind(&other, raw, count); u64 offset = 0; offset }], raw, count); }; bytes[0]; other[0]; };\n%%end",
];

pub(in crate::scalar) fn corrupt(cfg: &mut ScalarReferenceCfg, structs: &[ScalarStruct], defect: usize) {
    let row = expected(cfg, structs).expect("genuine independently derived original before mutation")[0].clone();
    let ordinal = row.association.association.0;
    let association = cfg.rebind_associations[ordinal].clone();
    match defect {
        0 => cfg.rebind_associations.clear(),
        1 => { let extra = cfg.rebind_associations[ordinal].clone(); cfg.rebind_associations.push(extra); },
        2 => cfg.rebind_associations[ordinal].owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
        3 => cfg.rebind_associations[ordinal].occurrence.source_span.range = ByteSpan::new(0, 0),
        4 => cfg.rebind_associations[ordinal].address_point = association.effect_point,
        5 => cfg.rebind_associations[ordinal].raw_point = association.length_point,
        6 => cfg.rebind_associations[ordinal].length_point = association.raw_point,
        7 => cfg.rebind_associations[ordinal].effect_point = association.call_point,
        8 => cfg.rebind_associations[ordinal].call_point = association.effect_point,
        9 => cfg.initial_points[row.event.point.0].possible_origins = Some(BTreeSet::from([row.origin.origin])),
        10 => cfg.initial_points[row.event.point.0].id = CfgPointId(usize::MAX),
        11 => cfg.initial_points[row.event.point.0].scope = ReferenceScopeId(usize::MAX),
        12 => cfg.initial_points[row.event.point.0].source_span.range = ByteSpan::new(0, 0),
        13 => cfg.initial_points[association.raw_point.0].successors.clear(),
        14 => cfg.initial_points[association.length_point.0].successors.clear(),
        15 => cfg.initial_points[association.effect_point.0].successors.clear(),
        16 => cfg.initial_points[association.raw_point.0].source_span.range = ByteSpan::new(0, 0),
        17 => cfg.initial_points[association.length_point.0].source_span.range = ByteSpan::new(0, 0),
        18 => cfg.initial_points[association.call_point.0].kind = CfgPointKind::ScopeExit,
        19 => cfg.allocations.clear(),
        20 => cfg.name_resolution_context = NameResolutionContext::Building,
        21..=25 => {
            let CfgPointKind::AllocationIdentityAddress { request } = &mut cfg.initial_points[row.event.point.0].kind else { panic!("genuine allocation identity fixture") };
            match defect {
                21 => request.association = ReferenceRebindAssociationId(usize::MAX),
                22 => request.candidate.origin = ReferenceOriginId(usize::MAX),
                23 => request.candidate.place.binding.declaration_span = ByteSpan::new(0, 0),
                24 => request.candidate.loan.creation_span = ByteSpan::new(0, 0),
                25 => request.candidate.place.projections.clear(),
                _ => unreachable!(),
            }
            if defect == 25 { request.candidate.place.projections.push(ReferencePlaceProjection::Field(ScalarFieldReference::Unresolved { name: "redirect".into(), span: ByteSpan::new(0, 0) })); }
        },
        26..=38 => {
            let ReferenceOrigin::Fresh { allocation, loan } = &mut cfg.facts[row.origin.origin.0] else { panic!("genuine Fresh recipe") };
            match defect {
                26 => allocation.source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                27 => allocation.function_span = ByteSpan::new(0, 0),
                28 => allocation.declaration_span = ByteSpan::new(0, 0),
                29 => allocation.creation_span = ByteSpan::new(0, 0),
                30 => loan.mode = if loan.mode == ScalarReferenceMutability::Shared { ScalarReferenceMutability::Mutable } else { ScalarReferenceMutability::Shared },
                31 => loan.parent = Some(Box::new(loan.clone())),
                32 => loan.creation_source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                33 => loan.creation_span = ByteSpan::new(0, 0),
                34 => loan.origin_place.source = SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into()),
                35 => loan.origin_place.function_span = ByteSpan::new(1, 1),
                36 => loan.origin_place.binding.declaration_span = ByteSpan::new(0, 0),
                37 => loan.origin_place.block_span = ByteSpan::new(1, 1),
                38 => loan.origin_place.place = ScalarPlace::Name { name: "redirect".into(), span: ByteSpan::new(0, 0) },
                _ => unreachable!(),
            }
        },
        39 => cfg.evaluated_values.source_origin_derivations.clear(),
        40 => { let extra = cfg.evaluated_values.source_origin_derivations.iter().find(|row| matches!(row, null_constructor::SourceConstructorOriginDerivation::AllocationIdentityAddress(_))).expect("genuine candidate incidence").clone(); cfg.evaluated_values.source_origin_derivations.push(extra); },
        41..=44 => {
            let actual = cfg.evaluated_values.source_origin_derivations.iter_mut().find_map(|row| match row {
                null_constructor::SourceConstructorOriginDerivation::AllocationIdentityAddress(row) => Some(row),
                null_constructor::SourceConstructorOriginDerivation::NullExpression(_) | null_constructor::SourceConstructorOriginDerivation::CheckedParameter(_) | null_constructor::SourceConstructorOriginDerivation::DirectStorageAddress(_) => None,
            }).expect("genuine candidate incidence");
            match defect {
                41 => actual.origin.origin = ReferenceOriginId(usize::MAX),
                42 => actual.event.point = CfgPointId(usize::MAX),
                43 => actual.association.association = ReferenceRebindAssociationId(usize::MAX),
                44 => actual.association.owner = ProducerCfgOwnerId::Module(SourceIdentity::new("redirect".into(), "other".into(), "main.w".into(), "r1".into())),
                _ => unreachable!(),
            }
        },
        45 => {
            let extra = cfg.facts[row.origin.origin.0].clone();
            let next = ReferenceOriginId(cfg.facts.len());
            cfg.facts.push(extra);
            let CfgPointKind::AllocationIdentityAddress { request } = &mut cfg.initial_points[row.event.point.0].kind else { panic!("genuine original allocation fixture") };
            request.candidate.origin = next;
        },
        46 => cfg.initial_points[row.event.point.0].successors.clear(),
        47 => cfg.rebind_associations[ordinal].occurrence.output_projection.push(usize::MAX),
        _ => unreachable!(),
    }
}

#[test]
fn allocation_identity_five_genuine_sources_whole_original_and_candidate_defects() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                for (group, text) in SOURCES.iter().enumerate() {
                    let module = factory(endpoint, text, layout, project);
                    let original = forest(&module);
                    let count = original.sources.iter().map(|source| expected(source.resource_construction.original(), source.resource_construction.structs()).expect("true original source admission").len()).sum::<usize>();
                    assert_eq!(count, [1, 1, 1, 1, 2][group]);
                    for source in &original.sources {
                        let cfg = source.resource_construction.original();
                        let structs = source.resource_construction.structs();
                        reference::name_resolution::validate_null_constructors(cfg, structs).expect("four independently derived source families before mutation");
                        let rows = expected(cfg, structs).expect("original source complete tuple inventory");
                        assert_eq!(rows.len(), cfg.rebind_associations.len());
                        if rows.is_empty() { continue; }
                        for defect in 0..48 {
                            if defect == 27 && matches!(cfg.producer_owner, ProducerCfgOwnerId::Module(_)) { continue; }
                            let mut candidate = cfg.clone();
                            corrupt(&mut candidate, structs, defect);
                            let saved = candidate.clone();
                            assert!(reference::name_resolution::validate_null_constructors(&candidate, structs).is_err(), "group={group} defect={defect}");
                            assert_eq!(candidate, saved);
                            assert_eq!(cfg, source.resource_construction.original());
                        }
                        let candidate = serde_json::from_str::<ScalarReferenceCfg>(&serde_json::to_string(cfg).expect("candidate serialized")).expect("candidate decoded");
                        reference::name_resolution::validate_null_constructors(&candidate, structs).expect("candidate Serde retains external true original struct policy");
                        assert_eq!(candidate, *cfg);
                    }
                    assert_eq!(original, forest(&module));
                }
            }
        }
    }
}

#[test]
fn allocation_identity_nested_typed_reservations_and_captured_original_binding_identity() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                let nested = forest(&factory(endpoint, SOURCES[4], layout, project));
                let source = nested.sources.iter().find(|source| source.resource_construction.original().rebind_associations.len() == 2).expect("genuine nested reservations");
                let cfg = source.resource_construction.original();
                let rows = expected(cfg, source.resource_construction.structs()).expect("source preorder proven independently");
                assert_eq!(rows.iter().map(|row| row.association.association).collect::<Vec<_>>(), vec![ReferenceRebindAssociationId(1), ReferenceRebindAssociationId(0)]);
                assert!(cfg.rebind_associations[1].address_point < cfg.rebind_associations[0].address_point);
                assert!(cfg.rebind_associations[1].call_point < cfg.rebind_associations[0].address_point);
                let mut swapped = cfg.clone();
                swapped.rebind_associations.swap(0, 1);
                let saved = swapped.clone();
                assert!(expected(&swapped, source.resource_construction.structs()).is_err());
                assert_eq!(swapped, saved);
                let captured = forest(&factory(endpoint, SOURCES[3], layout, project));
                let child = captured.sources.iter().find(|source| matches!(source.producer, ProducerCfgOwnerId::Literal { .. }) && !source.resource_construction.original().rebind_associations.is_empty()).expect("genuine captured child creator");
                let cfg = child.resource_construction.original();
                let row = &expected(cfg, child.resource_construction.structs()).expect("true currentchild/ancestor source recipe")[0];
                assert_eq!(row.association.association, ReferenceRebindAssociationId(0));
                let ReferenceOrigin::Fresh { allocation, loan } = &cfg.facts[row.origin.origin.0] else { panic!("actual Fresh") };
                assert_eq!(allocation.source, loan.creation_source);
                assert_eq!(allocation.function_span, loan.origin_place.function_span);
                assert_ne!(loan.origin_place.function_span, loan.origin_place.binding.function_span);
                assert_eq!(loan.origin_place.declaration_span, loan.origin_place.binding.declaration_span);
                assert_eq!(loan.origin_place.block_span, loan.origin_place.binding.block_span);
            }
        }
    }
}

#[test]
fn allocation_identity_nested_original_index_and_control_incidence_defects_are_atomic() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            for endpoint in ["source", "destination"] {
                let original = forest(&factory(endpoint, SOURCES[4], layout, project));
                let source = original.sources.iter().find(|source| source.resource_construction.original().rebind_associations.len() == 2).expect("genuine nested source baseline");
                let cfg = source.resource_construction.original();
                let structs = source.resource_construction.structs();
                let row = expected(cfg, structs).expect("actual nested original").into_iter().find(|row| row.association.association == ReferenceRebindAssociationId(0)).expect("outer reserved before inner");
                for defect in 0..7 {
                    let mut candidate = cfg.clone();
                    let CfgPointKind::AllocationIdentityAddress { request } = &mut candidate.initial_points[row.event.point.0].kind else { panic!("actual projected allocation root") };
                    let value_point = request.indices[0].value_point;
                    match defect {
                        0 => request.indices.clear(),
                        1 => { let extra = request.indices[0].clone(); request.indices.push(extra); },
                        2 => request.indices[0].projection = usize::MAX,
                        3 => request.indices[0].value_point = row.event.point,
                        4 => candidate.initial_points[value_point.0].successors.clear(),
                        5 => {
                            let production = candidate.evaluated_values.productions.iter_mut().find(|production| production.evaluation_point == value_point).expect("actual canonical Block index output");
                            production.occurrence.range = ByteSpan::new(0, 0);
                        },
                        6 => {
                            let production = candidate.evaluated_values.productions.iter_mut().find(|production| production.evaluation_point == value_point).expect("actual canonical Block index output");
                            let OutputProductionSource::Block { scope_exit, .. } = &mut production.source else { panic!("genuine original block") };
                            *scope_exit = value_point;
                        },
                        _ => unreachable!(),
                    }
                    let saved = candidate.clone();
                    assert!(expected(&candidate, structs).is_err());
                    assert_eq!(candidate, saved);
                    assert_eq!(cfg, source.resource_construction.original());
                }
            }
        }
    }
}
