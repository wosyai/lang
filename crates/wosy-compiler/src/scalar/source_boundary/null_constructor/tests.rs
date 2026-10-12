use super::*;

fn original(text: &str, layout: ScalarTargetLayout, project: bool) -> ScalarModule {
    let identity = SourceIdentity::new("null-original".into(), "app".into(), "src/main.w".into(), "r1".into());
    let parsed = crate::parse_source(identity.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let single = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(single.diagnostics.is_empty(), "{:?}", single.diagnostics);
    let module = ScalarModule::from_program(single.program, Vec::new());
    if project {
        let result = validate_scalar_project(ScalarProject::new(vec![module], vec![identity]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.project.modules.into_iter().next().expect("actual original module")
    } else { module }
}

fn context(module: &ScalarModule) -> SourceCapturePolicyContext {
    SourceCapturePolicyContext {
        structs: module.structs.clone(), enums: module.enums.clone(),
        generic_scope: module.top_level_reference_cfg.producer_owner.clone(),
        generic_declarations: Vec::new(), generic_parameters: BTreeSet::new(),
    }
}

#[test]
fn ordinary_null_originals_are_local_typed_events_while_raw_primary_is_empty() {
    let text = "%%start\n*i32 pointer = null;\n*i32() run = fn { *?i32 raw = null; raw; *i32 local = null; *i32() child = fn { null }; child; local };\nrun;\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let module = original(text, layout, project);
            let root = &module.top_level_reference_cfg;
            let function = module.items.iter().find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "run" => Some(function), _ => None,
            }).expect("genuine named Null root");
            let named = &function.reference_cfg;
            let child = &named.literal_cfgs[0];
            for cfg in [root, named, child.reference_cfg.as_ref()] {
                reference::name_resolution::validate_null_constructors(cfg, &module.structs).expect("independent typed ordinary source baseline");
                assert_eq!(cfg.source_null_constructors.len(), 1);
                let row = &cfg.source_null_constructors[0];
                assert_eq!(row.origin.owner, cfg.producer_owner);
                assert_eq!(row.event.owner, cfg.producer_owner);
                assert_eq!(row.definition, ReferenceOrigin::Null { span: row.source_span.range });
                assert_eq!(row.scope, cfg.initial_points[row.event.point.0].scope);
                assert_eq!(cfg.facts[row.origin.origin.0], row.definition);
                assert!(matches!(&row.expression, ScalarExpression::Name { name, span } if name == "null" && *span == row.source_span.range));
            }
            assert!(named.initial_points.iter().any(|point| matches!(&point.kind,
                CfgPointKind::Evaluate(ScalarExpression::Name { name, .. }) if name == "null")
                && point.possible_origins == Some(BTreeSet::new())));
            assert!(child.constructor_seed.parent_seed_origins.iter().any(|row| matches!(row.definition, ReferenceOrigin::Null { .. })));
            assert!(child.reference_cfg.source_null_constructors.iter().all(|row| row.origin.owner == child.reference_cfg.producer_owner));
            let sources = capture_table_sources(named, &context(&module)).expect("original Null resource source snapshot");
            assert_eq!(sources[0].resource_construction.original().source_null_constructors, named.source_null_constructors);
        }
    }
}

#[test]
fn cast_null_definition_keeps_a_distinct_call_family_from_ordinary_argument_null() {
    let text = "%%start\n*i32() run = fn { unsafe { core.pointer_cast<*i32>(null) } };\n%%end";
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let module = original(text, layout, project);
            let cfg = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) => Some(&function.reference_cfg), _ => None }).expect("actual checked-cast root");
            reference::name_resolution::validate_null_constructors(cfg, &module.structs).expect("typed ordinary argument baseline");
            assert_eq!(cfg.source_null_constructors.len(), 1);
            let call = cfg.initial_points.iter().find(|point| matches!(point.kind, CfgPointKind::Call { target: ResolvedCallTarget::Core(CoreOperationId::PointerCast), output: 0, .. })).expect("actual checked pointer_cast");
            assert!(call.possible_origins.as_ref().expect("original cast definition association").iter().any(|origin| cfg.facts[origin.0] == ReferenceOrigin::Null { span: call.source_span.range }));
            assert_ne!(cfg.source_null_constructors[0].event.point, call.id);
            assert_ne!(cfg.source_null_constructors[0].source_span, call.source_span);
        }
    }
}

#[test]
fn independent_null_inventory_defects_and_missing_typed_source_are_atomic() {
    for layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for project in [false, true] {
            let module = original("%%start\n*i32 pointer = null;\npointer;\n%%end", layout, project);
            let cfg = &module.top_level_reference_cfg;
            reference::name_resolution::validate_null_constructors(cfg, &module.structs).expect("genuine original Null baseline before defects");
            for defect in 0..13 {
                let mut corrupt = cfg.clone();
                match defect {
                    0 => corrupt.source_null_constructors.clear(),
                    1 => corrupt.source_null_constructors.push(corrupt.source_null_constructors[0].clone()),
                    2 => corrupt.source_null_constructors[0].origin.owner = ProducerCfgOwnerId::Literal { enclosing: Box::new(cfg.producer_owner.clone()), literal: cfg.source_null_constructors[0].source_span.clone(), capture_point: cfg.source_null_constructors[0].event.point },
                    3 => corrupt.source_null_constructors[0].origin.origin = ReferenceOriginId(cfg.facts.len()),
                    4 => corrupt.source_null_constructors[0].event.point = CfgPointId(cfg.initial_points.len()),
                    5 => corrupt.source_null_constructors[0].source_span.range = ByteSpan::new(0, 0),
                    6 => corrupt.source_null_constructors[0].scope = ReferenceScopeId(usize::MAX),
                    7 => corrupt.source_null_constructors[0].expression = ScalarExpression::Name { name: "other".into(), span: cfg.source_null_constructors[0].source_span.range },
                    8 => corrupt.source_null_constructors[0].definition = ReferenceOrigin::Null { span: ByteSpan::new(0, 0) },
                    9 => corrupt.facts[cfg.source_null_constructors[0].origin.origin.0] = ReferenceOrigin::Null { span: ByteSpan::new(0, 0) },
                    10 => corrupt.name_resolution_context = NameResolutionContext::Building,
                    11 => {
                        let mut record = cfg.source_null_constructors[0].clone();
                        let mut point = cfg.initial_points[record.event.point.0].clone();
                        point.id = CfgPointId(corrupt.initial_points.len());
                        record.event.point = point.id;
                        corrupt.initial_points.push(point);
                        corrupt.source_null_constructors.push(record);
                    },
                    12 => {
                        let row = &cfg.source_null_constructors[0];
                        let origin = ReferenceOriginId(corrupt.facts.len());
                        corrupt.facts.push(row.definition.clone());
                        corrupt.source_null_constructors[0].origin.origin = origin;
                        corrupt.initial_points[row.event.point.0].possible_origins = Some(BTreeSet::from([origin]));
                    },
                    _ => unreachable!(),
                }
                let saved = corrupt.clone();
                assert!(reference::name_resolution::validate_null_constructors(&corrupt, &module.structs).is_err());
                assert_eq!(corrupt, saved);
            }
            let serialized = serde_json::to_string(&cfg.source_null_constructors).expect("original Null records serialize");
            let restored = serde_json::from_str::<Vec<SourceNullConstructor>>(&serialized).expect("independent original Null record decode");
            assert_eq!(restored, cfg.source_null_constructors);
            let mut repeated = cfg.clone();
            repeated.source_null_constructors = restored;
            reference::name_resolution::validate_null_constructors(&repeated, &module.structs).expect("typed source replay after record Serde");
            reference::name_resolution::validate_null_constructors(&repeated, &module.structs).expect("repeat typed replay");
            assert_eq!(repeated, *cfg);
        }
    }
}
