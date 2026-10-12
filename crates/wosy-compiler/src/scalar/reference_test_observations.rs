use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ReferenceTransferTestPassId(pub(super) usize);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReferenceTransferTestRole {
    TopLevel,
    Function,
    LiteralRefresh,
    SpecializedCopy { types: Vec<ScalarType> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReferenceTransferTestStage {
    InitialConstruction,
    Revalidation,
}

#[derive(Clone, Debug)]
pub(super) struct ReferenceTransferTestContext {
    pub(super) capture_input: source_boundary::SourceCapturePreflightInput,
    pub(super) deferred_policies: BTreeMap<usize, CopyPolicy>,
    pub(super) contracts: Vec<ConcreteReferenceOutputContract>,
    pub(super) literal_contracts: Vec<LiteralOutputContract>,
    pub(super) call_exits: Vec<GlobalCallExitSnapshot>,
}

#[derive(Clone)]
pub(super) struct ReferenceBorrowTransferSnapshot {
    pub(super) point: CfgPointId,
    pub(super) state: ReferenceEdgeState,
    pub(super) input: ReferenceBorrowInput,
    pub(super) cases: Vec<ReferenceBorrowCase>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReferenceTransferPhase {
    BeforeEvaluation,
    BeforeInputCleanup,
    AfterInputCleanup,
}

#[derive(Clone)]
pub(super) struct ReferenceTransferPointSnapshot {
    pub(super) point: CfgPointId,
    pub(super) phase: ReferenceTransferPhase,
    pub(super) state: ReferenceEdgeState,
}

#[derive(Clone)]
pub(super) struct ReferenceTransferTestObservations {
    pub(super) borrows: Vec<ReferenceBorrowTransferSnapshot>,
    pub(super) points: Vec<ReferenceTransferPointSnapshot>,
}

#[derive(Clone)]
pub(super) struct ReferenceTransferTestPass {
    pub(super) id: ReferenceTransferTestPassId,
    pub(super) owner: ProducerCfgOwnerId,
    pub(super) role: ReferenceTransferTestRole,
    pub(super) stage: ReferenceTransferTestStage,
    pub(super) context: ReferenceTransferTestContext,
    pub(super) cfg_at_entry: ScalarReferenceCfg,
    pub(super) cfg_at_exit: ScalarReferenceCfg,
    pub(super) observations: ReferenceTransferTestObservations,
    pub(super) errors: Vec<ReferenceAnalysisError>,
}

#[derive(Clone)]
pub(in crate::scalar) struct ReferenceValidationTestTrace {
    pub(super) passes: Vec<ReferenceTransferTestPass>,
}

impl ReferenceValidationTestTrace {
    pub(in crate::scalar) fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub(in crate::scalar) fn construction_fact(
        &self,
        cfg: &ScalarReferenceCfg,
        fact: &EvaluatedValueFact,
    ) -> Result<&EvaluatedValueFact, ReferenceAnalysisError> {
        let invalid = || ReferenceAnalysisError::MissingContext {
            source_span: fact.id.resolved.occurrence.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        let role = match &cfg.producer_owner {
            ProducerCfgOwnerId::Function { .. } => ReferenceTransferTestRole::Function,
            ProducerCfgOwnerId::Literal { .. } => ReferenceTransferTestRole::LiteralRefresh,
            _ => return Err(invalid()),
        };
        let pass = self.passes.iter().find(|pass| {
            pass.owner == cfg.producer_owner
                && pass.role == role
                && pass.stage == ReferenceTransferTestStage::InitialConstruction
        }).ok_or_else(invalid)?;
        if pass.cfg_at_entry.producer_owner != cfg.producer_owner
            || pass.cfg_at_exit.producer_owner != cfg.producer_owner
            || fact.id.resolved.producer != cfg.producer_owner
            || fact.shape.producer != cfg.producer_owner
            || self.passes.get(pass.id.0).map(|known| known.id) != Some(pass.id) {
            return Err(invalid());
        }
        let original = pass.cfg_at_entry.evaluated_values.values.iter()
            .filter(|original| original.id == fact.id).collect::<Vec<_>>();
        let validated = cfg.evaluated_values.values.iter()
            .filter(|validated| validated.id == fact.id).collect::<Vec<_>>();
        let ([original], [validated]) = (original.as_slice(), validated.as_slice()) else {
            return Err(invalid());
        };
        if *validated != fact || original.shape != fact.shape {
            return Err(invalid());
        }
        let original_shapes = pass.cfg_at_entry.evaluated_values.shapes.iter()
            .filter(|shape| shape.id == original.shape).collect::<Vec<_>>();
        let validated_shapes = cfg.evaluated_values.shapes.iter()
            .filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
        let ([original_shape], [validated_shape]) =
            (original_shapes.as_slice(), validated_shapes.as_slice()) else {
                return Err(invalid());
            };
        if original_shape != validated_shape {
            return Err(invalid());
        }
        let original_point = pass.cfg_at_entry.initial_points.get(fact.id.point.0)
            .ok_or_else(invalid)?;
        let validated_point = cfg.initial_points.get(fact.id.point.0).ok_or_else(invalid)?;
        if original_point.id != fact.id.point
            || validated_point.id != fact.id.point
            || original_point.source_span != fact.id.resolved.occurrence
            || original_point.source_span != validated_point.source_span
            || original_point.kind != validated_point.kind
            || original_point.scope != validated_point.scope {
            return Err(invalid());
        }
        Ok(original)
    }

    pub(in crate::scalar) fn assert_construction_rejections(
        &self,
        cfg: &ScalarReferenceCfg,
        fact: &EvaluatedValueFact,
    ) {
        self.construction_fact(cfg, fact).expect("actual initial source correspondence");
        let mut removed = self.clone();
        removed.passes.retain(|pass| pass.owner != cfg.producer_owner
            || pass.stage != ReferenceTransferTestStage::InitialConstruction);
        assert!(removed.construction_fact(cfg, fact).is_err());
        let mut wrong_role = self.clone();
        for pass in &mut wrong_role.passes {
            if pass.owner == cfg.producer_owner
                && pass.stage == ReferenceTransferTestStage::InitialConstruction {
                pass.role = ReferenceTransferTestRole::TopLevel;
            }
        }
        assert!(wrong_role.construction_fact(cfg, fact).is_err());
        let mut crossed = fact.clone();
        crossed.shape.producer = ProducerCfgOwnerId::Module(fact.id.resolved.occurrence.source.clone());
        assert!(self.construction_fact(cfg, &crossed).is_err());
        let mut foreign = self.clone();
        for pass in &mut foreign.passes {
            if pass.owner == cfg.producer_owner
                && pass.stage == ReferenceTransferTestStage::InitialConstruction {
                pass.owner = match &pass.owner {
                    ProducerCfgOwnerId::Literal { literal, enclosing, capture_point } =>
                        ProducerCfgOwnerId::Literal {
                            literal: literal.clone(),
                            enclosing: enclosing.clone(),
                            capture_point: CfgPointId(capture_point.0 + 1),
                        },
                    _ => ProducerCfgOwnerId::Module(fact.id.resolved.occurrence.source.clone()),
                };
            }
        }
        assert!(foreign.construction_fact(cfg, fact).is_err());
    }

    pub(in crate::scalar) fn selected_plain_fact(
        &self,
        cfg: &ScalarReferenceCfg,
        fact: &EvaluatedValueFact,
    ) -> Result<(), ReferenceAnalysisError> {
        let invalid = || ReferenceAnalysisError::MissingContext {
            source_span: fact.id.resolved.occurrence.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        let tables = &cfg.evaluated_values;
        let shapes = tables.shapes.iter().filter(|shape| shape.id == fact.shape).collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else { return Err(invalid()); };
        if fact.id.resolved.producer != cfg.producer_owner
            || fact.shape.producer != cfg.producer_owner
            || !matches!(shape.payload, ValuePayloadShape::Plain(_)) {
            return Err(invalid());
        }
        let slots = tables.source_slots.iter().filter(|slot| {
            slot.role == SourceCoordinateRole::Evaluated(fact.id.clone())
        }).collect::<Vec<_>>();
        let [slot] = slots.as_slice() else { return Err(invalid()); };
        if slot.id.owner != cfg.producer_owner || slot.shape != fact.shape { return Err(invalid()); }
        let contributors = tables.source_contributors.iter().filter(|tuple| {
            tuple.event.owner == cfg.producer_owner && tuple.event.point == fact.id.point
                && tuple.outputs.contains(&fact.id)
        }).collect::<Vec<_>>();
        if contributors.is_empty() || fact.cases.is_empty() { return Err(invalid()); }
        for (alternative, case) in fact.cases.iter().enumerate() {
            if case.id.value != fact.id || case.id.alternative != alternative
                || case.path.producer != cfg.producer_owner
                || case.availability != ValueAvailability::Available
                || case.payload != EvaluatedPayloadFact::Plain || !case.dependencies.is_empty()
                || !tables.source_case_views.iter().any(|view| view.case == case.id
                    && view.port == slot.id && contributors.iter().any(|tuple| {
                        tuple.id == view.contributor && tuple.output == view.configuration
                    })) {
                return Err(invalid());
            }
        }
        for contributor in contributors {
            let observations = tables.source_case_views.iter().filter(|view| {
                view.case.value == fact.id && view.port == slot.id
                    && view.contributor == contributor.id && view.configuration == contributor.output
            }).collect::<Vec<_>>();
            let [observation] = observations.as_slice() else { return Err(invalid()); };
            let cases = fact.cases.iter().filter(|case| case.id == observation.case).collect::<Vec<_>>();
            let [case] = cases.as_slice() else { return Err(invalid()); };
            let paths = tables.source_paths.iter().filter(|path| path.id == observation.path).collect::<Vec<_>>();
            let [path] = paths.as_slice() else { return Err(invalid()); };
            let configurations = tables.source_configurations.iter().filter(|row| row.id == contributor.output).collect::<Vec<_>>();
            let [configuration] = configurations.as_slice() else { return Err(invalid()); };
            let selected = configuration.slots.iter().filter(|selected| selected.coordinate == slot.id).collect::<Vec<_>>();
            let [selected] = selected.as_slice() else { return Err(invalid()); };
            let views = tables.source_views.iter().filter(|view| view.id == observation.view).collect::<Vec<_>>();
            let [view] = views.as_slice() else { return Err(invalid()); };
            if case.path != observation.path || path.event != contributor.event
                || path.configuration != configuration.id || path.contributor != contributor.id
                || path.frame != configuration.frame
                || selected.view != view.id || selected.availability != case.availability
                || view.shape != fact.shape || view.payload != SourcePayloadView::Plain
                || view.dependencies != case.dependencies
                || view.id.owner != cfg.producer_owner || configuration.id.owner != cfg.producer_owner
                || contributor.id.owner != cfg.producer_owner {
                return Err(invalid());
            }
            let equations = tables.source_port_equations.iter().filter(|equation| equation.contributor == contributor.id).collect::<Vec<_>>();
            let [equation] = equations.as_slice() else { return Err(invalid()); };
            let monitors = tables.source_port_monitors.iter().filter(|monitor| monitor.id == equation.output).collect::<Vec<_>>();
            let [monitor] = monitors.as_slice() else { return Err(invalid()); };
            let input_monitors = tables.source_port_monitors.iter().filter(|monitor| monitor.id == equation.input).collect::<Vec<_>>();
            let [_input_monitor] = input_monitors.as_slice() else { return Err(invalid()); };
            let ports = monitor.ports.iter().filter(|port| port.coordinate == slot.id).collect::<Vec<_>>();
            let [port] = ports.as_slice() else { return Err(invalid()); };
            if monitor.contributor != contributor.id || port.observation != **observation
                || monitor.pending.contains(&slot.id)
                || port.sources.is_empty()
                || port.sources.iter().any(|source| !tables.source_case_views.contains(source))
                || !equation.productions.iter().any(|production| production.observation == **observation)
                || !contributor.productions.iter().any(|production| production.destination == slot.id) {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

pub(in crate::scalar) enum ObservedScalarValidation {
    Program {
        validation: ScalarValidation,
        trace: ReferenceValidationTestTrace,
    },
    Project {
        validation: ScalarProjectValidation,
        trace: ReferenceValidationTestTrace,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_source;

    fn source() -> SourceIdentity {
        SourceIdentity {
            project: "project".to_owned(),
            package: "package".to_owned(),
            path: "src/main.w".to_owned(),
            revision: "r1".to_owned(),
        }
    }

    fn observed(text: &str, layout: ScalarTargetLayout, project: bool) -> ObservedScalarValidation {
        let parsed = parse_source(source(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty());
        let ordinary = derive_scalar_program_with_layout(&parsed.result, layout);
        let (validation, trace) = derive_scalar_program_observed_test(&parsed.result, layout);
        assert_eq!(validation, ordinary);
        if project {
            let project = ScalarProject::new(
                vec![ScalarModule::from_program(validation.program, Vec::new())],
                vec![source()],
            );
            let ordinary = validate_scalar_project(project.clone());
            let (validation, trace) = validate_scalar_project_observed_test(project);
            assert_eq!(validation, ordinary);
            ObservedScalarValidation::Project { validation, trace }
        } else {
            ObservedScalarValidation::Program { validation, trace }
        }
    }

    fn context_correspondence(layout: ScalarTargetLayout, project: bool) {
        for text in [
            "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *child = 2; *parent = 3; };\nrun();\n%%end",
            "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; unit() write = fn { *!u32 child = &!(*parent); *!u32 grandchild = &!(*child); *grandchild = 2; *child = 3; }; write(); *parent = 4; };\n%%end",
        ] {
            let result = observed(text, layout, project);
            let (items, diagnostics, trace) = match &result {
                ObservedScalarValidation::Program { validation, trace } => (&validation.program.items, &validation.diagnostics, trace),
                ObservedScalarValidation::Project { validation, trace } => (&validation.project.modules[0].items, &validation.diagnostics, trace),
            };
            assert!(diagnostics.is_empty(), "{diagnostics:#?}");
            let cfg = items.iter().find_map(|item| match item { ScalarItem::Function(function) if function.name == "run" => Some(&function.reference_cfg), _ => None }).unwrap();
            let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function).unwrap();
            assert_eq!(pass.cfg_at_exit.initial_points, cfg.initial_points);
            assert_eq!(pass.cfg_at_exit.checked_bindings, cfg.checked_bindings);
            assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
            assert_eq!(pass.cfg_at_exit.borrow_evaluations, cfg.borrow_evaluations);
            assert!(pass.errors.is_empty());
            assert!(pass.context.contracts.is_empty());
            assert!(pass.context.deferred_policies.is_empty());
            assert_eq!(pass.cfg_at_entry.producer_owner, cfg.producer_owner);
            for (index, pass) in trace.passes.iter().enumerate() {
                assert_eq!(pass.id, ReferenceTransferTestPassId(index));
                for borrow in &pass.observations.borrows {
                    assert!(pass.cfg_at_exit.borrow_evaluations.iter().any(|evaluation| evaluation.point == borrow.point && evaluation.input == borrow.input));
                    let CfgPointKind::Borrow { request } = &pass.cfg_at_entry.initial_points[borrow.point.0].kind else { unreachable!() };
                    if let ReferenceBorrowRequest::Through { pointer_point, .. } = request {
                        let ReferenceBorrowInput::Through { origins, .. } = &borrow.input else { unreachable!() };
                        assert_eq!(borrow.state.point_values[pointer_point.0].as_ref(), Some(origins));
                    }
                    assert!(!borrow.cases.is_empty());
                    let before_cleanup = pass.observations.points.iter().find(|snapshot| snapshot.point == borrow.point && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).unwrap();
                    let after_cleanup = pass.observations.points.iter().find(|snapshot| snapshot.point == borrow.point && snapshot.phase == ReferenceTransferPhase::AfterInputCleanup).unwrap();
                    if let ReferenceBorrowInput::Through { pointer_point, origins } = &borrow.input {
                        assert_eq!(before_cleanup.state.point_values[pointer_point.0].as_ref(), Some(origins));
                        assert!(after_cleanup.state.point_values[pointer_point.0].is_none());
                    }
                    for case in &borrow.cases {
                        let ReferenceBorrowCase::Accepted { origin, loan, access, .. } = case else { unreachable!() };
                        let install = pass.cfg_at_exit.checked_install_evaluations.iter().find(|evaluation| matches!(evaluation.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == borrow.point)).unwrap();
                        assert_eq!(install.accepted_origins, BTreeSet::from([*origin]));
                        let after_install = pass.observations.points.iter().find(|snapshot| snapshot.point == install.point && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).unwrap();
                        assert_eq!(stored_reference_origins(&after_install.state, &install.target), install.accepted_origins);
                        assert!(after_install.state.point_values[borrow.point.0].as_ref().unwrap().contains(origin));
                        let after_install_cleanup = pass.observations.points.iter().find(|snapshot| snapshot.point == install.point && snapshot.phase == ReferenceTransferPhase::AfterInputCleanup).unwrap();
                        assert!(after_install_cleanup.state.point_values[borrow.point.0].is_none());
                        let ReferenceOrigin::Fresh { loan: fact_loan, allocation } = &pass.cfg_at_exit.facts[origin.0] else { unreachable!() };
                        assert_eq!(fact_loan, loan);
                        assert_eq!(loan.creation_source, pass.cfg_at_entry.initial_points[borrow.point.0].source_span.source);
                        if let ReferenceBorrowAccess::Through { parent_origin, .. } = access {
                            let ReferenceOrigin::Fresh { loan: parent, allocation: parent_allocation } = &pass.cfg_at_exit.facts[parent_origin.0] else { unreachable!() };
                            assert_eq!(loan.parent.as_deref(), Some(parent));
                            assert_eq!(allocation, parent_allocation);
                        }
                    }
                }
                assert!(pass.observations.points.iter().any(|snapshot| snapshot.phase == ReferenceTransferPhase::BeforeEvaluation));
                assert!(pass.observations.points.iter().any(|snapshot| snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup));
                assert!(pass.observations.points.iter().any(|snapshot| snapshot.phase == ReferenceTransferPhase::AfterInputCleanup));
                for point in &pass.cfg_at_exit.points {
                    let CfgPointKind::AssignThrough { pointer_point, candidates, .. } = &point.kind else { continue };
                    let before = pass.observations.points.iter().find(|snapshot| snapshot.point == point.id && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation).unwrap();
                    let after = pass.observations.points.iter().find(|snapshot| snapshot.point == point.id && snapshot.phase == ReferenceTransferPhase::AfterInputCleanup).unwrap();
                    let origins = before.state.point_values[pointer_point.0].as_ref().unwrap();
                    assert_eq!(origins.len(), 1);
                    assert_eq!(candidates.len(), 1);
                    assert_eq!(origins, &BTreeSet::from([candidates[0].origin]));
                    let ReferenceOrigin::Fresh { loan, .. } = &pass.cfg_at_exit.facts[candidates[0].origin.0] else { unreachable!() };
                    assert_eq!(loan, &candidates[0].loan);
                    assert!(after.state.point_values[pointer_point.0].is_none());
                    let CfgPointKind::Read { binding } = &pass.cfg_at_entry.initial_points[pointer_point.0].kind else { unreachable!() };
                    assert_eq!(before.state.bindings[binding], after.state.bindings[binding]);
                }
            }
            for nested in &cfg.literal_cfgs {
                let nested_cfg = &nested.reference_cfg;
                let nested_pass = trace.passes.iter().rfind(|pass| pass.owner == nested_cfg.producer_owner && pass.role == ReferenceTransferTestRole::LiteralRefresh).unwrap();
                assert_eq!(nested_pass.cfg_at_exit.facts, nested_cfg.facts);
                assert_eq!(nested_pass.cfg_at_entry.inputs, nested_cfg.inputs);
                assert_eq!(nested_pass.cfg_at_exit.borrow_evaluations, nested_cfg.borrow_evaluations);
                assert!(nested_pass.context.call_exits.is_empty());
                assert!(nested_pass.context.literal_contracts.is_empty());
                for input in &nested_pass.cfg_at_entry.inputs {
                    for origin in &input.origins {
                        let mapped = &nested_pass.cfg_at_entry.facts[origin.0];
                        assert!(pass.cfg_at_exit.facts.contains(mapped));
                    }
                }
            }
            let restored: ScalarReferenceCfg = serde_json::from_str(&serde_json::to_string(cfg).unwrap()).unwrap();
            assert_eq!(cfg, &restored);
        }
    }

    macro_rules! context_test {
        ($name:ident, $layout:expr, $project:expr) => {
            #[test]
            fn $name() {
                context_correspondence($layout, $project);
            }
        };
    }
    context_test!(
        d3_reborrow_state_evaluated_borrow_trace_native_program,
        ScalarTargetLayout::NATIVE64,
        false
    );
    context_test!(
        d3_reborrow_state_evaluated_borrow_trace_native_project,
        ScalarTargetLayout::NATIVE64,
        true
    );
    context_test!(
        d3_reborrow_state_evaluated_borrow_trace_wasm_program,
        ScalarTargetLayout::WASM32,
        false
    );
    context_test!(
        d3_reborrow_state_evaluated_borrow_trace_wasm_project,
        ScalarTargetLayout::WASM32,
        true
    );

    #[derive(Clone, Copy)]
    enum Consumer {
        Bind,
        Assign,
        AssignThrough,
        Return,
    }

    #[derive(Clone, Copy)]
    enum Defect {
        ForeignOrigin,
        FullParent,
    }

    fn consumer_text(consumer: Consumer) -> &'static str {
        match consumer {
            Consumer::Bind => "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 child = &!(*parent); *child = 2; *parent = 3; };\nrun();\n%%end",
            Consumer::Assign => "%%start\nunit() run = fn { u32 item = 1; u32 old = 2; *!u32 parent = &!item; *!u32 destination = &!old; destination = &!(*parent); *destination = 3; *parent = 4; };\nrun();\n%%end",
            Consumer::AssignThrough => "%%start\nunit() run = fn { u32 item = 1; u32 old = 2; *!u32 parent = &!item; *!u32 destination = &!old; *!(*!u32) slot = &!destination; *slot = &!(*parent); *destination = 3; *parent = 4; };\nrun();\n%%end",
            Consumer::Return => "%%start\n*!u32() run = fn { u32 item = 1; *!u32 parent = &!item; &!(*parent) };\n%%end",
        }
    }

    fn assert_scalar_assignment_resource_bound(
        function: &ScalarFunction,
        pass: &ReferenceTransferTestPass,
        prior: &ReferenceTransferPointSnapshot,
        current: &ReferenceEdgeState,
    ) {
        let binds = pass
            .cfg_at_entry
            .initial_points
            .iter()
            .filter_map(|point| match &point.kind {
                CfgPointKind::Bind { binding } if point.id < prior.point => Some((point, binding)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            prior
                .state
                .callable_bindings
                .keys()
                .collect::<BTreeSet<_>>(),
            binds
                .iter()
                .map(|(_, binding)| *binding)
                .collect::<BTreeSet<_>>()
        );
        for (point, binding) in binds {
            assert_eq!(binding.source, point.source_span.source);
            assert_eq!(binding.source, source());
            assert_eq!(binding.kind, ReferenceBindingKind::Declared);
            assert_eq!(pass.owner, function.reference_cfg.producer_owner);
            assert!(
                matches!(&pass.owner, ProducerCfgOwnerId::Function { source, declaration, body: ProducerFunctionBodyId::Declaration }
                if source == &binding.source && *declaration == function.name_span)
            );
            assert_eq!(binding.function_span, function.span);
            assert_eq!(binding.declaration_span, point.source_span.range);
            let post_bind = pass
                .observations
                .points
                .iter()
                .find(|snapshot| {
                    snapshot.point == point.id
                        && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
                })
                .unwrap();
            assert!(post_bind.state.callable_bindings[binding].is_empty());
            assert_eq!(
                prior.state.callable_bindings[binding],
                post_bind.state.callable_bindings[binding]
            );
        }
        for state in [&prior.state, current] {
            assert!(state.backing_bindings.is_empty());
            assert!(state.backing_projected.is_empty());
            assert!(state.transferred_backings.is_empty());
            assert!(state.holder_relations.is_empty());
            assert!(state.holder_provenance.entries.is_empty());
            assert!(state.holder_value.is_empty());
            assert!(state.holder_sources.is_empty());
            assert!(state.holder_point_values.iter().all(Vec::is_empty));
            assert!(state.holder_point_sources.iter().all(Vec::is_empty));
            assert!(state.callable_bindings.values().all(BTreeSet::is_empty));
            assert!(state.callable_projected.is_empty());
        }
        assert_eq!(current.bindings, prior.state.bindings);
        assert!(current.projected == prior.state.projected);
        assert_eq!(current.moved, prior.state.moved);
        assert_eq!(current.uninitialized, prior.state.uninitialized);
        assert_eq!(current.released, prior.state.released);
        assert_eq!(current.invalidated, prior.state.invalidated);
        assert_eq!(current.selected_element, prior.state.selected_element);
        assert_eq!(current.index_values, prior.state.index_values);
        assert_eq!(current.backing_bindings, prior.state.backing_bindings);
        assert_eq!(current.backing_projected, prior.state.backing_projected);
        assert_eq!(current.holder_relations, prior.state.holder_relations);
        assert!(current.holder_provenance == prior.state.holder_provenance);
        assert_eq!(current.callable_bindings, prior.state.callable_bindings);
        assert_eq!(current.callable_projected, prior.state.callable_projected);
    }

    fn assert_retained_scalar_destination(
        function: &ScalarFunction,
        pass: &ReferenceTransferTestPass,
        prior: &ReferenceTransferPointSnapshot,
        cfg: &ScalarReferenceCfg,
        destination: &ReferencePlaceId,
        current: &ReferenceEdgeState,
        child_origin: ReferenceOriginId,
    ) {
        assert_eq!(prior.phase, ReferenceTransferPhase::BeforeEvaluation);
        assert_eq!(pass.owner, cfg.producer_owner);
        let origins = stored_reference_origins(&prior.state, destination);
        assert_eq!(origins.len(), 1);
        assert!(!origins.contains(&child_origin));
        assert_eq!(stored_reference_origins(current, destination), origins);
        assert_eq!(current.bindings, prior.state.bindings);
        assert!(current.projected == prior.state.projected);
        assert_eq!(current.moved, prior.state.moved);
        assert_eq!(current.uninitialized, prior.state.uninitialized);
        assert_eq!(current.released, prior.state.released);
        assert_eq!(current.invalidated, prior.state.invalidated);
        assert_eq!(destination.source, source());
        assert_eq!(destination.binding.source, source());
        assert_eq!(destination.binding.kind, ReferenceBindingKind::Declared);
        assert!(matches!(&cfg.checked_bindings[&destination.binding].place,
            ScalarPlace::Name { name, span } if name == "destination" && *span == destination.binding.declaration_span));
        assert_eq!(destination.function_span, destination.binding.function_span);
        assert_eq!(
            destination.declaration_span,
            destination.binding.declaration_span
        );
        assert_eq!(destination.block_span, destination.binding.block_span);
        let origin = *origins.first().unwrap();
        let ReferenceOrigin::Fresh { allocation, loan } = &pass.cfg_at_exit.facts[origin.0] else {
            unreachable!("actual prior scalar allocation and loan")
        };
        let producer = pass
            .observations
            .borrows
            .iter()
            .find(|snapshot| {
                snapshot.cases.iter().any(|case| {
                    matches!(case,
                ReferenceBorrowCase::Accepted { origin: actual, .. } if *actual == origin)
                })
            })
            .unwrap();
        assert!(producer.point < prior.point);
        let producer_point = &pass.cfg_at_entry.initial_points[producer.point.0];
        let CfgPointKind::Borrow {
            request:
                ReferenceBorrowRequest::DirectStorage {
                    place,
                    loan: requested_loan,
                    origin: requested_origin,
                    ..
                },
        } = &producer_point.kind
        else {
            unreachable!("actual old allocation address producer")
        };
        assert_eq!(*requested_origin, origin);
        assert_eq!(loan, requested_loan);
        assert_eq!(&loan.origin_place, place);
        assert!(
            matches!(&loan.origin_place.place, ScalarPlace::Name { name, .. } if name == "old")
        );
        assert_eq!(loan.mode, ScalarReferenceMutability::Mutable);
        assert!(loan.parent.is_none());
        assert_eq!(loan.creation_source, producer_point.source_span.source);
        assert_eq!(loan.creation_span, producer_point.source_span.range);
        assert_eq!(allocation.source, loan.origin_place.binding.source);
        assert_eq!(
            allocation.function_span,
            loan.origin_place.binding.function_span
        );
        assert_eq!(
            allocation.declaration_span,
            loan.origin_place.binding.declaration_span
        );
        assert_eq!(allocation.creation_span, producer_point.source_span.range);
        assert!(matches!(&producer.cases[..],
            [ReferenceBorrowCase::Accepted { origin: actual, loan: realized, place: realized_place, access: ReferenceBorrowAccess::DirectStorage }]
                if *actual == origin && realized == loan && realized_place == place));
        let prior_install = pass.cfg_at_exit.checked_install_evaluations.iter().find(|install| {
            install.target.binding == destination.binding
                && install.point < prior.point
                && matches!(install.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == producer.point)
        }).unwrap();
        assert_eq!(
            prior_install.disposition,
            ReferenceCheckedInstallDisposition::Written
        );
        assert_eq!(prior_install.accepted_origins, origins);
        let installed = pass
            .observations
            .points
            .iter()
            .find(|snapshot| {
                snapshot.point == prior_install.point
                    && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
            })
            .unwrap();
        assert_eq!(
            stored_reference_origins(&installed.state, destination),
            origins
        );
        for retained in stored_reference_origins(current, destination) {
            let ReferenceOrigin::Fresh {
                allocation: retained_allocation,
                loan: retained_loan,
            } = &cfg.facts[retained.0]
            else {
                unreachable!("retained actual old scalar origin")
            };
            assert_eq!(retained_loan, requested_loan);
            assert_eq!(retained_loan, loan);
            assert_eq!(retained_allocation, allocation);
        }
        assert_scalar_assignment_resource_bound(function, pass, prior, current);
    }

    fn composed(layout: ScalarTargetLayout, project: bool, consumer: Consumer, defect: Defect) {
        let text = consumer_text(consumer);
        let observed = observed(text, layout, project);
        let (function, cfg, trace) = match &observed {
            ObservedScalarValidation::Program { validation, trace } => {
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:#?}",
                    validation.diagnostics
                );
                let ScalarItem::Function(function) = &validation.program.items[0] else {
                    unreachable!()
                };
                (function, &function.reference_cfg, trace)
            }
            ObservedScalarValidation::Project { validation, trace } => {
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:#?}",
                    validation.diagnostics
                );
                let ScalarItem::Function(function) = &validation.project.modules[0].items[0] else {
                    unreachable!()
                };
                (function, &function.reference_cfg, trace)
            }
        };
        assert_eq!(function.name, "run");
        let pass = trace
            .passes
            .iter()
            .rfind(|pass| {
                pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
            })
            .unwrap();
        assert_eq!(pass.owner, function.reference_cfg.producer_owner);
        assert_eq!(pass.role, ReferenceTransferTestRole::Function);
        assert!(
            matches!(&pass.owner, ProducerCfgOwnerId::Function { source: actual_source, declaration, body: ProducerFunctionBodyId::Declaration }
            if actual_source == &source() && *declaration == function.name_span)
        );
        assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
        assert_eq!(pass.cfg_at_exit.checked_bindings, cfg.checked_bindings);
        assert_eq!(pass.cfg_at_exit.checked_installs, cfg.checked_installs);
        assert_eq!(pass.cfg_at_exit.borrow_evaluations, cfg.borrow_evaluations);
        let child = pass
            .cfg_at_entry
            .initial_points
            .iter()
            .find(|point| {
                matches!(
                    point.kind,
                    CfgPointKind::Borrow {
                        request: ReferenceBorrowRequest::Through { .. }
                    }
                )
            })
            .unwrap();
        let snapshot = pass
            .observations
            .borrows
            .iter()
            .find(|snapshot| snapshot.point == child.id)
            .unwrap();
        assert_eq!(
            pass.observations
                .borrows
                .iter()
                .filter(|snapshot| snapshot.point == child.id)
                .count(),
            1
        );
        assert_eq!(snapshot.cases.len(), 1);
        let CfgPointKind::Borrow { request } = &child.kind else {
            unreachable!()
        };
        let ReferenceBorrowCase::Accepted {
            origin: child_origin,
            access: ReferenceBorrowAccess::Through { parent_origin, .. },
            loan: child_loan,
            ..
        } = &snapshot.cases[0]
        else {
            unreachable!()
        };
        let ReferenceBorrowInput::Through {
            pointer_point,
            origins,
        } = &snapshot.input
        else {
            unreachable!()
        };
        assert_eq!(
            snapshot.state.point_values[pointer_point.0].as_ref(),
            Some(origins)
        );
        assert!(origins.contains(parent_origin));
        assert_eq!(
            child_loan.parent.as_deref(),
            Some(match &cfg.facts[parent_origin.0] {
                ReferenceOrigin::Fresh { loan, .. }
                | ReferenceOrigin::BorrowedFrom { loan, .. } => loan,
                _ => unreachable!(),
            })
        );
        let mut cfg = pass.cfg_at_exit.clone();
        let mut state = snapshot.state.clone();
        let mut cases = snapshot.cases.clone();
        match defect {
            Defect::ForeignOrigin => {
                let mut fact = cfg.facts[parent_origin.0].clone();
                let ReferenceOrigin::Fresh { allocation, loan } = &mut fact else {
                    unreachable!("actual Fresh parent required for allocation-identity defect")
                };
                let parent_loan = loan.clone();
                allocation.creation_span = child.source_span.range;
                assert_eq!(loan, &parent_loan);
                let foreign = evaluated_borrow::intern(&mut cfg.facts, fact);
                assert!(!origins.contains(&foreign));
                let ReferenceBorrowCase::Accepted {
                    access: ReferenceBorrowAccess::Through { parent_origin, .. },
                    ..
                } = &mut cases[0]
                else {
                    unreachable!()
                };
                *parent_origin = foreign;
            }
            Defect::FullParent => {
                let ReferenceBorrowCase::Accepted { loan, .. } = &mut cases[0] else {
                    unreachable!()
                };
                loan.parent.as_mut().unwrap().creation_span = child.source_span.range;
            }
        }
        let mut errors = Vec::new();
        let evaluation = validate_and_apply_reference_borrow_cases(
            &cfg.producer_owner,
            &cfg.facts,
            &cfg.environments,
            &cfg.initial_points,
            &binding_holder::uses(&cfg),
            &mut state,
            child,
            request,
            snapshot.input.clone(),
            cases,
            &mut errors,
        );
        assert!(
            matches!(&errors[..], [ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin }] if source_span == &child.source_span)
        );
        assert!(
            matches!(&evaluation.cases[..], [ReferenceBorrowCase::Rejected { diagnostic: ReferenceBorrowDiagnostic::Missing { source_span, reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin }, .. }] if source_span == &child.source_span)
        );
        assert!(evaluation.output.is_empty());
        assert_eq!(evaluation.input, snapshot.input);
        let mut point = child.clone();
        publish_checked_value(&mut point, &Some(evaluation.output.clone()));
        save_checked_value(&mut state, &point);
        assert_eq!(state.point_values[child.id.0], Some(BTreeSet::new()));
        assert_eq!(point.kind, child.kind);
        match consumer {
            Consumer::Return => {
                let point = cfg.points.iter().find(|point| matches!(point.kind, CfgPointKind::ReturnOutput { checked: true, value_point, .. } if value_point == child.id)).unwrap();
                let CfgPointKind::ReturnOutput {
                    value_point,
                    output,
                    ..
                } = point.kind
                else {
                    unreachable!()
                };
                let value = checked_return_input(&state, value_point);
                assert_eq!(value, Some(BTreeSet::new()));
                let returned_backings = cfg
                    .points
                    .iter()
                    .filter(|point| {
                        matches!(
                            point.kind,
                            CfgPointKind::ReturnOutput { checked: false, .. }
                        )
                    })
                    .flat_map(|point| point.possible_backings.iter().cloned())
                    .collect::<Vec<_>>();
                apply_checked_return(
                    &mut state,
                    &cfg.facts,
                    &cfg.allocations,
                    point,
                    &value,
                    &returned_backings,
                    &mut errors,
                );
                assert!(!state.return_outputs.contains_key(&output));
                assert!(
                    matches!(errors.last().unwrap(), ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin } if source_span == &point.source_span)
                );
            }
            Consumer::Bind | Consumer::Assign | Consumer::AssignThrough => {
                let install = cfg.checked_installs.iter().find(|install| matches!(install.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == child.id)).cloned().unwrap();
                let consumer = cfg.points[install.point.0].clone();
                let actual_consumer = pass
                    .observations
                    .points
                    .iter()
                    .find(|snapshot| {
                        snapshot.point == install.point
                            && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
                    })
                    .unwrap();
                assert_eq!(
                    actual_consumer.state.point_values[child.id.0],
                    Some(BTreeSet::from([*child_origin]))
                );
                if matches!(
                    consumer.kind,
                    CfgPointKind::Assign { .. } | CfgPointKind::AssignThrough { .. }
                ) {
                    assert_eq!(
                        pass.observations
                            .points
                            .iter()
                            .filter(|snapshot| {
                                snapshot.point == consumer.id
                                    && snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
                            })
                            .count(),
                        1
                    );
                    let evaluations = pass
                        .observations
                        .points
                        .iter()
                        .filter(|snapshot| {
                            snapshot.phase == ReferenceTransferPhase::BeforeEvaluation
                        })
                        .collect::<Vec<_>>();
                    let position = evaluations
                        .iter()
                        .position(|snapshot| snapshot.point == child.id)
                        .unwrap();
                    assert!(snapshot.state == evaluations[position].state);
                    let after_borrow = pass
                        .observations
                        .points
                        .iter()
                        .find(|snapshot| {
                            snapshot.point == child.id
                                && snapshot.phase == ReferenceTransferPhase::AfterInputCleanup
                        })
                        .unwrap();
                    assert_eq!(state.bindings, actual_consumer.state.bindings);
                    assert!(state.projected == actual_consumer.state.projected);
                    assert_eq!(state.moved, actual_consumer.state.moved);
                    assert_eq!(
                        state.selected_element,
                        actual_consumer.state.selected_element
                    );
                    assert_eq!(state.index_values, actual_consumer.state.index_values);
                    assert_scalar_assignment_resource_bound(
                        function,
                        pass,
                        actual_consumer,
                        &state,
                    );
                    let ReferenceCheckedInstallInput::Evaluated { value_point } = install.input
                    else {
                        unreachable!("actual rejected RHS dependency")
                    };
                    assert_eq!(value_point, child.id);
                    match &consumer.kind {
                        CfgPointKind::Assign {
                            rhs_point, target, ..
                        } => {
                            assert_eq!(*rhs_point, child.id);
                            assert_eq!(child.successors, vec![consumer.id]);
                            assert_eq!(evaluations[position + 1].point, consumer.id);
                            assert!(after_borrow.state == actual_consumer.state);
                            assert_eq!(
                                cfg.initial_points
                                    .iter()
                                    .filter(|point| point.successors.contains(&consumer.id))
                                    .map(|point| point.id)
                                    .collect::<Vec<_>>(),
                                vec![child.id]
                            );
                            assert_eq!(child.source_span.source, consumer.source_span.source);
                            assert_retained_scalar_destination(
                                function,
                                pass,
                                actual_consumer,
                                &cfg,
                                &available_place(target),
                                &state,
                                *child_origin,
                            );
                        }
                        CfgPointKind::AssignThrough {
                            rhs_point,
                            pointer_point,
                            candidates,
                            place,
                            ..
                        } => {
                            assert_eq!(*rhs_point, child.id);
                            assert!(child.id < *pointer_point && *pointer_point < consumer.id);
                            let mut pointer = cfg.initial_points[pointer_point.0].clone();
                            assert_eq!(child.successors, vec![*pointer_point]);
                            assert_eq!(pointer.successors, vec![consumer.id]);
                            for (target, predecessor) in
                                [(*pointer_point, child.id), (consumer.id, *pointer_point)]
                            {
                                assert_eq!(
                                    cfg.initial_points
                                        .iter()
                                        .filter(|point| point.successors.contains(&target))
                                        .map(|point| point.id)
                                        .collect::<Vec<_>>(),
                                    vec![predecessor]
                                );
                            }
                            assert_eq!(evaluations[position + 1].point, *pointer_point);
                            assert_eq!(evaluations[position + 2].point, consumer.id);
                            assert_eq!(
                                evaluations
                                    .iter()
                                    .filter(|snapshot| snapshot.point == *pointer_point)
                                    .count(),
                                1
                            );
                            let pointer_entry = evaluations[position + 1];
                            let pointer_saved = pass
                                .observations
                                .points
                                .iter()
                                .find(|snapshot| {
                                    snapshot.point == *pointer_point
                                        && snapshot.phase
                                            == ReferenceTransferPhase::BeforeInputCleanup
                                })
                                .unwrap();
                            let pointer_exit = pass
                                .observations
                                .points
                                .iter()
                                .find(|snapshot| {
                                    snapshot.point == *pointer_point
                                        && snapshot.phase
                                            == ReferenceTransferPhase::AfterInputCleanup
                                })
                                .unwrap();
                            assert!(after_borrow.state == pointer_entry.state);
                            assert!(pointer_exit.state == actual_consumer.state);
                            assert_scalar_assignment_resource_bound(
                                function,
                                pass,
                                pointer_entry,
                                &pointer_saved.state,
                            );
                            assert_scalar_assignment_resource_bound(
                                function,
                                pass,
                                pointer_entry,
                                &pointer_exit.state,
                            );
                            assert_eq!(pointer.source_span.source, child.source_span.source);
                            assert_eq!(pointer.source_span.source, consumer.source_span.source);
                            let ScalarPlace::Dereference {
                                pointer: expression,
                                ..
                            } = place
                            else {
                                unreachable!()
                            };
                            assert!(
                                matches!(expression.as_ref(), ScalarExpression::Name { name, span } if name == "slot" && *span == pointer.source_span.range)
                            );
                            let CfgPointKind::Read { binding } = &pointer.kind else {
                                unreachable!("actual intervening slot Read")
                            };
                            let binding = binding.clone();
                            assert_eq!(binding.source, pointer.source_span.source);
                            let pointer_value = read_checked_binding(
                                &state,
                                &cfg.checked_bindings,
                                &pointer,
                                &binding,
                                &mut errors,
                            )
                            .unwrap();
                            assert_eq!(errors.len(), 1);
                            assert_eq!(
                                pointer_value,
                                pointer_saved.state.point_values[pointer_point.0]
                                    .clone()
                                    .unwrap()
                            );
                            publish_checked_value(&mut pointer, &Some(pointer_value.clone()));
                            save_checked_value(&mut state, &pointer);
                            assert_eq!(state.point_values[child.id.0], Some(BTreeSet::new()));
                            assert_eq!(
                                state.point_values[pointer_point.0],
                                Some(pointer_value.clone())
                            );
                            for witness in
                                [pointer_entry, pointer_saved, pointer_exit, actual_consumer]
                            {
                                assert_eq!(
                                    witness.state.point_values[child.id.0],
                                    Some(BTreeSet::from([*child_origin]))
                                );
                            }
                            assert_eq!(
                                pointer_saved.state.point_values[pointer_point.0],
                                Some(pointer_value.clone())
                            );
                            assert_eq!(
                                pointer_exit.state.point_values[pointer_point.0],
                                Some(pointer_value.clone())
                            );
                            let consumer_saved = pass
                                .observations
                                .points
                                .iter()
                                .find(|snapshot| {
                                    snapshot.point == consumer.id
                                        && snapshot.phase
                                            == ReferenceTransferPhase::BeforeInputCleanup
                                })
                                .unwrap();
                            let consumer_exit = pass
                                .observations
                                .points
                                .iter()
                                .find(|snapshot| {
                                    snapshot.point == consumer.id
                                        && snapshot.phase
                                            == ReferenceTransferPhase::AfterInputCleanup
                                })
                                .unwrap();
                            assert_eq!(
                                consumer_saved.state.point_values[child.id.0],
                                Some(BTreeSet::from([*child_origin]))
                            );
                            assert_eq!(
                                consumer_saved.state.point_values[pointer_point.0],
                                Some(pointer_value.clone())
                            );
                            assert!(consumer_exit.state.point_values[child.id.0].is_none());
                            assert!(consumer_exit.state.point_values[pointer_point.0].is_none());
                            assert_eq!(candidates.len(), 1);
                            let pointer_origins = actual_consumer.state.point_values
                                [pointer_point.0]
                                .as_ref()
                                .unwrap();
                            assert_eq!(
                                state.point_values[pointer_point.0].as_ref(),
                                Some(pointer_origins)
                            );
                            assert_eq!(pointer_origins, &BTreeSet::from([candidates[0].origin]));
                            assert!(
                                matches!(&cfg.checked_bindings[&binding].place, ScalarPlace::Name { name, .. } if name == "slot")
                            );
                            assert_eq!(
                                state.bindings[&binding],
                                actual_consumer.state.bindings[&binding]
                            );
                            assert_eq!(pointer_value, state.bindings[&binding]);
                            let slot_origin = *pointer_value.first().unwrap();
                            let ReferenceOrigin::Fresh { loan, allocation } =
                                &pass.cfg_at_exit.facts[slot_origin.0]
                            else {
                                unreachable!("actual checked destination alias")
                            };
                            assert_eq!(loan, &candidates[0].loan);
                            assert_eq!(loan.origin_place.binding, candidates[0].place.binding);
                            assert_eq!(allocation.source, loan.creation_source);
                            assert_eq!(allocation.creation_span, loan.creation_span);
                            let slot_producer = pass.observations.borrows.iter().find(|snapshot| {
                                snapshot.cases.iter().any(|case| matches!(case, ReferenceBorrowCase::Accepted { origin, .. } if *origin == slot_origin))
                            }).unwrap();
                            assert!(slot_producer.point < child.id);
                            let CfgPointKind::Borrow {
                                request:
                                    ReferenceBorrowRequest::DirectStorage {
                                        origin,
                                        loan: requested,
                                        place: requested_place,
                                        ..
                                    },
                            } = &cfg.initial_points[slot_producer.point.0].kind
                            else {
                                unreachable!()
                            };
                            assert_eq!(*origin, slot_origin);
                            assert_eq!(requested, loan);
                            assert_eq!(requested_place, &loan.origin_place);
                            assert_eq!(
                                allocation.declaration_span,
                                loan.origin_place.declaration_span
                            );
                            assert_eq!(allocation.function_span, loan.origin_place.function_span);
                            let mut derived_place = loan.origin_place.clone();
                            derived_place.place = place.clone();
                            collect_reference_projections(
                                place,
                                None,
                                &mut derived_place.projections,
                            );
                            let derived_candidate = InvalidationCandidate {
                                origin: slot_origin,
                                place: derived_place,
                                loan: loan.clone(),
                            };
                            assert_eq!(candidates, &vec![derived_candidate]);
                            assert_scalar_assignment_resource_bound(
                                function,
                                pass,
                                actual_consumer,
                                &state,
                            );
                            assert_retained_scalar_destination(
                                function,
                                pass,
                                actual_consumer,
                                &cfg,
                                &available_place(&candidates[0].place),
                                &state,
                                *child_origin,
                            );
                        }
                        _ => unreachable!(),
                    }
                }
                let input = checked_install_input(&state, &Some(install.clone()));
                assert!(input.is_empty());
                let mut installations = Vec::new();
                let frozen_context = assignment_frozen_flow::committed(
                    &state,
                    &cfg.evaluated_values.destinations,
                    &cfg.evaluated_values.commits,
                    consumer.id,
                );
                let registered = cfg
                    .evaluated_values
                    .commits
                    .iter()
                    .filter(|commit| commit.point == consumer.id)
                    .collect::<Vec<_>>();
                match &frozen_context {
                    Some(rows) => {
                        let [commit] = registered.as_slice() else {
                            panic!("exact replay commit incidence")
                        };
                        assert!(matches!(
                            consumer.kind,
                            CfgPointKind::Assign { .. } | CfgPointKind::AssignThrough { .. }
                        ));
                        assert_eq!(commit.point, consumer.id);
                        assert_eq!(commit.destination.operation.producer, cfg.producer_owner);
                        assert_eq!(
                            consumer.source_span.source,
                            commit.destination.operation.occurrence.source_span.source
                        );
                        let destinations = cfg
                            .evaluated_values
                            .destinations
                            .iter()
                            .filter(|destination| destination.id == commit.destination)
                            .collect::<Vec<_>>();
                        let [destination] = destinations.as_slice() else {
                            panic!("exact replay destination incidence")
                        };
                        assert_eq!(destination.id.operation.producer, cfg.producer_owner);
                        assert_eq!(destination.source_span.source, consumer.source_span.source);
                        assert_eq!(actual_consumer.point, consumer.id);
                        assert_eq!(
                            actual_consumer.phase,
                            ReferenceTransferPhase::BeforeEvaluation
                        );
                        assert!(rows.iter().all(|row| row.destination == destination.id
                            && row.point == destination.point));
                        assert_eq!(rows, &state.assignment_destinations[&destination.point]);
                        assert_eq!(
                            rows,
                            &actual_consumer.state.assignment_destinations[&destination.point]
                        );
                    }
                    None => {
                        assert!(registered.is_empty());
                        assert!(matches!(
                            consumer.kind,
                            CfgPointKind::Bind { .. }
                                | CfgPointKind::Assign { .. }
                                | CfgPointKind::AssignThrough { .. }
                        ));
                    }
                }
                let value = apply_checked_install(
                    &mut state,
                    &cfg.producer_owner,
                    &cfg.checked_bindings,
                    &consumer,
                    frozen_context.as_deref(),
                    &Some(install),
                    &input,
                    &mut installations,
                    &mut errors,
                );
                assert_eq!(value, Some(BTreeSet::new()));
                assert_eq!(installations.len(), 1);
                let installed = &installations[0];
                assert!(installed.accepted_origins.is_empty());
                assert!(!stored_reference_origins(&state, &installed.target).contains(child_origin));
                match consumer.kind {
                    CfgPointKind::Bind { .. } => {
                        assert_eq!(
                            installed.disposition,
                            ReferenceCheckedInstallDisposition::Uninitialized
                        );
                        assert!(state.uninitialized.contains(&installed.target.binding));
                    }
                    CfgPointKind::Assign { .. } | CfgPointKind::AssignThrough { .. } => {
                        assert_eq!(
                            installed.disposition,
                            ReferenceCheckedInstallDisposition::Retained
                        );
                        assert!(!installed.previous_origins.is_empty());
                        assert_eq!(
                            stored_reference_origins(&state, &installed.target),
                            installed.previous_origins
                        );
                        assert_eq!(
                            stored_reference_origins(&actual_consumer.state, &installed.target),
                            installed.previous_origins
                        );
                        for origin in &installed.previous_origins {
                            assert_eq!(cfg.facts[origin.0], pass.cfg_at_exit.facts[origin.0]);
                        }
                        assert_retained_scalar_destination(
                            function,
                            pass,
                            actual_consumer,
                            &cfg,
                            &installed.target,
                            &state,
                            *child_origin,
                        );
                    }
                    _ => unreachable!(),
                }
                let read = cfg.points.iter().find(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding == &installed.target.binding) && point.id > consumer.id).unwrap();
                let value = read_checked_binding(
                    &state,
                    &cfg.checked_bindings,
                    read,
                    &installed.target.binding,
                    &mut errors,
                )
                .unwrap();
                assert!(!value.contains(child_origin));
                if installed.disposition == ReferenceCheckedInstallDisposition::Retained {
                    assert_eq!(value, installed.previous_origins);
                    assert_eq!(read.source_span.source, installed.target.source);
                    assert_retained_scalar_destination(
                        function,
                        pass,
                        actual_consumer,
                        &cfg,
                        &installed.target,
                        &state,
                        *child_origin,
                    );
                } else {
                    assert!(value.is_empty());
                }
            }
        }
        assert_eq!(cfg.initial_points[child.id.0].kind, child.kind);
        let restored: ScalarReferenceCfg =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, restored);
    }

    macro_rules! composed_test {
        ($name:ident, $layout:expr, $project:expr, $consumer:ident, $defect:ident) => {
            #[test]
            fn $name() {
                composed($layout, $project, Consumer::$consumer, Defect::$defect);
            }
        };
    }

    fn missing_and_nonchecked(layout: ScalarTargetLayout, project: bool) {
        for consumer in [
            Consumer::Bind,
            Consumer::Assign,
            Consumer::AssignThrough,
            Consumer::Return,
        ] {
            let observed = observed(consumer_text(consumer), layout, project);
            let (items, trace) = match &observed {
                ObservedScalarValidation::Program { validation, trace } => {
                    (&validation.program.items, trace)
                }
                ObservedScalarValidation::Project { validation, trace } => {
                    (&validation.project.modules[0].items, trace)
                }
            };
            let ScalarItem::Function(function) = &items[0] else {
                unreachable!()
            };
            let pass = trace
                .passes
                .iter()
                .rfind(|pass| {
                    pass.owner == function.reference_cfg.producer_owner
                        && pass.role == ReferenceTransferTestRole::Function
                })
                .unwrap();
            assert_eq!(pass.cfg_at_exit.facts, function.reference_cfg.facts);
            let child = pass
                .cfg_at_entry
                .initial_points
                .iter()
                .find(|point| {
                    matches!(
                        point.kind,
                        CfgPointKind::Borrow {
                            request: ReferenceBorrowRequest::Through { .. }
                        }
                    )
                })
                .unwrap();
            let original_output = pass
                .cfg_at_exit
                .borrow_evaluations
                .iter()
                .find(|evaluation| evaluation.point == child.id)
                .unwrap()
                .output
                .clone();
            let missing = pass
                .cfg_at_entry
                .initial_points
                .iter()
                .find(|point| matches!(point.kind, CfgPointKind::Entry))
                .unwrap()
                .id;
            let nonchecked = pass
                .cfg_at_entry
                .initial_points
                .iter()
                .find(|point| {
                    matches!(
                        point.kind,
                        CfgPointKind::Evaluate(ScalarExpression::Integer { .. })
                    )
                })
                .unwrap()
                .id;
            assert_ne!(missing, nonchecked);
            for producer in [missing, nonchecked] {
                let mut cfg = pass.cfg_at_exit.clone();
                let CfgPointKind::Borrow {
                    request: ReferenceBorrowRequest::Through { pointer_point, .. },
                } = &mut cfg.initial_points[child.id.0].kind
                else {
                    unreachable!()
                };
                *pointer_point = producer;
                cfg.points = cfg.initial_points.clone();
                let mut trace = ReferenceValidationTestTrace::new();
                let errors = transfer_reference_facts(
                    &mut cfg,
                    &pass.context.deferred_policies,
                    &pass.context.contracts,
                    &pass.context.literal_contracts,
                    &pass.context.call_exits,
                    &pass.context.capture_input,
                    ReferenceTransferTestRole::Function,
                    &mut trace,
                );
                assert!(errors.iter().any(|error| matches!(error, ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin } if source_span == &child.source_span)));
                let evaluation = cfg
                    .borrow_evaluations
                    .iter()
                    .find(|evaluation| evaluation.point == child.id)
                    .unwrap();
                assert_eq!(
                    evaluation.input,
                    ReferenceBorrowInput::MissingProducer {
                        pointer_point: producer
                    }
                );
                assert!(evaluation.cases.is_empty());
                assert!(evaluation.output.is_empty());
                assert_eq!(
                    cfg.points[child.id.0].possible_origins,
                    Some(BTreeSet::new())
                );
                assert!(cfg
                    .checked_install_evaluations
                    .iter()
                    .all(|evaluation| evaluation.accepted_origins.is_disjoint(&original_output)));
                for evaluation in cfg.checked_install_evaluations.iter().filter(|evaluation| matches!(evaluation.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == child.id)) {
                    assert!(evaluation.accepted_origins.is_empty());
                    match consumer {
                        Consumer::Bind => assert_eq!(evaluation.disposition, ReferenceCheckedInstallDisposition::Uninitialized),
                        Consumer::Assign | Consumer::AssignThrough => {
                            assert_eq!(evaluation.disposition, ReferenceCheckedInstallDisposition::Retained);
                            assert!(!evaluation.previous_origins.is_empty());
                            assert!(cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Read { binding } if binding == &evaluation.target.binding) && point.id > evaluation.point && point.possible_origins.as_ref() == Some(&evaluation.previous_origins)));
                        }
                        Consumer::Return => unreachable!(),
                    }
                }
                for point in cfg.points.iter().filter(|point| {
                    matches!(point.kind, CfgPointKind::ReturnOutput { checked: true, .. })
                }) {
                    assert!(point
                        .possible_origins
                        .as_ref()
                        .unwrap()
                        .is_disjoint(&original_output));
                }
                let restored: ScalarReferenceCfg =
                    serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
                assert_eq!(cfg, restored);
            }
        }
    }

    macro_rules! producer_test {
        ($name:ident, $layout:expr, $project:expr) => {
            #[test]
            fn $name() {
                missing_and_nonchecked($layout, $project);
            }
        };
    }
    producer_test!(
        d3_reborrow_state_evaluated_borrow_producers_native_program,
        ScalarTargetLayout::NATIVE64,
        false
    );
    producer_test!(
        d3_reborrow_state_evaluated_borrow_producers_native_project,
        ScalarTargetLayout::NATIVE64,
        true
    );
    producer_test!(
        d3_reborrow_state_evaluated_borrow_producers_wasm_program,
        ScalarTargetLayout::WASM32,
        false
    );
    producer_test!(
        d3_reborrow_state_evaluated_borrow_producers_wasm_project,
        ScalarTargetLayout::WASM32,
        true
    );

    fn import_correspondence(layout: ScalarTargetLayout, project: bool, derived: bool) {
        let observed = if project {
            let main_text = "%%start\nleft = namespace app \"src/left.w\"; right = namespace app \"src/right.w\"; unit() run = fn { u32 first = 1; u32 second = 2; *u32 first_parent = left.relay(&first); *u32 second_parent = right.relay(&second); *u32 first_child = &(*first_parent); *u32 second_child = &(*second_parent); *first_child; *second_child; };\n%%end";
            let relay_text = if derived {
                "%%start\n*u32(*u32) relay = fn(input) { &(*input) };\n%%end"
            } else {
                "%%start\n*u32(*u32) relay = fn(input) { input };\n%%end"
            };
            let left_source = SourceIdentity {
                path: "src/left.w".to_owned(),
                ..source()
            };
            let right_source = SourceIdentity {
                path: "src/right.w".to_owned(),
                ..source()
            };
            let mut modules = Vec::new();
            for (identity, text, namespaces) in [
                (
                    source(),
                    main_text,
                    vec![
                        ScalarNamespaceBinding {
                            binding: "left".to_owned(),
                            target: left_source.clone(),
                            span: ByteSpan::new(8, 12),
                        },
                        ScalarNamespaceBinding {
                            binding: "right".to_owned(),
                            target: right_source.clone(),
                            span: ByteSpan::new(41, 46),
                        },
                    ],
                ),
                (left_source, relay_text, Vec::new()),
                (right_source, relay_text, Vec::new()),
            ] {
                let parsed = parse_source(identity, text.to_owned(), &[]);
                assert!(parsed.diagnostics.is_empty());
                let ordinary = derive_scalar_program_with_layout(&parsed.result, layout);
                let (validation, _) = derive_scalar_program_observed_test(&parsed.result, layout);
                assert_eq!(validation, ordinary);
                modules.push(ScalarModule::from_program(validation.program, namespaces));
            }
            let project = ScalarProject::new(modules, vec![source()]);
            let ordinary = validate_scalar_project(project.clone());
            let (validation, trace) = validate_scalar_project_observed_test(project);
            assert_eq!(validation, ordinary);
            assert!(
                validation.diagnostics.is_empty(),
                "{:#?}",
                validation.diagnostics
            );
            let relay_cfgs = [1, 2].map(|index| {
                let ScalarItem::Function(function) = &validation.project.modules[index].items[0]
                else {
                    unreachable!()
                };
                &function.reference_cfg
            });
            assert_ne!(relay_cfgs[0].producer_owner, relay_cfgs[1].producer_owner);
            assert_eq!(
                relay_cfgs[0]
                    .initial_points
                    .iter()
                    .map(|point| (point.id, point.source_span.range))
                    .collect::<Vec<_>>(),
                relay_cfgs[1]
                    .initial_points
                    .iter()
                    .map(|point| (point.id, point.source_span.range))
                    .collect::<Vec<_>>()
            );
            let roots = relay_cfgs.map(|cfg| {
                cfg.facts
                    .iter()
                    .find_map(|origin| match origin {
                        ReferenceOrigin::BorrowedFrom { parameter: 0, loan } => Some(loan),
                        _ => None,
                    })
                    .unwrap()
            });
            assert_ne!(roots[0].creation_source, roots[1].creation_source);
            assert_ne!(
                roots[0].origin_place.binding.source,
                roots[1].origin_place.binding.source
            );
            assert_eq!(roots[0].creation_span, roots[1].creation_span);
            assert_eq!(
                roots[0].origin_place.binding.declaration_span,
                roots[1].origin_place.binding.declaration_span
            );
            assert_eq!(
                roots[0].origin_place.binding.kind,
                ReferenceBindingKind::BorrowedReferent { parameter: 0 }
            );
            assert_eq!(
                roots[1].origin_place.binding.kind,
                ReferenceBindingKind::BorrowedReferent { parameter: 0 }
            );
            for cfg in relay_cfgs {
                let pass = trace
                    .passes
                    .iter()
                    .rfind(|pass| {
                        pass.owner == cfg.producer_owner
                            && pass.role == ReferenceTransferTestRole::Function
                    })
                    .unwrap();
                assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
                assert_eq!(pass.cfg_at_exit.checked_bindings, cfg.checked_bindings);
                assert!(!pass.context.contracts.is_empty());
                if derived {
                    let evaluation = cfg
                        .borrow_evaluations
                        .iter()
                        .find(|evaluation| {
                            matches!(
                                cfg.points[evaluation.point.0].kind,
                                CfgPointKind::Borrow {
                                    request: ReferenceBorrowRequest::Through { .. }
                                }
                            )
                        })
                        .unwrap();
                    assert_eq!(evaluation.output.len(), 1);
                    let ReferenceOrigin::BorrowedFrom { parameter, loan } =
                        &cfg.facts[evaluation.output.first().unwrap().0]
                    else {
                        unreachable!()
                    };
                    assert_eq!(*parameter, 0);
                    assert!(loan.parent.is_some());
                    assert_eq!(
                        loan.creation_source,
                        cfg.points[evaluation.point.0].source_span.source
                    );
                    assert!(cfg.points.iter().any(|point| matches!(
                        point.kind,
                        CfgPointKind::ReturnOutput { checked: true, .. }
                    ) && point.possible_origins.as_ref()
                        == Some(&evaluation.output)));
                }
            }
            ObservedScalarValidation::Project { validation, trace }
        } else {
            let text = if derived {
                "%%start\n*u32(*u32) left = fn(input) { &(*input) }; *u32(*u32) right = fn(input) { &(*input) }; unit() run = fn { u32 first = 1; u32 second = 2; *u32 first_parent = left(&first); *u32 second_parent = right(&second); *u32 first_child = &(*first_parent); *u32 second_child = &(*second_parent); *first_child; *second_child; };\n%%end"
            } else {
                "%%start\n*u32(*u32) left = fn(input) { input }; *u32(*u32) right = fn(input) { input }; unit() run = fn { u32 first = 1; u32 second = 2; *u32 first_parent = left(&first); *u32 second_parent = right(&second); *u32 first_child = &(*first_parent); *u32 second_child = &(*second_parent); *first_child; *second_child; };\n%%end"
            };
            observed(text, layout, false)
        };
        let (items, trace) = match &observed {
            ObservedScalarValidation::Program { validation, trace } => {
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:#?}",
                    validation.diagnostics
                );
                (&validation.program.items, trace)
            }
            ObservedScalarValidation::Project { validation, trace } => {
                (&validation.project.modules[0].items, trace)
            }
        };
        let cfg = items
            .iter()
            .find_map(|item| match item {
                ScalarItem::Function(function) if function.name == "run" => {
                    Some(&function.reference_cfg)
                }
                _ => None,
            })
            .unwrap();
        let pass = trace
            .passes
            .iter()
            .rfind(|pass| {
                pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
            })
            .unwrap();
        assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
        assert_eq!(pass.cfg_at_exit.checked_bindings, cfg.checked_bindings);
        assert_eq!(pass.cfg_at_exit.borrow_evaluations, cfg.borrow_evaluations);
        assert!(!pass.context.contracts.is_empty());
        let calls = cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::Call { .. }))
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert_ne!(calls[0].possible_origins, calls[1].possible_origins);
        if derived {
            for call in &calls {
                let CfgPointKind::Call {
                    argument_points, ..
                } = &call.kind
                else {
                    unreachable!()
                };
                assert_eq!(
                    call.possible_origins,
                    cfg.points[argument_points[0].0].possible_origins
                );
                for origin in call.possible_origins.as_ref().unwrap() {
                    let ReferenceOrigin::Fresh { loan, .. } = &cfg.facts[origin.0] else {
                        unreachable!()
                    };
                    assert!(loan.parent.is_none());
                    assert_eq!(loan.creation_source, source());
                }
            }
        }
        if project {
            for (call, path) in calls.iter().zip(["src/left.w", "src/right.w"]) {
                assert!(
                    matches!(&call.kind, CfgPointKind::Call { target: ResolvedCallTarget::ModuleCallable { source: callee_source, .. }, .. } if callee_source.path == path)
                );
            }
        }
        let children = pass
            .observations
            .borrows
            .iter()
            .filter(|snapshot| matches!(snapshot.input, ReferenceBorrowInput::Through { .. }))
            .collect::<Vec<_>>();
        assert_eq!(children.len(), 2);
        for (index, snapshot) in children.iter().enumerate() {
            let ReferenceBorrowInput::Through {
                pointer_point,
                origins,
            } = &snapshot.input
            else {
                unreachable!()
            };
            assert_eq!(
                snapshot.state.point_values[pointer_point.0].as_ref(),
                Some(origins)
            );
            assert_eq!(calls[index].possible_origins.as_ref(), Some(origins));
            assert!(origins.is_disjoint(calls[1 - index].possible_origins.as_ref().unwrap()));
            let ReferenceBorrowCase::Accepted {
                origin,
                loan,
                access: ReferenceBorrowAccess::Through { parent_origin, .. },
                ..
            } = &snapshot.cases[0]
            else {
                unreachable!()
            };
            assert!(origins.contains(parent_origin));
            let ReferenceOrigin::Fresh {
                allocation,
                loan: parent,
            } = &cfg.facts[parent_origin.0]
            else {
                unreachable!()
            };
            assert_eq!(parent.creation_source, source());
            assert_eq!(loan.creation_source, source());
            assert_eq!(loan.parent.as_deref(), Some(parent));
            assert!(
                matches!(&cfg.facts[origin.0], ReferenceOrigin::Fresh { allocation: child_allocation, loan: child } if child_allocation == allocation && child == loan)
            );
            assert!(cfg.checked_install_evaluations.iter().any(|install| matches!(install.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == snapshot.point) && install.disposition == ReferenceCheckedInstallDisposition::Written && install.accepted_origins == BTreeSet::from([*origin])));
            if let ObservedScalarValidation::Project { validation, .. } = &observed {
                let ScalarItem::Function(other) = &validation.project.modules[2 - index].items[0]
                else {
                    unreachable!()
                };
                let root = other.reference_cfg.facts.iter().find(|origin| matches!(origin, ReferenceOrigin::BorrowedFrom { parameter: 0, loan } if loan.parent.is_none())).unwrap().clone();
                let mut cloned_cfg = cfg.clone();
                let foreign = evaluated_borrow::intern(&mut cloned_cfg.facts, root);
                assert!(!origins.contains(&foreign));
                let mut cases = snapshot.cases.clone();
                let ReferenceBorrowCase::Accepted {
                    access: ReferenceBorrowAccess::Through { parent_origin, .. },
                    ..
                } = &mut cases[0]
                else {
                    unreachable!()
                };
                *parent_origin = foreign;
                let point = &cloned_cfg.initial_points[snapshot.point.0];
                let CfgPointKind::Borrow { request } = &point.kind else {
                    unreachable!()
                };
                let mut errors = Vec::new();
                let evaluation = validate_and_apply_reference_borrow_cases(
                    &cloned_cfg.producer_owner,
                    &cloned_cfg.facts,
                    &cloned_cfg.environments,
                    &cloned_cfg.initial_points,
                    &binding_holder::uses(&cloned_cfg),
                    &mut snapshot.state.clone(),
                    point,
                    request,
                    snapshot.input.clone(),
                    cases,
                    &mut errors,
                );
                assert!(evaluation.output.is_empty());
                assert!(matches!(
                    &evaluation.cases[..],
                    [ReferenceBorrowCase::Rejected { .. }]
                ));
                assert!(
                    matches!(&errors[..], [ReferenceAnalysisError::MissingContext { source_span, reason: ReferenceAnalysisErrorReason::MissingRequiredDereferenceOrigin }] if source_span == &point.source_span)
                );
            }
        }
        let restored: ScalarReferenceCfg =
            serde_json::from_str(&serde_json::to_string(cfg).unwrap()).unwrap();
        assert_eq!(cfg, &restored);
    }

    macro_rules! import_test {
        ($name:ident, $layout:expr, $project:expr) => {
            #[test]
            fn $name() {
                import_correspondence($layout, $project, false);
            }
        };
    }
    import_test!(
        d3_reborrow_state_evaluated_borrow_import_trace_native_program,
        ScalarTargetLayout::NATIVE64,
        false
    );
    import_test!(
        d3_reborrow_state_evaluated_borrow_import_trace_native_project,
        ScalarTargetLayout::NATIVE64,
        true
    );
    import_test!(
        d3_reborrow_state_evaluated_borrow_import_trace_wasm_program,
        ScalarTargetLayout::WASM32,
        false
    );
    import_test!(
        d3_reborrow_state_evaluated_borrow_import_trace_wasm_project,
        ScalarTargetLayout::WASM32,
        true
    );

    macro_rules! derived_dependency_test {
        ($name:ident, $layout:expr, $project:expr) => {
            #[test]
            fn $name() {
                import_correspondence($layout, $project, true);
            }
        };
    }
    derived_dependency_test!(
        d3_reborrow_state_evaluated_borrow_derived_return_dependency_native_program,
        ScalarTargetLayout::NATIVE64,
        false
    );
    derived_dependency_test!(
        d3_reborrow_state_evaluated_borrow_derived_return_dependency_native_project,
        ScalarTargetLayout::NATIVE64,
        true
    );
    derived_dependency_test!(
        d3_reborrow_state_evaluated_borrow_derived_return_dependency_wasm_program,
        ScalarTargetLayout::WASM32,
        false
    );
    derived_dependency_test!(
        d3_reborrow_state_evaluated_borrow_derived_return_dependency_wasm_project,
        ScalarTargetLayout::WASM32,
        true
    );

    fn retained_inputs(layout: ScalarTargetLayout, project: bool) {
        for text in [
            "%%start\nunit() run = fn { u32[2] items = [1, 2]; u64 index = 0; *!(u32[2]) parent = &!items; *!u32 child = &!(*parent)[index]; *child = 3; (*parent)[0] = 4; };\n%%end",
            "%%start\nunit((u8())[]) run = fn(items) { u64 index = 0; index; *(u8()) borrowed = &items[index]; borrowed; items; };\n%%end",
            "%%start\nunit((u8())[]) run = fn(items) { *(u8()) borrowed = &items[0]; borrowed; items; };\n%%end",
        ] {
            let observed = observed(text, layout, project);
            let (items, diagnostics, trace) = match &observed {
                ObservedScalarValidation::Program { validation, trace } => (&validation.program.items, &validation.diagnostics, trace),
                ObservedScalarValidation::Project { validation, trace } => (&validation.project.modules[0].items, &validation.diagnostics, trace),
            };
            assert!(diagnostics.is_empty(), "{diagnostics:#?}");
            let ScalarItem::Function(function) = &items[0] else { unreachable!() };
            let cfg = &function.reference_cfg;
            let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function).unwrap();
            assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
            assert_eq!(pass.cfg_at_exit.borrow_evaluations, cfg.borrow_evaluations);
            let borrow = pass.observations.borrows.iter().find(|snapshot| {
                let CfgPointKind::Borrow { request } = &pass.cfg_at_entry.initial_points[snapshot.point.0].kind else { unreachable!() };
                !evaluated_borrow::indices(request).is_empty()
            }).unwrap();
            let point = &pass.cfg_at_entry.initial_points[borrow.point.0];
            let CfgPointKind::Borrow { request } = &point.kind else { unreachable!() };
            let indices = evaluated_borrow::indices(request);
            assert_eq!(indices.len(), 1);
            let index = &indices[0];
            let ReferenceBorrowCase::Accepted { loan, .. } = &borrow.cases[0] else { unreachable!() };
            let ReferencePlaceProjection::Index(index_expression) = &loan.origin_place.projections[index.projection] else { unreachable!() };
            let index_source = SourceSpan::new(loan.origin_place.source.clone(), expression_span(index_expression));
            assert!(index.value_point < borrow.point);
            assert_eq!(pass.cfg_at_entry.initial_points[index.value_point.0].source_span, index_source);
            if let ReferenceBorrowRequest::Through { pointer_point, .. } = request {
                assert!(*pointer_point < index.value_point);
                let ReferenceBorrowInput::Through { origins, .. } = &borrow.input else { unreachable!() };
                assert_eq!(borrow.state.point_values[pointer_point.0].as_ref(), Some(origins));
            }
            let before = pass.observations.points.iter().find(|snapshot| snapshot.point == borrow.point && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).unwrap();
            let after = pass.observations.points.iter().find(|snapshot| snapshot.point == borrow.point && snapshot.phase == ReferenceTransferPhase::AfterInputCleanup).unwrap();
            if let ReferenceBorrowInput::Through { pointer_point, origins } = &borrow.input {
                assert_eq!(before.state.point_values[pointer_point.0].as_ref(), Some(origins));
                assert!(after.state.point_values[pointer_point.0].is_none());
            }
            let known = match &pass.cfg_at_entry.initial_points[index.value_point.0].kind {
                CfgPointKind::Read { binding } => {
                    assert_eq!(borrow.state.index_values[binding], BigInt::from(0));
                    Some(binding)
                }
                CfgPointKind::Evaluate(ScalarExpression::Integer { value, .. }) => {
                    assert_eq!(value, &BigInt::from(0));
                    None
                }
                _ => unreachable!(),
            };
            let families = before.state.backing_bindings.values().flatten().flat_map(|backing| &backing.callable_family).collect::<Vec<_>>();
            if function.parameter_spans.is_empty() {
                assert!(families.is_empty());
            } else {
                assert!(!families.is_empty());
                let representatives = families.iter().flat_map(|family| &family.partitions).flat_map(|partition| &partition.representatives).collect::<Vec<_>>();
                assert!(representatives.iter().any(|rep| rep.key == CallableIndexKeyFact::Constant(BigInt::from(0))));
                if let Some(binding) = known {
                    assert!(representatives.iter().any(|rep| matches!(&rep.key, CallableIndexKeyFact::LivePlace(place) if &place.binding == binding)));
                }
                let selected = representatives.iter().find(|rep| matches!(&rep.key, CallableIndexKeyFact::Evaluated { point, occurrence, .. } if *point == index.value_point && occurrence == &index_source)).unwrap();
                assert_eq!(selected.anchors.len(), 1);
                assert!(matches!(&selected.anchors[0], CallableIndexAnchorFact::ActiveLoan { loan: actual, origin, backing, owner, length } if actual == loan && origin == &point.source_span && before.state.backing_bindings.values().flatten().any(|fact| &fact.value == backing && &fact.owner == owner && &fact.length == length)));
            }
            let restored: ScalarReferenceCfg = serde_json::from_str(&serde_json::to_string(cfg).unwrap()).unwrap();
            assert_eq!(cfg, &restored);
        }
    }

    macro_rules! retention_test {
        ($name:ident, $layout:expr, $project:expr) => {
            #[test]
            fn $name() {
                retained_inputs($layout, $project);
            }
        };
    }
    retention_test!(
        d3_reborrow_state_evaluated_borrow_retention_native_program,
        ScalarTargetLayout::NATIVE64,
        false
    );
    retention_test!(
        d3_reborrow_state_evaluated_borrow_retention_native_project,
        ScalarTargetLayout::NATIVE64,
        true
    );
    retention_test!(
        d3_reborrow_state_evaluated_borrow_retention_wasm_program,
        ScalarTargetLayout::WASM32,
        false
    );
    retention_test!(
        d3_reborrow_state_evaluated_borrow_retention_wasm_project,
        ScalarTargetLayout::WASM32,
        true
    );

    fn rejection_and_greatest(layout: ScalarTargetLayout, project: bool) {
        for text in [
            "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *u32 sibling = &(*parent); unit() read = fn { *sibling; }; *!u32 child = &!(*parent); *child = 2; read(); };\n%%end",
            "%%start\nunit() run = fn { u32 item = 1; *!u32 parent = &!item; *!u32 moved = parent; *(*!u32) child = &parent; child; *moved = 2; };\n%%end",
        ] {
            let observed = observed(text, layout, project);
            let (items, diagnostics, trace) = match &observed {
                ObservedScalarValidation::Program { validation, trace } => (&validation.program.items, &validation.diagnostics, trace),
                ObservedScalarValidation::Project { validation, trace } => (&validation.project.modules[0].items, &validation.diagnostics, trace),
            };
            assert!(!diagnostics.is_empty());
            let ScalarItem::Function(function) = &items[0] else { unreachable!() };
            let cfg = &function.reference_cfg;
            let pass = trace.passes.iter().rfind(|pass| pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function).expect("actual latest rejected transfer context");
            assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
            assert_eq!(pass.cfg_at_exit.borrow_evaluations, cfg.borrow_evaluations);
            let rejected = pass.cfg_at_exit.borrow_evaluations.iter().find(|evaluation| evaluation.output.is_empty() && !evaluation.cases.is_empty()).unwrap();
            let point = &pass.cfg_at_exit.points[rejected.point.0];
            let snapshot = pass.observations.borrows.iter().find(|snapshot| snapshot.point == rejected.point).unwrap();
            assert!(snapshot.cases.iter().all(|case| matches!(case, ReferenceBorrowCase::Accepted { .. })));
            assert!(rejected.cases.iter().all(|case| matches!(case, ReferenceBorrowCase::Rejected { .. })));
            assert!(pass.errors.iter().any(|error| error.source_span() == &point.source_span && matches!(error, ReferenceAnalysisError::ConflictingPlace { .. })));
            assert!(pass.errors.iter().any(|error| diagnostics.contains(&error.diagnostic())));
            let install = cfg.checked_install_evaluations.iter().find(|install| matches!(install.input, ReferenceCheckedInstallInput::Evaluated { value_point } if value_point == rejected.point)).unwrap();
            assert_eq!(install.disposition, ReferenceCheckedInstallDisposition::Uninitialized);
            assert!(install.accepted_origins.is_empty());
            let after = pass.observations.points.iter().find(|snapshot| snapshot.point == install.point && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup).unwrap();
            assert!(after.state.uninitialized.contains(&install.target.binding));
            assert!(stored_reference_origins(&after.state, &install.target).is_empty());
        }
        let text = "%%start\nunit() run = fn { u32[2] items = [1, 2]; *!(u32[2]) parent = &!items; *!u32 first = &!(*parent)[0]; *first = 3; *!u32 second = &!(*parent)[1]; *second = 4; (*parent)[0] = 5; };\n%%end";
        let observed = observed(text, layout, project);
        let (items, trace) = match &observed {
            ObservedScalarValidation::Program { validation, trace } => {
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:#?}",
                    validation.diagnostics
                );
                (&validation.program.items, trace)
            }
            ObservedScalarValidation::Project { validation, trace } => {
                assert!(
                    validation.diagnostics.is_empty(),
                    "{:#?}",
                    validation.diagnostics
                );
                (&validation.project.modules[0].items, trace)
            }
        };
        let ScalarItem::Function(function) = &items[0] else {
            unreachable!()
        };
        let cfg = &function.reference_cfg;
        let pass = trace
            .passes
            .iter()
            .rfind(|pass| {
                pass.owner == cfg.producer_owner && pass.role == ReferenceTransferTestRole::Function
            })
            .unwrap();
        assert_eq!(pass.cfg_at_exit.facts, cfg.facts);
        let requests = cfg
            .initial_points
            .iter()
            .filter_map(|point| match point.kind {
                CfgPointKind::Borrow {
                    request: ReferenceBorrowRequest::Through { pointer_point, .. },
                } => Some((point.id, pointer_point)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 2);
        let mut cfg = cfg.clone();
        let CfgPointKind::Borrow {
            request: ReferenceBorrowRequest::Through { pointer_point, .. },
        } = &mut cfg.initial_points[requests[1].0 .0].kind
        else {
            unreachable!()
        };
        *pointer_point = requests[0].1;
        cfg.points = cfg.initial_points.clone();
        let mut trace = ReferenceValidationTestTrace::new();
        assert!(transfer_reference_facts(
            &mut cfg,
            &pass.context.deferred_policies,
            &pass.context.contracts,
            &pass.context.literal_contracts,
            &pass.context.call_exits,
            &pass.context.capture_input,
            ReferenceTransferTestRole::Function,
            &mut trace
        )
        .is_empty());
        let actual = trace.passes.last().unwrap();
        let pointer = requests[0].1;
        let saved = actual
            .observations
            .borrows
            .iter()
            .find(|snapshot| snapshot.point == requests[0].0)
            .unwrap()
            .state
            .point_values[pointer.0]
            .clone()
            .unwrap();
        for borrow in actual
            .observations
            .borrows
            .iter()
            .filter(|snapshot| requests.iter().any(|(point, _)| *point == snapshot.point))
        {
            assert_eq!(borrow.state.point_values[pointer.0].as_ref(), Some(&saved));
            assert!(
                matches!(&borrow.input, ReferenceBorrowInput::Through { pointer_point, origins } if *pointer_point == pointer && origins == &saved)
            );
        }
        for snapshot in actual
            .observations
            .points
            .iter()
            .filter(|snapshot| snapshot.point > pointer && snapshot.point < requests[1].0)
        {
            assert_eq!(
                snapshot.state.point_values[pointer.0].as_ref(),
                Some(&saved)
            );
        }
        let before = actual
            .observations
            .points
            .iter()
            .find(|snapshot| {
                snapshot.point == requests[1].0
                    && snapshot.phase == ReferenceTransferPhase::BeforeInputCleanup
            })
            .unwrap();
        let after = actual
            .observations
            .points
            .iter()
            .find(|snapshot| {
                snapshot.point == requests[1].0
                    && snapshot.phase == ReferenceTransferPhase::AfterInputCleanup
            })
            .unwrap();
        assert_eq!(before.state.point_values[pointer.0].as_ref(), Some(&saved));
        assert!(after.state.point_values[pointer.0].is_none());
        let facts = cfg.facts.clone();
        let map = cfg.checked_bindings.clone();
        let borrows = cfg.borrow_evaluations.clone();
        let installs = cfg.checked_install_evaluations.clone();
        cfg.points = cfg.initial_points.clone();
        assert!(transfer_reference_facts(
            &mut cfg,
            &pass.context.deferred_policies,
            &pass.context.contracts,
            &pass.context.literal_contracts,
            &pass.context.call_exits,
            &pass.context.capture_input,
            ReferenceTransferTestRole::Function,
            &mut ReferenceValidationTestTrace::new()
        )
        .is_empty());
        assert_eq!(cfg.facts, facts);
        assert_eq!(cfg.checked_bindings, map);
        assert_eq!(cfg.borrow_evaluations, borrows);
        assert_eq!(cfg.checked_install_evaluations, installs);
        let restored: ScalarReferenceCfg =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, restored);
    }

    macro_rules! boundary_test {
        ($name:ident, $layout:expr, $project:expr) => {
            #[test]
            fn $name() {
                rejection_and_greatest($layout, $project);
            }
        };
    }
    boundary_test!(
        d3_reborrow_state_evaluated_borrow_boundaries_native_program,
        ScalarTargetLayout::NATIVE64,
        false
    );
    boundary_test!(
        d3_reborrow_state_evaluated_borrow_boundaries_native_project,
        ScalarTargetLayout::NATIVE64,
        true
    );
    boundary_test!(
        d3_reborrow_state_evaluated_borrow_boundaries_wasm_program,
        ScalarTargetLayout::WASM32,
        false
    );
    boundary_test!(
        d3_reborrow_state_evaluated_borrow_boundaries_wasm_project,
        ScalarTargetLayout::WASM32,
        true
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_bind_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        Bind,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_bind_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        Bind,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_bind_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        Bind,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_bind_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        Bind,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_bind_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        Bind,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_bind_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        Bind,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_bind_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        Bind,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_bind_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        Bind,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_assign_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        Assign,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_assign_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        Assign,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_assign_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        Assign,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_assign_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        Assign,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_assign_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        Assign,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_assign_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        Assign,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_assign_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        Assign,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_assign_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        Assign,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_through_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        AssignThrough,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_through_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        AssignThrough,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_through_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        AssignThrough,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_through_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        AssignThrough,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_through_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        AssignThrough,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_through_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        AssignThrough,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_through_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        AssignThrough,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_through_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        AssignThrough,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_return_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        Return,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_return_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        Return,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_return_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        Return,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_foreign_return_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        Return,
        ForeignOrigin
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_return_native_program,
        ScalarTargetLayout::NATIVE64,
        false,
        Return,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_return_native_project,
        ScalarTargetLayout::NATIVE64,
        true,
        Return,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_return_wasm_program,
        ScalarTargetLayout::WASM32,
        false,
        Return,
        FullParent
    );
    composed_test!(
        d3_reborrow_state_evaluated_borrow_composed_parent_return_wasm_project,
        ScalarTargetLayout::WASM32,
        true,
        Return,
        FullParent
    );
}

#[path = "c3_history_worker_tests.rs"]
mod c3_history_worker_tests;
