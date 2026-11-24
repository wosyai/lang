//! Quantified claims about prepared immutable fixed-array return cells.
use super::*;
use super::super::checker_flow::{
    charge_call_work, collect_exact_fixed_array_return, observe_fixed_array_returns,
    FixedArrayCheck, FixedArrayEntries, FixedArrayQuantifier, FixedArrayReturnFailure, FixedArraySymbolicCheck,
};

pub(super) struct ArrayProof<'a> {
    proof: &'a CstNode,
    source: &'a SourceIdentity,
    target: PreparedCallable,
    goal: ByteSpan,
    failed_step: Option<ByteSpan>,
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
        source: &'a SourceIdentity, selected: SelectedModuleProof<'a>,
    ) -> Result<Self, ProjectCheck> {
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
        let quantifier = match (exists, foralls.next(), foralls.next()) {
            (Some(exists), None, None) => (exists, true),
            (None, Some(forall), None) => (forall, false),
            _ => return Err(ProjectCheck::Unsupported),
        };
        let (quantifier, existential) = quantifier;
        let index_ty = super::super::derive_type(&child(&quantifier, SyntaxKind::TypeSpec)?);
        let mut names = identifiers(&quantifier);
        let binder = names.next().ok_or(ProjectCheck::InvalidProof)?;
        if index_ty != ScalarType::U64 || binder == binding || names.next().is_some()
            || is_null_literal_name(&binder) || is_null_literal_name(&binding)
        {
            return Err(ProjectCheck::InvalidProof);
        }
        let domain = typed_quantifier_domain(&quantifier, &binder, &inputs, &index_ty)?;
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
        let mut failed_step = None;
        let quantifier_kind = if existential && steps.items.is_empty() {
            FixedArrayQuantifier::FindWitness
        } else if existential {
            let value = concrete_integer_literal(authored_witness_argument(&steps)?)
                .ok_or(ProjectCheck::Unsupported)?.into_owned();
            if !integer_literal_fits_type(&value, &index_ty) || !domain.contains(&value)
                || u64::try_from(&value).ok().is_none_or(|value| value >= *length)
            {
                return Err(ProjectCheck::InvalidProof);
            }
            let [ScalarBlockItem::Expression(expression)] = steps.items.as_slice() else {
                return Err(ProjectCheck::Unsupported);
            };
            failed_step = Some(super::super::typecheck::expression_span(expression));
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
        if !matches!(quantifier_kind, FixedArrayQuantifier::ExplicitWitness(_))
            && domain.extremes().is_some_and(|(first, last)| u64::try_from(first).is_err()
                || u64::try_from(last).ok().is_none_or(|index| index >= *length))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let goal = if matches!(quantifier_kind, FixedArrayQuantifier::FindWitness) { &quantifier } else { &goal_node };
        Ok(Self { proof, source, target, goal: wosy_syntax::byte_span(goal), failed_step,
            entry_filter, domain, quantifier: quantifier_kind, relation, expected, body,
            result: &output.ty })
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
            cell: failure.cell.map(|(index, value)| AggregateProofCell { index: BigInt::from(index), value }),
            witness: ScalarCheckerIntegerWitness { equal_axes, axes, state: failure.inputs },
        })
    }

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
    pub(super) fn new(plans: Vec<ArrayProof<'a>>) -> Result<Self, ProjectCheck> {
        let mut groups = HashMap::<PreparedCallable, Group>::new();
        let mut work = 0;
        charge_call_work(&mut work, plans.len()).ok_or(ProjectCheck::Unsupported)?;
        for (id, plan) in plans.iter().enumerate() {
            let group = groups.entry(plan.target).or_insert_with(|| Group {
                ids: Vec::new(), supported_body: std::cell::OnceCell::new(),
            });
            if group.ids.first().is_some_and(|&prior| !std::ptr::eq(plan.body, plans[prior].body)) {
                return Err(ProjectCheck::Unsupported);
            }
            group.ids.push(id);
        }
        Ok(Self { plans, groups })
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
        self.cohort.plans.len() == 1 && self.seen.contains(&self.cohort.plans[0].target)
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
        &mut self, target: PreparedCallable, body: &FunctionCallSites, args: &[&BigInt], call: wosy_syntax::SourceSpan,
    ) -> Result<Vec<BigInt>, ProjectCheck> {
        let group = self.cohort.groups.get(&target).ok_or(ProjectCheck::Unsupported)?;
        let first = &self.cohort.plans[group.ids[0]];
        if &call.source != first.source { return Err(ProjectCheck::Unsupported); }
        charge_call_work(&mut self.work, body.occurrences.len().checked_add(group.ids.len())
            .ok_or(ProjectCheck::Unsupported)?).ok_or(ProjectCheck::Unsupported)?;
        group.validate(first, body)?;
        let mut checks = Vec::with_capacity(group.ids.len());
        let mut applicable = false;
        for &id in &group.ids {
            let plan = &self.cohort.plans[id];
            if &call.source != plan.source { return Err(ProjectCheck::Unsupported); }
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
        ).ok_or(ProjectCheck::Unsupported)?;
        self.seen.insert(target);
        self.applicable |= applicable;
        for (id, failed) in group.ids.iter().copied().zip(failures) {
            if let Some(failed) = failed {
                let plan = &self.cohort.plans[id];
                self.failures.push(plan.diagnostic(failed, plan.source, Some(call.clone()))?);
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
