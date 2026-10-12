use super::*;
use crate::parse_source;

const SOURCE: &str = "%%start\nstruct Pair { u8 left; u8 right; }\nunit(*u8, u8[2], u8(), *u8, u8(), u8[], u8[], Pair, *?u8) inspect = fn(first, fixed, callback, last, other, left, right, pair, raw) { first; fixed[0]; callback; last; other; left; right; pair.left; raw; };\nunit() outer = fn { u8 captured = 7; unit(*u8, u8[2], u8(), *u8, u8(), u8[], u8[], Pair, *?u8) nested = fn(first, fixed, callback, last, other, left, right, pair, raw) { captured; first; fixed[0]; callback; last; other; left; right; pair.left; raw; }; nested; };\n%%end";

fn prove_cfg(cfg: &ScalarReferenceCfg, trace: &ReferenceValidationTestTrace) {
    let original = cfg.clone();
    let points = introductions(cfg).expect("actual total introductions");
    validate_owners(cfg).expect("actual total parameter owners");
    assert_eq!(cfg.reference_parameters.len(), 9);
    assert_eq!(points.iter().copied().collect::<BTreeSet<_>>().len(), 9);
    let signature = &cfg.evaluated_values.boundary_signatures[0];
    let mut checked = Vec::new();
    let mut nonchecked = Vec::new();
    for (ordinal, (binding, id)) in cfg.reference_parameters.iter().zip(&points).enumerate() {
        let point = &cfg.initial_points[id.0];
        assert_eq!(point.source_span, SourceSpan::new(binding.source.clone(), binding.declaration_span));
        assert_eq!(point.kind, CfgPointKind::Bind { binding: binding.clone() });
        assert!(cfg.binding_points.iter().all(|row| row.point != *id));
        assert!(cfg.evaluated_values.binding_inputs.iter().all(|row| row.consumer != *id));
        assert!(cfg.evaluated_values.values.iter().all(|row| row.id.point != *id));
        let port = signature.ports.iter().find(|port| port.role == BoundaryPortRole::Parameter { ordinal, binding: binding.clone() }).expect("original ordinal");
        let schema = cfg.evaluated_values.shapes.iter().find(|shape| shape.id == port.shape).expect("original declared shape");
        let source_shape = cfg.evaluated_values.source_binding_shapes.iter().find(|row| row.binding == *binding).expect("original binding shape");
        assert_eq!(source_shape.shape, port.shape);
        let owner = cfg.evaluated_values.owner_capabilities.iter().find(|owner| owner.role == OwnerRole::Parameter(binding.clone())).expect("actual parameter capability");
        assert_eq!(owner.id, OwnerCapabilityId { producer: cfg.producer_owner.clone(), introduction: *id });
        assert_eq!(owner.frame, signature.frame);
        for endpoint in &owner.endpoints {
            assert_eq!(endpoint.kind, OwnerEndpointKind::ParameterExit { binding: binding.clone() });
            assert_eq!(endpoint.frame, signature.frame);
            assert_eq!(cfg.initial_points[endpoint.event.point.0].source_span, SourceSpan::new(binding.source.clone(), binding.block_span));
            assert!(matches!(cfg.initial_points[endpoint.event.point.0].kind, CfgPointKind::ScopeExit));
        }
        if matches!(schema.ty, ScalarType::CheckedReference { .. }) {
            checked.push(*id);
            assert_eq!(cfg.checked_installs.iter().filter(|install| install.point == *id).count(), 1);
        } else {
            nonchecked.push(*id);
            assert!(cfg.checked_installs.iter().all(|install| install.point != *id));
            for pass in trace.passes.iter().filter(|pass| pass.owner == cfg.producer_owner) {
                let before = pass.observations.points.iter().filter(|snapshot| snapshot.point == *id && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation).collect::<Vec<_>>();
                let after = pass.observations.points.iter().filter(|snapshot| snapshot.point == *id && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).collect::<Vec<_>>();
                assert!(!before.is_empty());
                assert_eq!(before.len(), after.len());
                for (before, after) in before.iter().zip(after) {
                    assert_eq!(before.state.callable_bindings, after.state.callable_bindings);
                    assert_eq!(before.state.callable_projected, after.state.callable_projected);
                    assert_eq!(before.state.backing_bindings, after.state.backing_bindings);
                    assert_eq!(before.state.backing_projected, after.state.backing_projected);
                    assert_eq!(before.state.holder_relations, after.state.holder_relations);
                    assert!(before.state.holder_provenance == after.state.holder_provenance);
                    assert_eq!(before.state.bindings, after.state.bindings);
                    assert_eq!(before.state.index_values, after.state.index_values);
                    assert_eq!(before.state.uninitialized, after.state.uninitialized);
                    assert_eq!(before.state.released, after.state.released);
                    assert_eq!(before.state.invalidated, after.state.invalidated);
                    assert_eq!(before.state.moved, after.state.moved);
                    assert_eq!(after.state.callable_value, callable(&before.state, binding));
                    let seeded = before.state.backing_bindings.get(binding).into_iter().flatten().cloned().collect::<Vec<_>>();
                    assert_eq!(after.state.backing_value, seeded);
                }
                assert!(pass.cfg_at_exit.holder_edges.iter().filter(|edge| edge.from == *id).all(|edge| edge.actions.is_empty()));
            }
        }
    }
    assert_eq!(checked.len(), 2);
    assert_eq!(nonchecked.len(), 7);
    let ordered = checked.into_iter().chain(nonchecked).collect::<Vec<_>>();
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
    let encoded = serde_json::to_string(cfg).expect("original parameter CFG serialization");
    let decoded: ScalarReferenceCfg = serde_json::from_str(&encoded).expect("original parameter CFG deserialization");
    assert_eq!(decoded, *cfg);
    for mutation in 0..6 {
        introductions(cfg).expect("baseline before independent original negative");
        validate_owners(cfg).expect("baseline owner before independent negative");
        let mut wrong = cfg.clone();
        let id = points[1];
        match mutation {
            0 => wrong.evaluated_values.boundary_signatures[0].ports[1].role = BoundaryPortRole::Parameter { ordinal: 0, binding: cfg.reference_parameters[1].clone() },
            1 => wrong.points[id.0].source_span.range = cfg.reference_parameters[0].declaration_span,
            2 => wrong.points[id.0].kind = CfgPointKind::Join,
            3 => { let duplicate = wrong.points[id.0].clone(); wrong.points.push(duplicate); },
            4 => {
                let owner = wrong.evaluated_values.owner_capabilities.iter_mut().find(|owner| owner.role == OwnerRole::Parameter(cfg.reference_parameters[1].clone())).expect("original owner negative");
                owner.id.introduction = points[0];
            },
            5 => {
                let shape = wrong.evaluated_values.source_binding_shapes.iter_mut().find(|shape| shape.binding == cfg.reference_parameters[1]).expect("original shape negative");
                shape.shape = cfg.evaluated_values.boundary_signatures[0].ports[0].shape.clone();
            },
            _ => unreachable!(),
        }
        let before = wrong.clone();
        let error = validate_owners(&wrong).expect_err("independent original rejection");
        assert!(matches!(error, ReferenceAnalysisError::MissingContext { reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence, .. }));
        assert_eq!(wrong, before);
        assert_eq!(*cfg, original);
    }
}

fn prove(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity::new("total-parameter-introduction-proof".into(), "app".into(), "src/parameters.w".into(), "r1".into());
    let parsed = parse_source(source.clone(), SOURCE.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
    let (module, trace) = if project {
        let (validation, trace) = validate_scalar_project_observed_test(ScalarProject::new(vec![ScalarModule::from_program(validation.program, Vec::new())], vec![source]));
        assert!(validation.diagnostics.is_empty(), "{:#?}", validation.diagnostics);
        (validation.project.modules.into_iter().next().expect("actual project module"), trace)
    } else {
        (ScalarModule::from_program(validation.program, Vec::new()), trace)
    };
    let named = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "inspect" => Some(function), _ => None }).expect("actual named producer");
    prove_cfg(&named.reference_cfg, &trace);
    let outer = module.items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "outer" => Some(function), _ => None }).expect("actual literal parent");
    assert_eq!(outer.reference_cfg.literal_cfgs.len(), 1);
    prove_cfg(&outer.reference_cfg.literal_cfgs[0].reference_cfg, &trace);
}

#[test]
fn parameter_introduction_program_native64() { prove(ScalarTargetLayout::NATIVE64, false); }
#[test]
fn parameter_introduction_program_wasm32() { prove(ScalarTargetLayout::WASM32, false); }
#[test]
fn parameter_introduction_project_native64() { prove(ScalarTargetLayout::NATIVE64, true); }
#[test]
fn parameter_introduction_project_wasm32() { prove(ScalarTargetLayout::WASM32, true); }
