//! Quantified claims about prepared immutable fixed-array return cells.
use super::*;
use super::super::checker_flow::{
    charge_call_work, collect_exact_fixed_array_return, observe_fixed_array_returns,
    FixedArrayCheck, FixedArrayEntries, FixedArrayQuantifier, FixedArrayReturnFailure, FixedArraySymbolicCheck,
};

pub(super) struct ArrayProof<'a> {
    proof: &'a CstNode,
    source: &'a SourceIdentity,
    owner: &'a SourceIdentity,
    target: PreparedCallable,
    goal: ByteSpan,
    failed_step: Option<ByteSpan>,
    shifted_witness: Option<(String, BigInt)>,
    entry_filter: Option<Vec<CheckerPredicate>>,
    domain: IntegerSet,
    quantifier: FixedArrayQuantifier<BigInt>,
    relation: IntegerRelation,
    expected: BigInt,
    body: &'a FunctionCallSites,
    result: &'a ScalarType,
}

pub(super) fn is_array_target(proof: &SelectedModuleProof<'_>) -> bool {
    matches!(proof.body.boundary.results.as_slice(), [out] if matches!(out.ty, ScalarType::Array { .. }))
}

impl<'a> ArrayProof<'a> {
    pub(super) fn prepare(
        source: &'a SourceIdentity, selected: SelectedModuleProof<'a>, array_work: &mut usize,
    ) -> Result<Vec<Self>, ProjectCheck> {
        let proof = selected.node;
        let target = (selected.function.0, selected.function.1);
        let owner = selected.owner;
        let body = selected.body;
        let [output] = body.boundary.results.as_slice() else { return Err(ProjectCheck::Unsupported); };
        let ScalarType::Array { element, length, .. } = &output.ty else { return Err(ProjectCheck::Unsupported); };
        if !body.boundary.parameters.iter().all(|parameter| is_integer_type(&parameter.ty))
            || !is_integer_type(element)
            || !matches!(selected.target, ResolvedCallTarget::ModuleCallable { source, declaration_span,
                concrete: ConcreteCallSelection::Declaration } if source == &owner.source
                    && *declaration_span == selected.function.2.name_span)
            || owner.source.project != source.project || owner.source.package != source.package
            || (&owner.source != source && proof_target_tokens(proof)?.0.is_none())
            || proof.children().any(|node| node.kind() == SyntaxKind::ProofDecreases)
        {
            return Err(ProjectCheck::Unsupported);
        }
        let target_node = child(proof, SyntaxKind::ProofTarget)?;
        let inputs = proof_input_names(&target_node)?;
        if inputs.len() != body.boundary.parameters.len() {
            return Err(ProjectCheck::InvalidProof);
        }
        let outputs = child(&child(proof, SyntaxKind::ProofOutputs)?, SyntaxKind::Parameters)?;
        let mut result_names = identifiers(&outputs);
        let binding = result_names.next().ok_or(ProjectCheck::InvalidProof)?;
        if result_names.next().is_some() { return Err(ProjectCheck::InvalidProof); }
        let mut unique = HashSet::new();
        if !inputs.iter().chain([&binding]).all(|name| !is_null_literal_name(name) && unique.insert(name.as_str())) {
            return Err(ProjectCheck::InvalidProof);
        }

        let ensures = child(proof, SyntaxKind::ProofEnsures)?;
        let exists = ensures.children().find(|node| node.kind() == SyntaxKind::ProofExists);
        let mut foralls = proof.children().filter(|node| node.kind() == SyntaxKind::ProofForall);
        let (quantifier, existential, dependency) = match (exists, foralls.next(), foralls.next()) {
            (Some(exists), None, None) => (exists, true, None),
            (Some(exists), Some(forall), None) => (exists, true, Some(forall)),
            (None, Some(forall), None) => (forall, false, None),
            _ => return Err(ProjectCheck::Unsupported),
        };
        let index_binder = |clause: &CstNode| -> Result<String, ProjectCheck> {
            let index_ty = super::super::derive_type(&child(clause, SyntaxKind::TypeSpec)?);
            let mut names = identifiers(clause);
            let binder = names.next().ok_or(ProjectCheck::InvalidProof)?;
            if index_ty != ScalarType::U64 || unique.contains(binder.as_str())
                || names.next().is_some() || is_null_literal_name(&binder)
            {
                return Err(ProjectCheck::InvalidProof);
            }
            Ok(binder)
        };
        let index_ty = ScalarType::U64;
        let binder = index_binder(&quantifier)?;
        let target_domain = typed_quantifier_domain(&quantifier, &binder, &inputs, &index_ty)?;
        let (mut domain, witness_dependency) = if let Some(forall) = dependency {
            let universal = index_binder(&forall)?;
            if universal == binder { return Err(ProjectCheck::InvalidProof); }
            let active = typed_quantifier_domain(&forall, &universal, &inputs, &index_ty)?;
            (active, Some((universal, target_domain)))
        } else {
            (target_domain, None)
        };
        if proof.children().find(|node| node.kind() == SyntaxKind::ProofWhere).is_some_and(|clause|
            clause.descendants().take(MAX_ACTUAL_CALL_DEPTH + 1).count() > MAX_ACTUAL_CALL_DEPTH)
        {
            return Err(ProjectCheck::Unsupported);
        }
        let entry_filter = prepare_proof_entry_filter_in_scope(proof, source, owner.target_layout, &inputs, body, true)?;
        let goal_node = child(if existential { &quantifier } else { &ensures }, SyntaxKind::Expression)?;
        // The ordinary scalar derivation is recursive; reject before handling
        // a deep or oversized language shape outside this narrow proposition.
        let claim = |node: &CstNode| -> Result<ScalarExpression, ProjectCheck> {
            if node.descendants().take(MAX_ACTUAL_CALL_DEPTH + 1).count() > MAX_ACTUAL_CALL_DEPTH {
                return Err(ProjectCheck::Unsupported);
            }
            Ok(derive_expression(node))
        };
        let checked_claim = |expression: &ScalarExpression| -> Result<(IntegerRelation, BigInt), ProjectCheck> {
            let ScalarExpression::Binary { operator, left, right, .. } = expression else {
                return Err(ProjectCheck::Unsupported);
            };
            let ScalarExpression::IndexedRead { place: super::super::ScalarPlace::Index { base, index, .. }, .. } = left.as_ref() else {
                return Err(ProjectCheck::Unsupported);
            };
            if !matches!(base.as_ref(), super::super::ScalarPlace::Name { name, .. } if name == &binding)
                || !matches!(index.as_ref(), ScalarExpression::Name { name, .. } if name == &binder)
            {
                return Err(ProjectCheck::InvalidProof);
            }
            let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
            let literal = concrete_integer_literal(right).ok_or(ProjectCheck::Unsupported)?;
            integer_literal_fits_type(literal.as_ref(), element).then(|| (relation, literal.into_owned()))
                .ok_or(ProjectCheck::InvalidProof)
        };
        let (relation, expected) = checked_claim(&claim(&goal_node)?)?;
        let block = child(proof, SyntaxKind::Block)?;
        if block.descendants().take(MAX_ACTUAL_CALL_DEPTH + 1).count() > MAX_ACTUAL_CALL_DEPTH {
            return Err(ProjectCheck::Unsupported);
        }
        let steps = derive_block(&block);
        if existential && witness_split(&steps).is_some() {
            if inputs.len() != 1 { return Err(ProjectCheck::Unsupported); }
            let mut branches = Vec::new();
            let mut active = entry_filter.unwrap_or_default();
            compile_witness_branches(&steps, &inputs, body, &mut active, array_work, 0,
                &mut |leaf, filter, work| {
                    let argument = authored_witness_argument(leaf)?;
                    let (image, quantifier, shifted_witness) = if let Some((universal, target)) = &witness_dependency {
                        let (image, shift) = dependent_witness_image(argument, universal, &domain, target, work)?;
                        check_array_index_domain(&image, *length)?;
                        (image, FixedArrayQuantifier::All, Some((universal.clone(), shift.unwrap_or_default())))
                    } else {
                        let index = checked_explicit_index(argument, &domain, *length)?;
                        (Cow::Borrowed(&domain), FixedArrayQuantifier::ExplicitWitness(index), None)
                    };
                    let clone_cost = match &image {
                        Cow::Borrowed(domain) => domain.interval_count(),
                        Cow::Owned(_) => 0,
                    };
                    charge_call_work(work, clone_cost.checked_add(1).ok_or(ProjectCheck::Unsupported)?)
                        .ok_or(ProjectCheck::Unsupported)?;
                    branches.push(Self {
                        proof, source, owner: &owner.source, target, goal: wosy_syntax::byte_span(&goal_node),
                        failed_step: Some(witness_step_span(leaf)?), shifted_witness, entry_filter: Some(filter.to_vec()),
                        domain: image.into_owned(), quantifier, relation, expected: expected.clone(), body, result: &output.ty,
                    });
                    Ok(())
                })?;
            return Ok(branches);
        }
        let mut failed_step = None;
        let mut shifted_witness = None;
        let quantifier_kind = if let Some((universal, target_domain)) = witness_dependency {
            let argument = authored_witness_argument(&steps)?;
            let (image, shift) = dependent_witness_image(argument, &universal, &domain, &target_domain, array_work)?;
            if let Cow::Owned(image) = image { domain = image; }
            shifted_witness = shift.map(|offset| (universal, offset));
            failed_step = Some(witness_step_span(&steps)?);
            FixedArrayQuantifier::All
        } else if existential && steps.items.is_empty() {
            FixedArrayQuantifier::FindWitness
        } else if existential {
            let value = checked_explicit_index(authored_witness_argument(&steps)?, &domain, *length)?;
            failed_step = Some(witness_step_span(&steps)?);
            FixedArrayQuantifier::ExplicitWitness(value)
        } else {
            if !steps.items.is_empty() {
                let ([item], [true]) = (steps.items.as_slice(), steps.terminated_items.as_slice()) else {
                    return Err(ProjectCheck::Unsupported);
                };
                let shown = proof_show_expression(item, true)?;
                let (shown_relation, shown_bound) = checked_claim(shown)?;
                if shown_relation != relation || shown_bound != expected { return Err(ProjectCheck::Unsupported); }
                let ScalarBlockItem::Expression(expression) = item else { return Err(ProjectCheck::Unsupported); };
                failed_step = Some(super::super::typecheck::expression_span(expression));
            }
            FixedArrayQuantifier::All
        };
        if !matches!(quantifier_kind, FixedArrayQuantifier::ExplicitWitness(_)) {
            check_array_index_domain(&domain, *length)?;
        }
        let goal = if matches!(quantifier_kind, FixedArrayQuantifier::FindWitness) { &quantifier } else { &goal_node };
        Ok(vec![Self { proof, source, owner: &owner.source, target, goal: wosy_syntax::byte_span(goal), failed_step, shifted_witness,
            entry_filter, domain, quantifier: quantifier_kind, relation, expected, body,
            result: &output.ty }])
    }

    fn check(&self) -> FixedArrayCheck<'_> {
        FixedArrayCheck {
            domain: &self.domain, quantifier: self.quantifier.as_ref(),
            relation: self.relation, expected: &self.expected,
        }
    }

    fn diagnostic(
        &self, failure: FixedArrayReturnFailure, source: &SourceIdentity, call: Option<wosy_syntax::SourceSpan>,
    ) -> Result<AggregateProofDiagnostic, ProjectCheck> {
        let at = |span| wosy_syntax::SourceSpan::new(self.source.clone(), span);
        let bindings = self.body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
        let axes = bindings.iter().zip(&self.body.boundary.parameters)
            .map(|(binding, parameter)| (binding.clone(), parameter.ty.clone())).collect();
        let equal_axes = failure.inputs.row_equal_axes(0).ok_or(ProjectCheck::Unsupported)?.to_vec();
        Ok(AggregateProofDiagnostic {
            proof: at(wosy_syntax::byte_span(self.proof)), goal: at(self.goal), failed_step: self.failed_step.map(at),
            output: wosy_syntax::SourceSpan::new(source.clone(), failure.span), call,
            cell: failure.cell.map(|(index, value)| {
                let index = BigInt::from(index);
                let universal = self.shifted_witness.as_ref().map(|(name, offset)| (name.clone(), &index - offset));
                AggregateProofCell { index, value, universal }
            }),
            witness: ScalarCheckerIntegerWitness { equal_axes, axes, state: failure.inputs },
        })
    }

}

fn witness_split(steps: &ScalarBlock) -> Option<(&ScalarExpression, &ScalarBlock, &ScalarBlock)> {
    let [ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. })]
        = steps.items.as_slice() else { return None; };
    (steps.terminated_items == [false] && !steps.unsafe_context)
        .then_some((condition, then_branch, else_branch))
}

fn compile_witness_branches(
    steps: &ScalarBlock, inputs: &[String], body: &FunctionCallSites, active: &mut Vec<CheckerPredicate>,
    work: &mut usize, depth: usize,
    emit: &mut impl FnMut(&ScalarBlock, &[CheckerPredicate], &mut usize) -> Result<(), ProjectCheck>,
) -> Result<(), ProjectCheck> {
    if depth >= MAX_ACTUAL_CALL_DEPTH { return Err(ProjectCheck::Unsupported); }
    charge_call_work(work, 1).ok_or(ProjectCheck::Unsupported)?;
    if let Some((condition, yes, no)) = witness_split(steps) {
        for (branch, negated) in [(yes, false), (no, true)] {
            active.push(proof_input_condition::<true>(condition, inputs, body, negated)?);
            compile_witness_branches(branch, inputs, body, active, work, depth + 1, emit)?;
            active.pop();
        }
        return Ok(());
    }
    let cost = active.iter().try_fold(1usize, |total, filter| total.checked_add(match filter {
        CheckerPredicate::EntryDomain { values, .. } => values.interval_count().checked_add(1)?,
        _ => 1,
    })).ok_or(ProjectCheck::Unsupported)?;
    charge_call_work(work, cost).ok_or(ProjectCheck::Unsupported)?;
    emit(steps, active, work)
}

fn witness_step_span(steps: &ScalarBlock) -> Result<ByteSpan, ProjectCheck> {
    let [ScalarBlockItem::Expression(expression)] = steps.items.as_slice() else { return Err(ProjectCheck::Unsupported); };
    Ok(super::super::typecheck::expression_span(expression))
}

fn checked_explicit_index(argument: &ScalarExpression, domain: &IntegerSet, length: u64) -> Result<BigInt, ProjectCheck> {
    let value = concrete_integer_literal(argument).ok_or(ProjectCheck::Unsupported)?.into_owned();
    if !integer_literal_fits_type(&value, &ScalarType::U64) || !domain.contains(&value)
        || u64::try_from(&value).ok().is_none_or(|value| value >= length)
    {
        return Err(ProjectCheck::InvalidProof);
    }
    Ok(value)
}

fn check_array_index_domain(domain: &IntegerSet, length: u64) -> Result<(), ProjectCheck> {
    if domain.extremes().is_some_and(|(first, last)| u64::try_from(first).is_err()
        || u64::try_from(last).ok().is_none_or(|last| last >= length))
    {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(())
}

fn dependent_witness_image<'a>(
    argument: &ScalarExpression, universal: &str, domain: &'a IntegerSet, target: &IntegerSet,
    work: &mut usize,
) -> Result<(Cow<'a, IntegerSet>, Option<BigInt>), ProjectCheck> {
    let (image, shift) = checked_quantified_witness_translation(argument, universal, domain, &ScalarType::U64, work)?;
    let cost = image.interval_count().checked_add(target.interval_count()).ok_or(ProjectCheck::Unsupported)?;
    charge_call_work(work, cost).ok_or(ProjectCheck::Unsupported)?;
    if !image.is_subset_of(target) { return Err(ProjectCheck::Unsupported); }
    Ok((image, shift))
}

type ArrayCoverage = ProofCoverage<Option<AggregateProofDiagnostic>>;

pub(super) struct Cohort<'a> {
    plans: Vec<ArrayProof<'a>>,
    groups: HashMap<PreparedCallable, Group>,
}

struct Group {
    ids: Vec<usize>,
    supported_body: std::cell::OnceCell<bool>,
}

impl Group {
    fn validate(&self, plan: &ArrayProof<'_>, body: &FunctionCallSites) -> Result<(), ProjectCheck> {
        if !std::ptr::eq(body, plan.body) { return Err(ProjectCheck::Unsupported); }
        let supported = self.supported_body.get_or_init(|| {
            let structure = &body.structure;
            body.call_occurrences.is_empty() && body.assignments.is_empty() && body.arms.is_empty()
                && body.allocation_length_calls.is_empty() && structure.checker_directives.is_empty()
                && !structure.has_ordinary_contextual_places() && !structure.checker_has_bool_contextual_declaration
                && !body.conditionals.values().any(|region| matches!(region, ConditionalRegion::Unit { .. }))
        });
        supported.then_some(()).ok_or(ProjectCheck::Unsupported)
    }
}

impl<'a> Cohort<'a> {
    pub(super) fn new(plans: Vec<ArrayProof<'a>>, roots: Option<&ProjectCallSiteResults>) -> Result<Self, ProjectCheck> {
        let mut groups = HashMap::<PreparedCallable, Group>::new();
        let mut work = 0;
        charge_call_work(&mut work, plans.len()).ok_or(ProjectCheck::Unsupported)?;
        for (id, plan) in plans.iter().enumerate() {
            use std::collections::hash_map::Entry;
            match groups.entry(plan.target) {
                Entry::Vacant(group) => {
                    // The existing resolved-edge walk marks nested uses.
                    // Such a use has no direct array initializer receipt and
                    // must not turn into a symbolic unused root.
                    if roots.is_some_and(|calls| calls.has_nested_proof_use(plan.target)) {
                        return Err(ProjectCheck::Unsupported);
                    }
                    group.insert(Group { ids: vec![id], supported_body: std::cell::OnceCell::new() });
                }
                Entry::Occupied(mut entry) => {
                    let group = entry.get_mut();
                    let prior = &plans[group.ids[0]];
                    if prior.owner != plan.owner || !std::ptr::eq(prior.body, plan.body) {
                        return Err(ProjectCheck::Unsupported);
                    }
                    group.ids.push(id);
                }
            }
        }
        Ok(Self { plans, groups })
    }

    /// Actual proof receipts and closed no-incoming-edge deferred claims use
    /// one prepared owner and share a single metered startup walk.
    pub(super) fn run_capped(
        &self, owner: &PreparedProofModule, scope: ScalarCheckerScope, cap: Option<std::num::NonZeroUsize>,
        bare: &[PlannedModuleProof<'_>], rank: Option<&PlannedModuleProof<'_>>, provider: Option<&SourceIdentity>,
    ) -> Result<(MainFlowProofReceipt, StartupAuthoredCompletion), ProjectCheck> {
        use failure_search::StartupProofRole::{Aggregate, Bare, Ranked};
        let prepared = &owner.prepared;
        let (project, calls) = prepared.parts();
        let (root, ordinals) = package_startup_order(project, &project.modules)?;
        if root != &owner.source || (provider.is_some() && rank.is_some()) { return Err(ProjectCheck::Unsupported); }
        let owned = |source: &SourceIdentity| source == root || Some(source) == provider;
        let (_, authored_roots) = package_obligation_roots(prepared, scope, project.modules.len(), root)?;
        if !authored_roots.is_empty() { return Err(ProjectCheck::Unsupported); }

        let mut targets = Vec::with_capacity(self.groups.len().checked_add(bare.len())
            .and_then(|count| count.checked_add(usize::from(rank.is_some()))).ok_or(ProjectCheck::Unsupported)?);
        for (&target, group) in &self.groups {
            let plan = &self.plans[group.ids[0]];
            let module = project.modules.get(target.0).ok_or(ProjectCheck::Unsupported)?;
            let Some(ScalarItem::Function(declaration)) = module.items.get(target.1) else { return Err(ProjectCheck::Unsupported); };
            if &module.source != plan.owner || !owned(plan.owner) || plan.source != plan.owner {
                return Err(ProjectCheck::Unsupported);
            }
            if group.ids.iter().any(|&id| self.plans[id].source != plan.owner) { return Err(ProjectCheck::Unsupported); }
            targets.push((target, plan.owner, declaration.name_span, Aggregate));
        }
        for plan in bare {
            if !owned(plan.proof_source) || self.groups.contains_key(&plan.target) {
                return Err(ProjectCheck::Unsupported);
            }
            targets.push((plan.target, plan.proof_source, plan.function.name_span, Bare));
        }
        if let Some(plan) = rank {
            if plan.target.0 != 0 || plan.proof_source != root || self.groups.contains_key(&plan.target)
                || bare.iter().any(|bare| bare.target == plan.target || bare.name == plan.name)
            { return Err(ProjectCheck::Unsupported); }
            targets.push((plan.target, plan.proof_source, plan.function.name_span, Ranked));
        }
        let scalar = if let [first, rest @ ..] = bare {
            let mut observer = MainFlowProofObserver::from_first_plan(scope, root, first)?;
            for plan in rest {
                observer.insert(scope, false, plan, MainFlowProofCase::prepare(plan, plan.proof_source)?);
            }
            observer.finish_plans(prepared, scope)?;
            Some(observer)
        } else { None };
        let incoming = calls.owned_direct_startup_callable_uses(root, provider,
            targets.iter().map(|(_, source, span, _)| (*source, *span)), rank.map(|plan| (plan.proof_source, plan.target.1, plan.function.name_span))).ok_or(ProjectCheck::Unsupported)?;
        let mut trace = match cap {
            Some(cap) => failure_search::StartUpSearch::new(root, cap),
            None => failure_search::StartUpSearch::formal_only(root),
        };
        trace.retain_after_cutoff = true;
        trace.named_cross_source = provider.is_some();
        let mut reached_arrays = HashSet::new();
        let mut deferred_arrays = Vec::new();
        let mut invoked_scalars = HashSet::new();
        let mut scalar_observations = 0usize;
        let mut verified_rank = None;
        for (target, source, span, role) in targets {
            let sites = incoming.get(&(source, span)).ok_or(ProjectCheck::Unsupported)?;
            if sites.is_empty() {
                if role == Aggregate { deferred_arrays.push(target); }
                continue;
            }
            match role {
                Aggregate => { reached_arrays.insert(target); },
                Bare => {
                    let Some(plan) = bare.iter().find(|plan| plan.target == target) else { return Err(ProjectCheck::Unsupported); };
                    if !matches!(plan.typed.body.boundary.parameters.as_slice(), [slot] if is_integer_type(&slot.ty)) {
                        return Err(ProjectCheck::Unsupported);
                    }
                    if !invoked_scalars.insert(target) { return Err(ProjectCheck::Unsupported); }
                    scalar_observations = scalar_observations.checked_add(sites.len()).ok_or(ProjectCheck::Unsupported)?;
                }
                Ranked => {
                    let plan = rank.filter(|plan| plan.target == target).ok_or(ProjectCheck::Unsupported)?;
                    verified_rank = Some(prepare_capped_ranked_receipts(plan)?);
                    trace.ranked_target = Some(target);
                    trace.metered_frames |= sites.len() > 1;
                },
                failure_search::StartupProofRole::Cited => return Err(ProjectCheck::Unsupported),
            }
            trace.enroll_proof_sites(target, role, sites).ok_or(ProjectCheck::Unsupported)?;
        }
        trace.metered_frames |= scalar_observations > 1;
        deferred_arrays.sort_unstable();
        let mut effects = OrderedFailures { startup_search: Some(Rc::new(RefCell::new(trace))), ..OrderedFailures::default() };
        let (init, evidence) = walk_prepared_initialization::<_, true, true>(
            prepared, root, ordinals, Evidence::new(self)?, Some(&mut effects),
        );
        let mut trace = Rc::try_unwrap(effects.startup_search.take().ok_or(ProjectCheck::Unsupported)?)
            .map_err(|_| ProjectCheck::Unsupported)?.into_inner();
        if trace.invalid() || !trace.required_sites.is_empty() || evidence.seen != reached_arrays
            || effects.has_authored_state || effects.has_selected_code || !effects.snapshot_records.is_empty()
            || cap.map_or(0, std::num::NonZeroUsize::get).checked_sub(trace.remaining()) != Some(effects.records.len())
        { return Err(ProjectCheck::Unsupported); }
        let (completion, mut active) = match init {
            Ok(init) if cap.is_none() => {
                let applicable = evidence.complete_startup(&init.reached)?;
                (StartupAuthoredCompletion::LimitReached, init.had_obligations || applicable)
            },
            Ok(init) if trace.remaining() > 0 => {
                let applicable = evidence.complete_startup(&init.reached)?;
                (StartupAuthoredCompletion::Exhausted, init.had_obligations || applicable)
            },
            Err(ProjectCheck::StartupAuthoredLimitReached) if trace.remaining() == 0 =>
                (StartupAuthoredCompletion::LimitReached, true),
            Err(error) => return Err(error),
            _ => return Err(ProjectCheck::Unsupported),
        };
        if let Some(verified) = &mut verified_rank {
            check_capped_ranked_receipts(verified, &mut trace, root)?;
            active = true;
        }
        let bare_evidence = scalar.map(|mut observer| {
            if trace.named_observations.len() != scalar_observations { return Err(ProjectCheck::Unsupported); }
            if bare.len() == 1 {
                observe_capped_bare_receipts(&mut observer, &mut trace, invoked_scalars.iter().copied().next(), root)?;
            } else {
                for observed in std::mem::take(&mut trace.named_observations) {
                    observe_capped_bare_receipt(&mut observer, &mut trace, &invoked_scalars, root, observed)?;
                }
                if !observer.complete(&invoked_scalars) { return Err(ProjectCheck::Unsupported); }
            }
            active |= observer.active;
            Ok::<_, ProjectCheck>(NamedOrderedEvidence { observed: observer })
        }).transpose()?;
        let ((diagnostic, additional), bare_evidence) = if scope == ScalarCheckerScope::CurrentPackage {
            let mut ranked = HashMap::new();
            let mut deferred_numeric = bare.iter().filter(|plan| !invoked_scalars.contains(&plan.target))
                .collect::<Vec<_>>();
            if let Some(plan) = rank.filter(|_| verified_rank.is_none()) {
                ranked.insert(plan.target, closed_ranked_evidence::<false>(root, 1, plan,
                    plan.typed.measure.ok_or(ProjectCheck::Unsupported)?)?);
                deferred_numeric.push(plan);
            }
            let numeric = MixedOrderedEvidence { ranked, bare: bare_evidence };
            let mut symbolic = numeric.symbolic();
            for ranked in symbolic.ranked.values_mut() { ranked.allow_summary = false; }
            deferred_numeric.sort_unstable_by_key(|plan| plan.target);
            for plan in deferred_numeric {
                active |= symbolic.try_collect(plan.target, plan.typed.body, plan.function, plan.proof_source,
                    &mut effects, SymbolicProofRuntime { project, calls, visits: None })?.ok_or(ProjectCheck::Unsupported)?;
            }
            symbolic.complete()?;
            let result = MixedOrderedEvidence::finish_symbolic(symbolic);
            (result, numeric.bare)
        } else { ((None, Vec::new()), bare_evidence) };
        let actual = bare_evidence.map_or_else(Vec::new, |named| named.observed.diagnostics);
        let symbolic = if scope == ScalarCheckerScope::CurrentPackage {
            let mut symbolic = evidence.symbolic();
            // Only the original complete edge census authorizes deferred
            // work here. No replay of a partly observed startup invocation.
            for (ordinal, item) in deferred_arrays {
                let module = project.modules.get(ordinal).ok_or(ProjectCheck::Unsupported)?;
                let Some(ScalarItem::Function(function)) = module.items.get(item) else { return Err(ProjectCheck::Unsupported); };
                let body = calls.function_body(ordinal, item, None).ok_or(ProjectCheck::Unsupported)?;
                active |= symbolic.try_collect((ordinal, item), body, function, &module.source, &mut effects,
                    SymbolicProofRuntime { project, calls, visits: None })?.ok_or(ProjectCheck::Unsupported)?;
            }
            symbolic.complete()?;
            Evidence::finish_symbolic(symbolic)
        } else { Vec::new() };
        let aggregate_diagnostics = evidence.into_diagnostics(scope, symbolic)?;
        Ok((MainFlowProofReceipt {
            active, directives: effects.records, diagnostics: actual, mixed_context_after_scalar: None,
            contextual_diagnostic: None, additional_contextual_diagnostics: Vec::new(),
            aggregate_diagnostics, pointer_effect_diagnostic: None, symbolic_diagnostic: diagnostic,
            additional_symbolic_diagnostics: additional, entry_evidence: None,
        }, completion))
    }
}

pub(super) struct Evidence<'plan, 'proof> {
    cohort: &'plan Cohort<'proof>,
    guards: Vec<Option<CheckedIntegerGuardSlots<'plan>>>,
    seen: HashSet<PreparedCallable>,
    pub(super) applicable: bool,
    pub(super) failures: Vec<AggregateProofDiagnostic>,
    work: usize,
}

impl<'plan, 'proof> Evidence<'plan, 'proof> {
    pub(super) fn new(cohort: &'plan Cohort<'proof>) -> Result<Self, ProjectCheck> {
        let mut work = 0;
        let mut guards = Vec::with_capacity(cohort.plans.len());
        for plan in &cohort.plans {
            let filter = plan.entry_filter.as_deref().unwrap_or_default();
            charge_call_work(&mut work, filter.len().checked_add(1).ok_or(ProjectCheck::Unsupported)?)
                .ok_or(ProjectCheck::Unsupported)?;
            guards.push(checked_scalar_guard_slots::<true>(plan.body, filter, false, true));
        }
        Ok(Self { cohort, guards, seen: HashSet::new(), applicable: false, failures: Vec::new(), work })
    }

    pub(super) fn saw_single_target(&self) -> bool {
        self.cohort.groups.len() == 1 && self.seen.contains(&self.cohort.plans[0].target)
    }

    pub(super) fn into_diagnostics(
        mut self, scope: ScalarCheckerScope, symbolic: Vec<Option<ArrayCoverage>>,
    ) -> Result<Vec<AggregateProofDiagnostic>, ProjectCheck> {
        if scope == ScalarCheckerScope::CurrentPackage {
            if symbolic.len() != self.cohort.plans.len() { return Err(ProjectCheck::Unsupported); }
            for (plan, verdict) in self.cohort.plans.iter().zip(symbolic) {
                match verdict {
                    Some(ProofCoverage::Applicable(Some(failure))) => self.failures.push(failure),
                    None if !self.seen.contains(&plan.target) => return Err(ProjectCheck::Unsupported),
                    _ => {}
                }
            }
        }
        Ok(self.failures)
    }
}

impl ActualIntegerObserver for Evidence<'_, '_> {
    const ENABLED: bool = false;
    const FIXED_ARRAY_INITIALIZERS: bool = true;
    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}

    fn watches_fixed_array(&self, target: PreparedCallable) -> bool { self.cohort.groups.contains_key(&target) }

    fn observe_fixed_array_initializer(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, args: &[&BigInt],
        call: wosy_syntax::SourceSpan, trace: Option<&failure_search::SharedStartupTrace>,
    ) -> Result<Vec<BigInt>, ProjectCheck> {
        let group = self.cohort.groups.get(&target).ok_or(ProjectCheck::Unsupported)?;
        let first = &self.cohort.plans[group.ids[0]];
        charge_call_work(&mut self.work, body.occurrences.len().checked_add(group.ids.len())
            .ok_or(ProjectCheck::Unsupported)?).ok_or(ProjectCheck::Unsupported)?;
        group.validate(first, body)?;
        let mut checks = Vec::with_capacity(group.ids.len());
        let mut applicable = false;
        for &id in &group.ids {
            let plan = &self.cohort.plans[id];
            let guard = self.guards[id].as_ref().ok_or(ProjectCheck::Unsupported)?;
            let cost = guard.comparisons.len().checked_add(guard.domain.map_or(0, |(_, domain)| domain.interval_count()))
                .and_then(|cost| cost.checked_add(1)).ok_or(ProjectCheck::Unsupported)?;
            charge_call_work(&mut self.work, cost).ok_or(ProjectCheck::Unsupported)?;
            let check = plan.check();
            let selected = (guard.accepts(args) && check.applicable()).then_some(check);
            applicable |= selected.is_some();
            checks.push(selected);
        }
        let (returned, failures) = collect_exact_fixed_array_return(
            body, first.result, args, &mut checks, &mut self.work,
            trace.map(|trace| (first.owner, trace)),
        ).ok_or(ProjectCheck::Unsupported)?;
        self.seen.insert(target);
        self.applicable |= applicable;
        for (id, failed) in group.ids.iter().copied().zip(failures) {
            if let Some(failed) = failed {
                let plan = &self.cohort.plans[id];
                self.failures.push(plan.diagnostic(failed, plan.owner, Some(call.clone()))?);
            }
        }
        Ok(returned)
    }
}

impl OrderedRunEvidence for Evidence<'_, '_> {
    type Symbolic<'a> = Symbolic<'a, 'a, 'a> where Self: 'a;
    type SymbolicReport = Vec<Option<ArrayCoverage>>;
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if self.cohort.groups.keys().any(|target| reached.contains(target) != self.seen.contains(target)) {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(self.applicable)
    }
    fn symbolic(&self) -> Self::Symbolic<'_> {
        Symbolic { cohort: self.cohort, guards: &self.guards,
            coverage: (0..self.cohort.plans.len()).map(|_| None).collect(), work: self.work }
    }
    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run { proof.coverage }
}

pub(super) struct Symbolic<'run, 'plan, 'proof> {
    cohort: &'plan Cohort<'proof>,
    guards: &'run [Option<CheckedIntegerGuardSlots<'plan>>],
    coverage: Vec<Option<ArrayCoverage>>,
    work: usize,
}

impl SymbolicRootProof for Symbolic<'_, '_, '_> {
    const ENABLED: bool = true;
    const FIXED_ARRAY_RETURN: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        for target in self.cohort.groups.keys() {
            if !reached.contains(target) {
                roots.entry(target.0).or_default().insert(target.1);
            }
        }
    }

    fn try_collect(&mut self, target: PreparedCallable, body: &FunctionCallSites, _: &ScalarFunction,
        source: &SourceIdentity, _: &mut OrderedFailures, _: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        let Some(group) = self.cohort.groups.get(&target) else { return Ok(None); };
        let first = &self.cohort.plans[group.ids[0]];
        if source != first.owner { return Err(ProjectCheck::Unsupported); }
        charge_call_work(&mut self.work, body.occurrences.len().checked_add(group.ids.len())
            .ok_or(ProjectCheck::Unsupported)?).ok_or(ProjectCheck::Unsupported)?;
        group.validate(first, body)?;
        let entry = self.symbolic_entry(group, first)?;
        let mut checks = Vec::with_capacity(group.ids.len());
        for (index, &id) in group.ids.iter().enumerate() {
            let enabled = !matches!(&entry, SymbolicEntry::Nullary)
                || self.guards[id].as_ref().ok_or(ProjectCheck::Unsupported)?.accepts(&[]);
            let domain = match &entry {
                SymbolicEntry::Guarded { arity, rectangles } => Some(&rectangles[index][..*arity]),
                _ => None,
            };
            checks.push(FixedArraySymbolicCheck { check: self.cohort.plans[id].check(), enabled, entry: domain });
        }
        let no_nullary_claims = [CheckerPredicate::Boolean(false)];
        let entries = match &entry {
            SymbolicEntry::Shared(filter) => FixedArrayEntries::Common(filter),
            SymbolicEntry::Guarded { arity, rectangles } => FixedArrayEntries::Guarded { arity: *arity, rectangles },
            SymbolicEntry::Nullary if checks.iter().any(|check| check.enabled) => FixedArrayEntries::Common(&[]),
            SymbolicEntry::Nullary => FixedArrayEntries::Common(&no_nullary_claims),
        };
        let coverage = observe_fixed_array_returns(body, first.result, entries, &checks, &mut self.work)
            .ok_or(ProjectCheck::Unsupported)?;
        let mut applicable = false;
        for (id, verdict) in group.ids.iter().copied().zip(coverage) {
            self.coverage[id] = Some(match verdict {
                ProofCoverage::NoApplicableInputs => ProofCoverage::NoApplicableInputs,
                ProofCoverage::Applicable(failure) => {
                    applicable = true;
                    let diagnostic = failure.map(|failure| self.cohort.plans[id].diagnostic(failure, source, None)).transpose()?;
                    ProofCoverage::Applicable(diagnostic)
                }
            });
        }
        Ok(Some(applicable))
    }
}

enum SymbolicEntry<'a> {
    Shared(&'a [CheckerPredicate]),
    Nullary,
    Guarded { arity: usize, rectangles: Vec<[IntegerSet; 2]> },
}

impl Symbolic<'_, '_, '_> {
    fn symbolic_entry<'a>(
        &mut self, group: &Group, first: &'a ArrayProof<'_>,
    ) -> Result<SymbolicEntry<'a>, ProjectCheck> {
        let common = first.entry_filter.as_deref().unwrap_or_default();
        let mut same = true;
        for &id in &group.ids[1..] {
            let filter = self.cohort.plans[id].entry_filter.as_deref().unwrap_or_default();
            let guards = filter.iter().chain(common).try_fold(0usize, |cost, predicate| {
                cost.checked_add(match predicate {
                    CheckerPredicate::EntryDomain { values, .. } => values.interval_count().checked_add(1)?,
                    _ => 1,
                })
            }).ok_or(ProjectCheck::Unsupported)?;
            charge_call_work(&mut self.work, guards.checked_add(1).ok_or(ProjectCheck::Unsupported)?)
                .ok_or(ProjectCheck::Unsupported)?;
            if common != filter { same = false; break; }
        }
        if same { return Ok(SymbolicEntry::Shared(common)); }
        if first.body.boundary.parameters.is_empty() { return Ok(SymbolicEntry::Nullary); }

        let arity = first.body.boundary.parameters.len();
        if !matches!(arity, 1 | 2) { return Err(ProjectCheck::Unsupported); }
        let rectangles = group.ids.iter().map(|&id| {
            self.guards[id].as_ref().and_then(|guard| guard.integer_rectangle(first.body, &mut self.work))
                .ok_or(ProjectCheck::Unsupported)
        }).collect::<Result<Vec<_>, _>>()?;
        Ok(SymbolicEntry::Guarded { arity, rectangles })
    }
}
