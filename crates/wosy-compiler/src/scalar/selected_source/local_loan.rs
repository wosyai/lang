use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StagedLoanQueryRef {
    pub owner: ProducerCfgOwnerId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LocalLoanPendingReason {
    Coverage(CommonLoanPending),
    MissingContributor(LocalLoanCandidateRef),
    MissingMonitor(JointProducerTupleRef),
    UnsupportedRole(CommonLoanRoleId),
    MissingPort(CommonLoanRoleId),
    Unavailable {
        role: CommonLoanRoleId,
        state: ValueAvailability,
    },
    CheckedState {
        role: CommonLoanRoleId,
        state: CheckedValueState,
    },
    CheckedConstructor(CommonLoanRoleId),
    MultipleReferents(CommonLoanRoleId),
    UnsupportedTarget,
    Selector,
    Mode,
    RemainingSelectedUses,
    EscapeOutlives,
    CompleteFiniteNormalization,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalLoanSupport {
    pub role: CommonLoanRoleId,
    pub site: CommonLoanSiteId,
    pub selected: JointSelectedPortFact,
    pub grant: SourceGrantClassFact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LocalLoanAccess {
    DirectStorage {
        referent: SourceInstanceRef,
        coordinate: SourceSlotRef,
        projection: Vec<ValueProjection>,
    },
    Through(LocalLoanSupport),
    Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LocalLoanPairOutcome {
    Disjoint,
    CompatibleModes,
    PairedAuthority,
    IncompatiblePair,
    PendingPredicate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalLoanPair {
    pub support: LocalLoanSupport,
    pub protected: SourceGrantClassFact,
    pub outcome: LocalLoanPairOutcome,
    pub remaining_uses: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalLoanTupleQuery {
    pub candidate: LocalLoanCandidateRef,
    pub contributor: JointProducerTupleRef,
    pub input: SourceConfigurationRef,
    pub output: SourceConfigurationRef,
    pub equation: JointPortMonitorEquation,
    pub frame: FrameVariableRef,
    pub point: LocalLoanPoint,
    pub access: LocalLoanAccess,
    pub pairs: Vec<LocalLoanPair>,
    pub remaining_uses: SourceUseObservation,
    pub pending: Vec<LocalLoanPendingReason>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StagedLoanQueryResult {
    CommonOnly,
    Candidates {
        slices: Vec<LocalLoanSlice>,
        queries: Vec<LocalLoanTupleQuery>,
        pending: Vec<LocalLoanPendingReason>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StagedLoanQueryAnalysis {
    pub id: StagedLoanQueryRef,
    pub epoch: CommonLoanInput,
    pub coverage: Vec<CommonLoanPending>,
    pub result: StagedLoanQueryResult,
}

fn role_port(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    common: &CommonLoanAnalysis,
    role_id: CommonLoanRoleId,
) -> Result<SourceSelectedSlotFact, LocalLoanPendingReason> {
    let role = &common.input.roles[role_id.0];
    if !role.projection.is_empty() {
        return Err(LocalLoanPendingReason::UnsupportedRole(role_id));
    }
    let coordinate = match &role.source {
        CommonLoanSourceRole::Binding(binding) => SourceCoordinateRole::Binding(binding.clone()),
        CommonLoanSourceRole::Evaluated(value) => SourceCoordinateRole::Evaluated(value.clone()),
        CommonLoanSourceRole::Saved(event) => {
            let saved = row
                .saved
                .iter()
                .filter(|saved| {
                    saved.snapshot == *event && saved.selection.application.frame == row.frame
                })
                .collect::<Vec<_>>();
            let [saved] = saved.as_slice() else {
                return Err(LocalLoanPendingReason::MissingPort(role_id));
            };
            return Ok(SourceSelectedSlotFact {
                coordinate: saved.coordinate.clone(),
                view: saved.view.clone(),
                availability: saved.availability.clone(),
            });
        }
        CommonLoanSourceRole::Result(event) => {
            let mut values = tables
                .results
                .iter()
                .filter(|result| {
                    result.value.resolved.producer == event.owner
                        && result.value.point == event.point
                })
                .map(|result| result.value.clone())
                .collect::<Vec<_>>();
            values.extend(
                tables
                    .final_inputs
                    .iter()
                    .filter(|input| {
                        input.consumer.producer == event.owner
                            && input.consumer.point == event.point
                    })
                    .map(|input| input.source.clone()),
            );
            let [value] = values.as_slice() else {
                return Err(LocalLoanPendingReason::UnsupportedRole(role_id));
            };
            SourceCoordinateRole::Evaluated(value.clone())
        }
        CommonLoanSourceRole::Frozen(_) | CommonLoanSourceRole::Boundary(_) => {
            return Err(LocalLoanPendingReason::UnsupportedRole(role_id))
        }
    };
    selected_slot(tables, row, &coordinate)
        .cloned()
        .map_err(|_| LocalLoanPendingReason::MissingPort(role_id))
}

fn support(
    tables: &EvaluatedValueTables,
    row: &SourceConfigurationFact,
    common: &CommonLoanAnalysis,
    role: CommonLoanRoleId,
    equation: &JointPortMonitorEquation,
    source: &SourceSpan,
) -> Result<Result<LocalLoanSupport, LocalLoanPendingReason>, ReferenceAnalysisError> {
    let slot = match role_port(tables, row, common, role) {
        Ok(slot) => slot,
        Err(reason) => return Ok(Err(reason)),
    };
    if slot.availability != ValueAvailability::Available {
        return Ok(Err(LocalLoanPendingReason::Unavailable {
            role,
            state: slot.availability,
        }));
    }
    let monitor = tables
        .source_port_monitors
        .get(equation.input.index)
        .filter(|monitor| monitor.id == equation.input)
        .ok_or_else(|| invalid(source))?;
    if monitor.pending.contains(&slot.coordinate) {
        return Ok(Err(LocalLoanPendingReason::MissingPort(role)));
    }
    let selected = port_monitor::query(
        tables,
        &equation.input,
        std::slice::from_ref(&slot.coordinate),
        source,
    )?;
    let [selected] = selected.as_slice() else {
        return Err(invalid(source));
    };
    if selected.observation.view != slot.view {
        return Err(invalid(source));
    }
    let paths = tables
        .source_paths
        .iter()
        .filter(|path| path.id == selected.observation.path)
        .collect::<Vec<_>>();
    let [path] = paths.as_slice() else {
        return Err(invalid(source));
    };
    if path.frame != row.frame || selected.coordinate.owner != row.id.owner {
        return Err(invalid(source));
    }
    let view = tables
        .source_views
        .get(slot.view.index)
        .filter(|view| view.id == slot.view)
        .ok_or_else(|| invalid(source))?;
    let SourcePayloadView::Checked { state, referents } = &view.payload else {
        return Ok(Err(LocalLoanPendingReason::UnsupportedRole(role)));
    };
    if *state != CheckedValueState::Live {
        return Ok(Err(LocalLoanPendingReason::CheckedState {
            role,
            state: state.clone(),
        }));
    }
    let [referent] = referents.as_slice() else {
        return Ok(Err(LocalLoanPendingReason::MultipleReferents(role)));
    };
    let SourceCheckedGrantState::Selected(class) = &referent.loan else {
        return Ok(Err(LocalLoanPendingReason::CheckedConstructor(role)));
    };
    selected_grant::read(tables, row, selected, source)?;
    let grants = row
        .grants
        .iter()
        .filter(|grant| grant.class == *class)
        .collect::<Vec<_>>();
    let [grant] = grants.as_slice() else {
        return Err(invalid(source));
    };
    if grant.class.owner != row.id.owner || grant.site.owner != row.id.owner {
        return Err(invalid(source));
    }
    if let SourceInstanceRef::Supported(id) = &grant.referent {
        let instance = tables
            .source_instances
            .get(id.index)
            .filter(|instance| instance.id == *id)
            .ok_or_else(|| invalid(source))?;
        if instance.frame != row.frame {
            return Err(invalid(source));
        }
    }
    let targets = row
        .slots
        .iter()
        .filter(|target| target.coordinate == grant.coordinate)
        .collect::<Vec<_>>();
    let [target] = targets.as_slice() else {
        return Ok(Err(LocalLoanPendingReason::MissingPort(role)));
    };
    if target.availability != ValueAvailability::Available {
        return Ok(Err(LocalLoanPendingReason::Unavailable {
            role,
            state: target.availability.clone(),
        }));
    }
    let sites = common
        .input
        .sites
        .iter()
        .enumerate()
        .filter(|(_, site)| site.event == grant.site)
        .collect::<Vec<_>>();
    let [(index, _)] = sites.as_slice() else {
        return Ok(Err(LocalLoanPendingReason::CheckedConstructor(role)));
    };
    Ok(Ok(LocalLoanSupport {
        role,
        site: CommonLoanSiteId(*index),
        selected: selected.clone(),
        grant: (*grant).clone(),
    }))
}

fn projection_overlap(left: &[ValueProjection], right: &[ValueProjection]) -> Option<bool> {
    for (left, right) in left.iter().zip(right) {
        match (left, right) {
            (ValueProjection::Field(left), ValueProjection::Field(right)) => {
                if left != right {
                    return Some(false);
                }
            }
            (ValueProjection::FixedIndex(left), ValueProjection::FixedIndex(right)) => {
                if left != right {
                    return Some(false);
                }
            }
            _ => return None,
        }
    }
    if left.iter().chain(right).any(|projection| {
        !matches!(
            projection,
            ValueProjection::Field(_) | ValueProjection::FixedIndex(_)
        )
    }) {
        return None;
    }
    Some(true)
}

pub(super) fn pair(
    row: &SourceConfigurationFact,
    access: &LocalLoanAccess,
    mode: ScalarReferenceMutability,
    protected: &SourceGrantClassFact,
) -> LocalLoanPairOutcome {
    let (referent, projection) = match access {
        LocalLoanAccess::DirectStorage {
            referent,
            projection,
            ..
        } => (referent, projection),
        LocalLoanAccess::Through(access) => (&access.grant.referent, &access.grant.projection),
        LocalLoanAccess::Pending => return LocalLoanPairOutcome::PendingPredicate,
    };
    if matches!(referent, SourceInstanceRef::Residual(_))
        || matches!(protected.referent, SourceInstanceRef::Residual(_))
    {
        return LocalLoanPairOutcome::PendingPredicate;
    }
    if *referent != protected.referent {
        return LocalLoanPairOutcome::Disjoint;
    }
    match projection_overlap(projection, &protected.projection) {
        Some(false) => return LocalLoanPairOutcome::Disjoint,
        None => return LocalLoanPairOutcome::PendingPredicate,
        Some(true) => {}
    }
    if let LocalLoanAccess::Through(access) = access {
        if mode == ScalarReferenceMutability::Mutable
            && access.grant.mode != ScalarReferenceMutability::Mutable
        {
            return LocalLoanPairOutcome::PendingPredicate;
        }
        if access.grant.coordinate != protected.coordinate {
            return LocalLoanPairOutcome::PendingPredicate;
        }
    }
    if mode == ScalarReferenceMutability::Shared
        && protected.mode == ScalarReferenceMutability::Shared
    {
        return LocalLoanPairOutcome::CompatibleModes;
    }
    if let LocalLoanAccess::Through(access) = access {
        if access.grant.class == protected.class
            || row.grant_ancestry.iter().any(|pair| {
                pair.ancestor == protected.class && pair.descendant == access.grant.class
            })
        {
            return LocalLoanPairOutcome::PairedAuthority;
        }
    }
    LocalLoanPairOutcome::IncompatiblePair
}

fn query(
    cfg: &ScalarReferenceCfg,
    common: &CommonLoanAnalysis,
    slice: &LocalLoanSlice,
    rule: &JointProducerTupleFact,
    equation: &JointPortMonitorEquation,
) -> Result<LocalLoanTupleQuery, ReferenceAnalysisError> {
    let tables = &cfg.evaluated_values;
    let candidate = &common.candidates[slice.candidate.ordinal];
    let source = &candidate.source_span;
    if !common.input.events.iter().any(|event| event.event == candidate.event && event.source_span == candidate.source_span && event.facts.iter().any(|fact| matches!(&fact.kind, CommonLoanFactKind::Use { role, mode, target, .. } if *role == candidate.role && *mode == candidate.mode && *target == candidate.target))) { return Err(invalid(source)); }
    let input = tables
        .source_configurations
        .get(rule.input.index)
        .filter(|row| row.id == rule.input)
        .ok_or_else(|| invalid(source))?;
    let output = tables
        .source_configurations
        .get(rule.output.index)
        .filter(|row| row.id == rule.output)
        .ok_or_else(|| invalid(source))?;
    let monitor = tables
        .source_port_monitors
        .get(equation.input.index)
        .filter(|monitor| monitor.id == equation.input)
        .ok_or_else(|| invalid(source))?;
    let prior = tables
        .source_contributors
        .get(monitor.contributor.index)
        .filter(|prior| prior.id == monitor.contributor)
        .ok_or_else(|| invalid(source))?;
    let after = tables
        .source_port_monitors
        .get(equation.output.index)
        .filter(|monitor| monitor.id == equation.output)
        .ok_or_else(|| invalid(source))?;
    if rule.event != candidate.event
        || rule.id != equation.contributor
        || rule.id.owner != slice.candidate.owner
        || input.id.owner != rule.id.owner
        || output.id.owner != rule.id.owner
        || prior.output != input.id
        || after.contributor != rule.id
        || input.frame != output.frame
        || input.frame.owner != candidate.event.owner
    {
        return Err(invalid(source));
    }
    port_monitor::query(tables, &equation.input, &[], source)?;
    port_monitor::query(tables, &equation.output, &[], source)?;
    let graphs = tables
        .source_use_graphs
        .iter()
        .filter(|graph| graph.id.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [graph] = graphs.as_slice() else {
        return Err(invalid(source));
    };
    let remaining_uses = source_use::observe(
        graph,
        &SourceUseNodeRef {
            contributor: prior.id.clone(),
            monitor: equation.input.clone(),
        },
    )?;
    let mut pending = vec![
        LocalLoanPendingReason::RemainingSelectedUses,
        LocalLoanPendingReason::CompleteFiniteNormalization,
    ];
    pending.extend(
        slice
            .coverage
            .iter()
            .cloned()
            .map(LocalLoanPendingReason::Coverage),
    );
    if candidate.reason == CommonLoanCandidateReason::EscapeOutlives {
        pending.push(LocalLoanPendingReason::EscapeOutlives);
    }
    let access = match &candidate.target {
        CommonLoanTarget::Through { pointer, place }
            if matches!(place, ScalarPlace::Dereference { .. }) =>
        {
            match support(tables, input, common, *pointer, equation, source)? {
                Ok(support) => LocalLoanAccess::Through(support),
                Err(reason) => {
                    pending.push(reason);
                    LocalLoanAccess::Pending
                }
            }
        }
        CommonLoanTarget::Storage {
            binding,
            projection,
        } => {
            let slot = selected_slot(
                tables,
                input,
                &SourceCoordinateRole::Binding(binding.clone()),
            );
            match slot {
                Ok(slot) if slot.availability == ValueAvailability::Available => {
                    let referent = match &tables.source_views[slot.view.index].payload {
                        SourcePayloadView::Backing { instance, .. } => Ok(instance.clone()),
                        _ => current_cell(tables, input, &slot.coordinate),
                    };
                    match referent {
                        Ok(referent) => LocalLoanAccess::DirectStorage {
                            referent,
                            coordinate: slot.coordinate.clone(),
                            projection: projection.clone(),
                        },
                        Err(()) => {
                            pending.push(LocalLoanPendingReason::UnsupportedTarget);
                            LocalLoanAccess::Pending
                        }
                    }
                }
                _ => {
                    pending.push(LocalLoanPendingReason::UnsupportedTarget);
                    LocalLoanAccess::Pending
                }
            }
        }
        _ => {
            pending.push(LocalLoanPendingReason::UnsupportedTarget);
            LocalLoanAccess::Pending
        }
    };
    let points = slice
        .points
        .iter()
        .filter(|point| point.event.event == candidate.event)
        .collect::<Vec<_>>();
    let [point] = points.as_slice() else {
        return Err(invalid(source));
    };
    let mut pairs = Vec::new();
    for member in point.incoming.iter().filter(|member| {
        point.demand_in.contains(&member.role) && candidate.sites.contains(&member.site)
    }) {
        let support = match support(tables, input, common, member.role, equation, source)? {
            Ok(support) => support,
            Err(reason) => {
                if !pending.contains(&reason) {
                    pending.push(reason);
                }
                continue;
            }
        };
        if support.site != member.site {
            continue;
        }
        for protected in input.grants.iter().filter(|grant| {
            grant.class == support.grant.class
                || input.grant_ancestry.iter().any(|pair| {
                    pair.ancestor == grant.class && pair.descendant == support.grant.class
                })
        }) {
            let outcome = pair(input, &access, candidate.mode, protected);
            if outcome == LocalLoanPairOutcome::PendingPredicate {
                let reason = match &access {
                    LocalLoanAccess::Through(access)
                        if candidate.mode == ScalarReferenceMutability::Mutable
                            && access.grant.mode != ScalarReferenceMutability::Mutable =>
                    {
                        LocalLoanPendingReason::Mode
                    }
                    LocalLoanAccess::Through(access)
                        if projection_overlap(&access.grant.projection, &protected.projection)
                            .is_none() =>
                    {
                        LocalLoanPendingReason::Selector
                    }
                    LocalLoanAccess::DirectStorage { projection, .. }
                        if projection_overlap(projection, &protected.projection).is_none() =>
                    {
                        LocalLoanPendingReason::Selector
                    }
                    _ => LocalLoanPendingReason::UnsupportedTarget,
                };
                if !pending.contains(&reason) {
                    pending.push(reason);
                }
            }
            pairs.push(LocalLoanPair {
                outcome,
                remaining_uses: remaining_uses.reachable.iter().copied().filter(|index| matches!(&graph.reachable[*index].support.subject, SourceUseSubject::Grant { class, referent } if *referent == protected.referent && (*class == protected.class || input.grant_ancestry.iter().any(|pair| pair.ancestor == protected.class && pair.descendant == *class)))).collect(),
                support: support.clone(),
                protected: protected.clone(),
            });
        }
    }
    Ok(LocalLoanTupleQuery {
        candidate: slice.candidate.clone(),
        contributor: rule.id.clone(),
        input: input.id.clone(),
        output: output.id.clone(),
        equation: equation.clone(),
        frame: input.frame.clone(),
        point: (*point).clone(),
        access,
        pairs,
        remaining_uses,
        pending,
    })
}

fn build(cfg: &ScalarReferenceCfg) -> Result<StagedLoanQueryAnalysis, ReferenceAnalysisError> {
    let source = &cfg.initial_points[0].source_span;
    let tables = &cfg.evaluated_values;
    let analyses = tables
        .common_loans
        .iter()
        .filter(|analysis| analysis.input.owner == cfg.producer_owner)
        .collect::<Vec<_>>();
    let [common] = analyses.as_slice() else {
        return Err(invalid(source));
    };
    common_loan::validate_epoch(cfg, common)?;
    source_use::validate(cfg)?;
    let result = if common.candidates.is_empty() {
        StagedLoanQueryResult::CommonOnly
    } else {
        let mut slices = Vec::new();
        let mut queries = Vec::new();
        let mut pending = Vec::new();
        for ordinal in 0..common.candidates.len() {
            let slice = common_loan::local_slice::solve(common, ordinal);
            let candidate = &common.candidates[ordinal];
            let rules = tables
                .source_contributors
                .iter()
                .filter(|rule| rule.event == candidate.event)
                .collect::<Vec<_>>();
            if rules.is_empty() {
                pending.push(LocalLoanPendingReason::MissingContributor(
                    slice.candidate.clone(),
                ));
            }
            for rule in rules {
                let equations = tables
                    .source_port_equations
                    .iter()
                    .filter(|equation| equation.contributor == rule.id)
                    .collect::<Vec<_>>();
                if equations.is_empty() {
                    pending.push(LocalLoanPendingReason::MissingMonitor(rule.id.clone()));
                }
                for equation in equations {
                    queries.push(query(cfg, common, &slice, rule, equation)?);
                }
            }
            slices.push(slice);
        }
        StagedLoanQueryResult::Candidates {
            slices,
            queries,
            pending,
        }
    };
    Ok(StagedLoanQueryAnalysis {
        id: StagedLoanQueryRef {
            owner: cfg.producer_owner.clone(),
        },
        epoch: common.input.clone(),
        coverage: common.pending.clone(),
        result,
    })
}

pub(in crate::scalar) fn register(
    cfg: &mut ScalarReferenceCfg,
) -> Result<(), ReferenceAnalysisError> {
    source_use::register(cfg)?;
    let analysis = build(cfg)?;
    let id = analysis.id.clone();
    cfg.evaluated_values
        .staged_loan_queries
        .retain(|old| old.id.owner != cfg.producer_owner);
    cfg.evaluated_values.staged_loan_queries.push(analysis);
    for draft in &mut cfg.evaluated_values.pending_owner_consumers {
        if draft.candidate.consumer.producer == cfg.producer_owner {
            let requirement = OwnerConsumerRequirement::CandidateLocalQueries(id.clone());
            if !draft.requirements.contains(&requirement) {
                draft.requirements.push(requirement);
            }
        }
    }
    Ok(())
}

pub(in crate::scalar) fn validate(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    let expected = build(cfg)?;
    let rows = cfg
        .evaluated_values
        .staged_loan_queries
        .iter()
        .filter(|row| row.id == expected.id)
        .collect::<Vec<_>>();
    let [row] = rows.as_slice() else {
        return Err(invalid(&cfg.initial_points[0].source_span));
    };
    if **row != expected {
        return Err(invalid(&cfg.initial_points[0].source_span));
    }
    for draft in cfg
        .evaluated_values
        .pending_owner_consumers
        .iter()
        .filter(|draft| draft.candidate.consumer.producer == cfg.producer_owner)
    {
        if !draft
            .requirements
            .contains(&OwnerConsumerRequirement::CandidateLocalQueries(
                expected.id.clone(),
            ))
        {
            return Err(invalid(
                &cfg.initial_points[draft.candidate.consumer.point.0].source_span,
            ));
        }
    }
    Ok(())
}
