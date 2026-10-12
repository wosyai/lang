use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct Packet { *?u8 data; u64 length; }\n*Packet() make_packet = fn { u64 length = 1; u8[length] bytes; bytes[0] = 4; *?u8 data = null; unsafe { data = &?bytes[0]; }; Packet result = { .data = data; .length = length; }; &result };\n%%end";

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new(
        "owner-introduction-endpoint-proof".into(),
        "app".into(),
        "src/packet.w".into(),
        "r1".into(),
    );
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let validation = derive_scalar_program_with_layout(&parsed.result, layout);
    assert!(
        validation.diagnostics.is_empty(),
        "{:#?}",
        validation.diagnostics
    );
    let items = if project {
        let validation = validate_scalar_project(ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source],
        ));
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        validation
            .project
            .modules
            .into_iter()
            .next()
            .expect("actual module")
            .items
    } else {
        validation.program.items
    };
    let functions = items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) if function.name == "make_packet" => Some(function),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [function] = functions.as_slice() else {
        panic!("actual source producer");
    };
    let cfg = &function.reference_cfg;
    validate(cfg).expect("canonical owner roles");
    let scopes = cfg
        .evaluated_values
        .owner_capabilities
        .iter()
        .filter(|owner| matches!(owner.role, OwnerRole::Scope(_)))
        .collect::<Vec<_>>();
    assert_eq!(scopes.len(), cfg.block_scope_exits.len());
    assert!(!scopes.is_empty());
    for scope in scopes {
        let OwnerRole::Scope(block) = &scope.role else {
            unreachable!();
        };
        let [endpoint] = scope.endpoints.as_slice() else {
            panic!("actual source exit");
        };
        assert_eq!(
            endpoint.kind,
            OwnerEndpointKind::ScopeExit {
                block: block.clone()
            }
        );
        assert_eq!(scope.id.introduction, scope.introduction.point);
        assert_eq!(scope.id.producer, cfg.producer_owner);
        assert_eq!(scope.frame, endpoint.frame);
        assert!(matches!(
            cfg.initial_points[scope.introduction.point.0].kind,
            CfgPointKind::ScopeEnter
        ));
        assert!(matches!(
            cfg.initial_points[endpoint.event.point.0].kind,
            CfgPointKind::ScopeExit
        ));
        assert!(cfg
            .block_scope_exits
            .iter()
            .any(|exit| exit.point == endpoint.event.point && &exit.occurrence == block));
        assert!(reaches(cfg, scope.introduction.point, endpoint.event.point));
    }
    let binds = cfg
        .initial_points
        .iter()
        .filter(|point| matches!(point.kind, CfgPointKind::Bind { .. }))
        .collect::<Vec<_>>();
    assert!(!binds.is_empty());
    for point in binds {
        let CfgPointKind::Bind { binding } = &point.kind else {
            unreachable!();
        };
        let owners = cfg
            .evaluated_values
            .owner_capabilities
            .iter()
            .filter(|owner| owner.introduction.point == point.id)
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            panic!("actual binding introduction");
        };
        assert_eq!(owner.role, OwnerRole::Binding(binding.clone()));
        let OwnerParent::Nested(parent) = &owner.parent else {
            panic!("actual lexical owner");
        };
        let scope = cfg
            .evaluated_values
            .owner_capabilities
            .iter()
            .find(|owner| &owner.id == parent)
            .expect("actual parent role");
        assert!(matches!(scope.role, OwnerRole::Scope(_)));
        assert_eq!(
            cfg.initial_points[scope.introduction.point.0]
                .source_span
                .range,
            binding.block_span
        );
        assert_eq!(owner.endpoints, scope.endpoints);
    }
    for input in &cfg.evaluated_values.final_inputs {
        let owners = cfg
            .evaluated_values
            .owner_capabilities
            .iter()
            .filter(|owner| owner.id.introduction == input.consumer.point)
            .collect::<Vec<_>>();
        let [owner] = owners.as_slice() else {
            panic!("actual result receiving port");
        };
        assert_eq!(owner.role, OwnerRole::Result(input.consumer.clone()));
        let [endpoint] = owner.endpoints.as_slice() else {
            panic!("actual handoff endpoint");
        };
        assert_eq!(endpoint.event.point, input.consumer.point);
        assert_eq!(
            endpoint.kind,
            OwnerEndpointKind::ResultHandoff {
                consumer: input.consumer.clone()
            }
        );
    }
    let encoded = serde_json::to_string(cfg).expect("owner CFG serialization");
    let decoded: ScalarReferenceCfg =
        serde_json::from_str(&encoded).expect("owner CFG deserialization");
    assert_eq!(decoded, *cfg);
    let mut wrong = cfg.clone();
    wrong.evaluated_values.owner_capabilities[0]
        .introduction
        .point = cfg.block_scope_exits[0].point;
    assert!(validate(&wrong).is_err());
    let mut wrong = cfg.clone();
    wrong.evaluated_values.owner_capabilities[0]
        .endpoints
        .clear();
    assert!(validate(&wrong).is_err());
    let mut wrong = cfg.clone();
    let first = wrong.evaluated_values.owner_capabilities[0].clone();
    wrong.evaluated_values.owner_capabilities.push(first);
    assert!(validate(&wrong).is_err());
    let mut wrong = cfg.clone();
    wrong.evaluated_values.owner_capabilities[0].parent =
        OwnerParent::Nested(wrong.evaluated_values.owner_capabilities[0].id.clone());
    assert!(validate(&wrong).is_err());
}

#[test]
fn owner_lifetime_source_program_native64() {
    prove(ScalarTargetLayout::NATIVE64, false);
}
#[test]
fn owner_lifetime_source_program_wasm32() {
    prove(ScalarTargetLayout::WASM32, false);
}
#[test]
fn owner_lifetime_source_project_native64() {
    prove(ScalarTargetLayout::NATIVE64, true);
}
#[test]
fn owner_lifetime_source_project_wasm32() {
    prove(ScalarTargetLayout::WASM32, true);
}
