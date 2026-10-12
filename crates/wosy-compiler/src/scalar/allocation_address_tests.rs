use super::*;
use crate::parse_source;

fn callable_family_dependency(layout: ScalarTargetLayout, project: bool) {
    let source = SourceIdentity {
        project: "project".to_owned(),
        package: "package".to_owned(),
        path: "src/main.w".to_owned(),
        revision: "r1".to_owned(),
    };
    let text = "%%start\nunit(u64, *?u8) run = fn(count, raw) { (u8())[count] readers; u8 captured = 7; readers[0] = fn { captured }; unsafe { core.rebind(&readers, raw, count); }; readers; };\n%%end";
    let parsed = parse_source(source.clone(), text.to_owned(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let ordinary = derive_scalar_program_with_layout(&parsed.result, layout);
    let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
    assert_eq!(validation, ordinary);
    let result = if project {
        let project = ScalarProject::new(
            vec![ScalarModule::from_program(validation.program, Vec::new())],
            vec![source.clone()],
        );
        let ordinary = validate_scalar_project(project.clone());
        let (validation, trace) = validate_scalar_project_observed_test(project);
        assert_eq!(validation, ordinary);
        ObservedScalarValidation::Project { validation, trace }
    } else {
        ObservedScalarValidation::Program { validation, trace }
    };
    let (items, diagnostics, trace) = match &result {
        ObservedScalarValidation::Program { validation, trace } => {
            (&validation.program.items, &validation.diagnostics, trace)
        }
        ObservedScalarValidation::Project { validation, trace } => (
            &validation.project.modules[0].items,
            &validation.diagnostics,
            trace,
        ),
    };
    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    let ScalarItem::Function(function) = &items[0] else {
        panic!("actual run function")
    };
    let cfg = &function.reference_cfg;
    let pass = trace
        .passes
        .iter()
        .rfind(|pass| {
            pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
        })
        .expect("actual same-owner function pass");
    assert!(pass.errors.is_empty(), "{:?}", pass.errors);
    assert_eq!(pass.cfg_at_exit.initial_points, cfg.initial_points);
    let effect = cfg
        .points
        .iter()
        .find(|point| {
            matches!(
                point.kind,
                CfgPointKind::Invalidate {
                    operation: CoreOperationId::Rebind,
                    ..
                }
            )
        })
        .expect("actual Core Rebind effect");
    let CfgPointKind::Invalidate {
        candidates,
        address_point,
        inputs,
        ..
    } = &effect.kind
    else {
        unreachable!()
    };
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.place.source, source);
    assert_eq!(candidate.place.binding.function_span, function.span);
    assert!(cfg.allocations.contains(&candidate.place.binding));
    assert!(candidate.loan.parent.is_none());
    assert_eq!(candidate.loan.origin_place, candidate.place);
    let before = pass
        .observations
        .points
        .iter()
        .find(|snapshot| {
            snapshot.point == effect.id
                && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
        })
        .expect("actual incoming effect state");
    let after = pass
        .observations
        .points
        .iter()
        .find(|snapshot| {
            snapshot.point == effect.id
                && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
        })
        .expect("actual outgoing effect state");
    let backings = before
        .state
        .backing_bindings
        .get(&candidate.place.binding)
        .expect("actual initialized callable allocation backing");
    assert_eq!(backings.len(), 1);
    let backing = &backings[0];
    assert_eq!(backing.owner, candidate.place.binding);
    assert_eq!(backing.kind, RuntimeBackingKind::OwnedAllocation);
    assert_eq!(backing.callable_family.len(), 1);
    let family = &backing.callable_family[0];
    assert_eq!(family.elements.len(), 1);
    let element = &family.elements[0];
    let environment = &cfg.environments[element.environment.index];
    assert_eq!(environment.literal.source, source);
    assert!(!environment.inputs.is_empty());
    assert!(family.partitions.iter().any(|partition| {
        partition.representatives.iter().any(|representative| {
            representative.cell.alternatives.iter().any(|case| {
                matches!(case, CallableCellCaseFact::Initialized { environment_origin, backing: value, owner, .. }
                    if *environment_origin == element.environment && *value == backing.value && *owner == backing.owner)
            })
        })
    }));
    assert!(before.state.point_values[address_point.0]
        .as_ref()
        .expect("actual saved checked address")
        .contains(&candidate.origin));
    assert!(matches!(inputs, InvalidationInputs::Rebind {
        raw_address: ScalarExpression::Name { name: raw, .. },
        length: ScalarExpression::Name { name: count, .. },
        ..
    } if raw == "raw" && count == "count"));
    assert_eq!(
        after.state.backing_bindings.get(&candidate.place.binding),
        Some(backings)
    );
    assert_eq!(before.state.holder_relations, after.state.holder_relations);
    assert_eq!(
        before.state.holder_provenance,
        after.state.holder_provenance
    );
    assert_eq!(
        before.state.callable_projected,
        after.state.callable_projected
    );
    let edges = cfg
        .holder_edges
        .iter()
        .filter(|edge| edge.from == effect.id)
        .collect::<Vec<_>>();
    assert!(!edges.is_empty());
    assert!(edges.iter().all(|edge| edge.actions.is_empty()));
    assert!(edges.iter().all(|edge| {
        edge.before == before.state.holder_relations
            && edge.after == after.state.holder_relations
            && effect.successors.contains(&edge.to)
    }));
}

#[test]
fn d3_rebind_identity_callable_family_dependency_native_program() {
    callable_family_dependency(ScalarTargetLayout::NATIVE64, false);
}

#[test]
fn d3_rebind_identity_callable_family_dependency_native_project() {
    callable_family_dependency(ScalarTargetLayout::NATIVE64, true);
}

#[test]
fn d3_rebind_identity_callable_family_dependency_wasm_program() {
    callable_family_dependency(ScalarTargetLayout::WASM32, false);
}

#[test]
fn d3_rebind_identity_callable_family_dependency_wasm_project() {
    callable_family_dependency(ScalarTargetLayout::WASM32, true);
}
