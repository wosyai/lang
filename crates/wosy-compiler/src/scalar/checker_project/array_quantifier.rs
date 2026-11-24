//! Checked observations of ordinary immutable fixed-array return constructors.
use super::*;
use super::super::reference::{OccurrenceEdge, OccurrenceNode, OccurrenceParent};

pub(super) struct ArrayProof<'a> {
    proof: &'a CstNode,
    source: &'a SourceIdentity,
    target: PreparedCallable,
    goal: ByteSpan,
    failed_step: Option<ByteSpan>,
    domain: IntegerSet,
    witness: Option<BigInt>,
    expected: BigInt,
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
        if !body.boundary.parameters.is_empty() || !is_integer_type(element)
            || !matches!(selected.target, ResolvedCallTarget::ModuleCallable { source, declaration_span,
                concrete: ConcreteCallSelection::Declaration } if source == &owner.source
                    && *declaration_span == selected.function.2.name_span)
            || owner.source.project != source.project || owner.source.package != source.package
            || proof.children().any(|node| matches!(node.kind(), SyntaxKind::ProofWhere | SyntaxKind::ProofDecreases))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let target_node = child(proof, SyntaxKind::ProofTarget)?;
        let mut parameters = target_node.children_with_tokens().filter(|entry| entry.kind() == SyntaxKind::Parameters);
        if !parameters.next().is_some_and(|entry| entry.into_token().is_some()) || parameters.next().is_some() {
            return Err(ProjectCheck::InvalidProof);
        }
        let outputs = child(&child(proof, SyntaxKind::ProofOutputs)?, SyntaxKind::Parameters)?;
        let mut result_names = identifiers(&outputs);
        let binding = result_names.next().ok_or(ProjectCheck::InvalidProof)?;
        if result_names.next().is_some() { return Err(ProjectCheck::InvalidProof); }

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
        let domain = typed_quantifier_domain(&quantifier, &binder, &[], &index_ty)?;
        let goal_node = child(if existential { &quantifier } else { &ensures }, SyntaxKind::Expression)?;
        // The ordinary scalar derivation is recursive; reject before handling
        // a deep or oversized language shape outside this narrow proposition.
        let claim = |node: &CstNode| -> Result<ScalarExpression, ProjectCheck> {
            if node.descendants().take(MAX_ACTUAL_CALL_DEPTH + 1).count() > MAX_ACTUAL_CALL_DEPTH {
                return Err(ProjectCheck::Unsupported);
            }
            Ok(derive_expression(node))
        };
        let checked_claim = |expression: &ScalarExpression| -> Result<BigInt, ProjectCheck> {
            let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } = expression else {
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
            let literal = concrete_integer_literal(right).ok_or(ProjectCheck::Unsupported)?;
            integer_literal_fits_type(literal.as_ref(), element).then(|| literal.into_owned())
                .ok_or(ProjectCheck::InvalidProof)
        };
        let expected = checked_claim(&claim(&goal_node)?)?;
        let block = child(proof, SyntaxKind::Block)?;
        if block.descendants().take(MAX_ACTUAL_CALL_DEPTH + 1).count() > MAX_ACTUAL_CALL_DEPTH {
            return Err(ProjectCheck::Unsupported);
        }
        let steps = derive_block(&block);
        let mut failed_step = None;
        let witness = if existential {
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
            Some(value)
        } else {
            if !steps.items.is_empty() {
                let ([item], [true]) = (steps.items.as_slice(), steps.terminated_items.as_slice()) else {
                    return Err(ProjectCheck::Unsupported);
                };
                let shown = proof_show_expression(item, true)?;
                if checked_claim(shown)? != expected { return Err(ProjectCheck::Unsupported); }
                let ScalarBlockItem::Expression(expression) = item else { return Err(ProjectCheck::Unsupported); };
                failed_step = Some(super::super::typecheck::expression_span(expression));
            }
            if domain.extremes().is_some_and(|(first, last)| u64::try_from(first).is_err()
                || u64::try_from(last).ok().is_none_or(|index| index >= *length))
            {
                return Err(ProjectCheck::Unsupported);
            }
            None
        };
        Ok(Self { proof, source, target, goal: wosy_syntax::byte_span(&goal_node), failed_step,
            domain, witness, expected, result: &output.ty })
    }

    fn verify_return(
        &self, body: &FunctionCallSites, function: &ScalarFunction, source: &SourceIdentity,
    ) -> Result<Option<AggregateProofDiagnostic>, ProjectCheck> {
        let ScalarType::Array { element, length, .. } = self.result else { return Err(ProjectCheck::Unsupported); };
        let structure = &body.structure;
        if !body.call_occurrences.is_empty() || !body.assignments.is_empty() || !body.arms.is_empty()
            || !body.allocation_length_calls.is_empty() || !structure.checker_directives.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        let [item] = structure.blocks.get(structure.root.0).ok_or(ProjectCheck::Unsupported)?.items.as_slice() else {
            return Err(ProjectCheck::Unsupported);
        };
        let BlockItemKind::FinalOutput(slot) = &item.kind else { return Err(ProjectCheck::Unsupported); };
        let mut outputs = function.body.final_expressions();
        let Some((ScalarExpression::ArrayLiteral { elements, span }, _)) = outputs.next() else {
            return Err(ProjectCheck::Unsupported);
        };
        if slot.output != 0 || outputs.next().is_some() || !structure.checked_types.expressions.get(&slot.expression)
            .is_some_and(|ty| super::super::scalar_type_equal(ty, self.result))
            || body.occurrences.get(slot.expression.0).map(|producer| producer.span) != Some(*span)
            || u64::try_from(elements.len()).ok() != Some(*length)
        {
            return Err(ProjectCheck::Unsupported);
        }
        let mut work = 0;
        super::super::checker_flow::charge_call_work(&mut work,
            elements.len().checked_mul(self.domain.interval_count().checked_add(4).ok_or(ProjectCheck::Unsupported)?)
                .ok_or(ProjectCheck::Unsupported)?).ok_or(ProjectCheck::Unsupported)?;
        let mut first = None;
        for (index, value) in elements.iter().enumerate() {
            let Some(OccurrenceNode::Expression(occurrence)) = structure.edges.get(&(
                OccurrenceParent::Expression(slot.expression), OccurrenceEdge::AggregateElement(index),
            )) else { return Err(ProjectCheck::Unsupported); };
            let span = body.occurrences.get(occurrence.0).ok_or(ProjectCheck::Unsupported)?.span;
            if structure.checked_types.expressions.get(occurrence) != Some(element)
                || span != super::super::typecheck::expression_span(value)
            {
                return Err(ProjectCheck::Unsupported);
            }
            let actual = concrete_integer_literal(value).ok_or(ProjectCheck::Unsupported)?;
            if !integer_literal_fits_type(actual.as_ref(), element) { return Err(ProjectCheck::Unsupported); }
            let index = BigInt::from(index);
            if first.is_none() && (self.witness.as_ref().map_or_else(
                || self.domain.contains(&index), |chosen| chosen == &index,
            )) && actual.as_ref() != &self.expected {
                let at = |span| wosy_syntax::SourceSpan::new(self.source.clone(), span);
                first = Some(AggregateProofDiagnostic {
                    proof: at(wosy_syntax::byte_span(self.proof)), goal: at(self.goal),
                    failed_step: self.failed_step.map(at),
                    output: wosy_syntax::SourceSpan::new(source.clone(), span), index, value: actual.into_owned(),
                });
            }
        }
        Ok(first)
    }
}

pub(super) struct Evidence<'a> { pub(super) plan: ArrayProof<'a> }

impl ActualIntegerObserver for Evidence<'_> {
    const ENABLED: bool = false;
    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}
}

impl OrderedRunEvidence for Evidence<'_> {
    type Symbolic<'a> = Symbolic<'a> where Self: 'a;
    type SymbolicReport = Option<ProofCoverage<Option<AggregateProofDiagnostic>>>;
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if reached.contains(&self.plan.target) { return Err(ProjectCheck::Unsupported); }
        Ok(false)
    }
    fn symbolic(&self) -> Self::Symbolic<'_> { Symbolic { plan: &self.plan, coverage: None } }
    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run { proof.coverage }
}

pub(super) struct Symbolic<'a> { plan: &'a ArrayProof<'a>, coverage: Option<ProofCoverage<Option<AggregateProofDiagnostic>>> }

impl SymbolicRootProof for Symbolic<'_> {
    const ENABLED: bool = true;
    const FIXED_ARRAY_RETURN: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, _: &HashSet<PreparedCallable>) {
        roots.entry(self.plan.target.0).or_default().insert(self.plan.target.1);
    }

    fn try_collect(&mut self, target: PreparedCallable, body: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, _: &mut OrderedFailures, _: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        if target != self.plan.target { return Ok(None); }
        let failure = self.plan.verify_return(body, function, source)?;
        let applicable = !self.plan.domain.is_empty();
        self.coverage = Some(if applicable { ProofCoverage::Applicable(failure) } else { ProofCoverage::NoApplicableInputs });
        Ok(Some(applicable))
    }
}
