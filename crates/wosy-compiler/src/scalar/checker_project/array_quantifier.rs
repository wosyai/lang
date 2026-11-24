//! Quantified claims about prepared immutable fixed-array return cells.
use super::*;
use super::super::checker_flow::{
    charge_call_work, collect_exact_fixed_array_return, observe_fixed_array_returns,
    FixedArrayCheck, FixedArrayQuantifier, FixedArrayReturnFailure,
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
    supported_body: std::cell::OnceCell<bool>,
    result: &'a ScalarType,
}

pub(super) fn is_array_target(project: &PreparedProofModule, proof: &SelectedModuleProof<'_>) -> bool {
    project.prepared.parts().1.function_body(proof.function.0, proof.function.1, None)
        .is_some_and(|body| matches!(body.boundary.results.as_slice(), [out] if matches!(out.ty, ScalarType::Array { .. })))
}

impl<'a> ArrayProof<'a> {
    pub(super) fn prepare(
        project: &'a PreparedProofModule, source: &'a SourceIdentity, selected: SelectedModuleProof<'a>,
    ) -> Result<Self, ProjectCheck> {
        let proof = selected.node;
        let target = (selected.function.0, selected.function.1);
        let (project, calls) = project.prepared.parts();
        let owner = &project.modules[target.0];
        let body = calls.function_body(target.0, target.1, None).ok_or(ProjectCheck::Unsupported)?;
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
        let mut parameters = target_node.children_with_tokens().filter(|entry| entry.kind() == SyntaxKind::Parameters);
        let inputs = match parameters.next() {
            Some(entry) => entry.into_node().map_or_else(Vec::new, |node| identifiers(&node).collect::<Vec<_>>()),
            None => return Err(ProjectCheck::InvalidProof),
        };
        if parameters.next().is_some() || inputs.len() != body.boundary.parameters.len() {
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
            supported_body: std::cell::OnceCell::new(), result: &output.ty })
    }

    fn check(&self) -> FixedArrayCheck<'_> {
        FixedArrayCheck {
            domain: &self.domain, quantifier: self.quantifier.as_ref(),
            relation: self.relation, expected: &self.expected,
        }
    }

    fn validate_body(&self, body: &FunctionCallSites) -> Result<(), ProjectCheck> {
        if !std::ptr::eq(body, self.body) { return Err(ProjectCheck::Unsupported); }
        let supported = self.supported_body.get_or_init(|| {
            let structure = &body.structure;
            body.call_occurrences.is_empty() && body.assignments.is_empty() && body.arms.is_empty()
                && body.allocation_length_calls.is_empty() && structure.checker_directives.is_empty()
                && !structure.has_ordinary_contextual_places() && !structure.checker_has_bool_contextual_declaration
                && !body.conditionals.values().any(|region| matches!(region, ConditionalRegion::Unit { .. }))
        });
        supported.then_some(()).ok_or(ProjectCheck::Unsupported)
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

    fn verify_return(
        &self, body: &FunctionCallSites, source: &SourceIdentity,
    ) -> Result<ProofCoverage<Option<AggregateProofDiagnostic>>, ProjectCheck> {
        self.validate_body(body)?;
        match observe_fixed_array_returns(body, self.result, self.entry_filter.as_deref(), self.check())
            .ok_or(ProjectCheck::Unsupported)?
        {
            ProofCoverage::NoApplicableInputs => Ok(ProofCoverage::NoApplicableInputs),
            ProofCoverage::Applicable(None) => Ok(ProofCoverage::Applicable(None)),
            ProofCoverage::Applicable(Some(failure)) =>
                Ok(ProofCoverage::Applicable(Some(self.diagnostic(failure, source, None)?))),
        }
    }
}

pub(super) struct Evidence<'plan, 'proof> {
    plan: &'plan ArrayProof<'proof>,
    guard: Option<CheckedIntegerGuardSlots<'plan>>,
    pub(super) seen: bool,
    pub(super) applicable: bool,
    pub(super) failures: Vec<AggregateProofDiagnostic>,
    work: usize,
}

impl<'plan, 'proof> Evidence<'plan, 'proof> {
    pub(super) fn new(plan: &'plan ArrayProof<'proof>) -> Self {
        let guard = checked_scalar_guard_slots::<true>(plan.body, plan.entry_filter.as_deref().unwrap_or_default(), false, true);
        Self { plan, guard, seen: false, applicable: false, failures: Vec::new(), work: 0 }
    }
}

impl ActualIntegerObserver for Evidence<'_, '_> {
    const ENABLED: bool = false;
    const FIXED_ARRAY_INITIALIZERS: bool = true;
    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}

    fn watches_fixed_array(&self, target: PreparedCallable) -> bool { target == self.plan.target }

    fn observe_fixed_array_initializer(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, args: &[&BigInt], call: wosy_syntax::SourceSpan,
    ) -> Result<Vec<BigInt>, ProjectCheck> {
        if !self.watches_fixed_array(target) || &call.source != self.plan.source {
            return Err(ProjectCheck::Unsupported);
        }
        let guard = self.guard.as_ref().ok_or(ProjectCheck::Unsupported)?;
        let cost = body.occurrences.len().checked_add(guard.comparisons.len())
            .and_then(|cost| cost.checked_add(guard.domain.map_or(0, |(_, domain)| domain.interval_count())))
            .and_then(|cost| cost.checked_add(1)).ok_or(ProjectCheck::Unsupported)?;
        charge_call_work(&mut self.work, cost).ok_or(ProjectCheck::Unsupported)?;
        self.plan.validate_body(body)?;
        let check = self.plan.check();
        let applicable = guard.accepts(args) && check.applicable();
        let (returned, failure) = collect_exact_fixed_array_return(
            body, self.plan.result, args, applicable.then_some(check), &mut self.work,
        ).ok_or(ProjectCheck::Unsupported)?;
        self.seen = true;
        self.applicable |= applicable;
        if let Some(failure) = failure {
            let source = self.plan.source;
            self.failures.push(self.plan.diagnostic(failure, source, Some(call))?);
        }
        Ok(returned)
    }
}

impl OrderedRunEvidence for Evidence<'_, '_> {
    type Symbolic<'a> = Symbolic<'a> where Self: 'a;
    type SymbolicReport = Option<ProofCoverage<Option<AggregateProofDiagnostic>>>;
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if reached.contains(&self.plan.target) != self.seen { return Err(ProjectCheck::Unsupported); }
        Ok(self.applicable)
    }
    fn symbolic(&self) -> Self::Symbolic<'_> { Symbolic { plan: self.plan, coverage: None } }
    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run { proof.coverage }
}

pub(super) struct Symbolic<'a> { plan: &'a ArrayProof<'a>, coverage: Option<ProofCoverage<Option<AggregateProofDiagnostic>>> }

impl SymbolicRootProof for Symbolic<'_> {
    const ENABLED: bool = true;
    const FIXED_ARRAY_RETURN: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        if !reached.contains(&self.plan.target) {
            roots.entry(self.plan.target.0).or_default().insert(self.plan.target.1);
        }
    }

    fn try_collect(&mut self, target: PreparedCallable, body: &FunctionCallSites, _: &ScalarFunction,
        source: &SourceIdentity, _: &mut OrderedFailures, _: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        if target != self.plan.target { return Ok(None); }
        let coverage = self.plan.verify_return(body, source)?;
        let applicable = matches!(coverage, ProofCoverage::Applicable(_));
        self.coverage = Some(coverage);
        Ok(Some(applicable))
    }
}
