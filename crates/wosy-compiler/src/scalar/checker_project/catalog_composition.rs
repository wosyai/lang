//! Join disjoint typed proof receivers above one ordered checker execution.
use super::*;

#[derive(Default)]
pub(super) struct ScalarDiagnostics {
    actual: Vec<MainFlowProofDiagnostic>,
    symbolic: Option<SymbolicProofDiagnostic>,
    additional: Vec<SymbolicProofDiagnostic>,
}

// These are the closed scalar catalog profiles. Do not merge a stronger
// initializer policy into them merely because an array plan also exists.
pub(super) trait ScalarCatalogEvidence: OrderedRunEvidence {
    fn into_diagnostics(self, symbolic: Self::SymbolicReport) -> ScalarDiagnostics;
}

impl ScalarCatalogEvidence for () {
    fn into_diagnostics(self, _: ()) -> ScalarDiagnostics { ScalarDiagnostics::default() }
}

impl ScalarCatalogEvidence for NamedOrderedEvidence<'_> {
    fn into_diagnostics(self, (symbolic, additional): Self::SymbolicReport) -> ScalarDiagnostics {
        ScalarDiagnostics { actual: self.observed.diagnostics, symbolic, additional }
    }
}

impl ScalarCatalogEvidence for RankedOrderedEvidence<'_, '_> {
    fn into_diagnostics(self, symbolic: Self::SymbolicReport) -> ScalarDiagnostics {
        ScalarDiagnostics { symbolic, ..ScalarDiagnostics::default() }
    }
}

impl ScalarCatalogEvidence for MixedOrderedEvidence<'_, '_> {
    fn into_diagnostics(self, (symbolic, additional): Self::SymbolicReport) -> ScalarDiagnostics {
        let actual = self.bare.map_or_else(Vec::new, |bare| bare.observed.diagnostics);
        ScalarDiagnostics { actual, symbolic, additional }
    }
}

struct WithArrays<'plan, 'proof, Scalar> {
    scalar: Scalar,
    array: array_quantifier::Evidence<'plan, 'proof>,
}

impl<Scalar: ScalarCatalogEvidence> ActualIntegerObserver for WithArrays<'_, '_, Scalar> {
    const ENABLED: bool = Scalar::ENABLED;
    const CONTEXTUAL: bool = Scalar::CONTEXTUAL;
    const FIXED_ARRAY_INITIALIZERS: bool = true;

    fn observe(&mut self, target: PreparedCallable, arguments: &[&BigInt], value: &BigInt) {
        self.scalar.observe(target, arguments, value);
    }

    fn charge_named_exact_work(&mut self, cost: usize) -> Option<()> {
        self.scalar.charge_named_exact_work(cost)
    }

    fn named_exact_watch(&self, target: PreparedCallable) -> Option<NamedExactWatch> {
        self.scalar.named_exact_watch(target)
    }

    fn enables_character_initializers(&self) -> bool {
        self.scalar.enables_character_initializers()
    }

    fn check_initializer_pair(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, site: InitializerProofSite<'_>,
        parameters: &[ScalarType], arguments: &[&BigInt],
    ) -> Result<[BigInt; 2], ProjectCheck> {
        self.scalar.check_initializer_pair(target, body, site, parameters, arguments)
    }

    fn permits_named_pure_affine_return(&self, target: PreparedCallable, arguments: &[&BigInt]) -> bool {
        self.scalar.permits_named_pure_affine_return(target, arguments)
    }

    fn observe_initializer(
        &mut self, target: PreparedCallable, site: Option<InitializerProofSite<'_>>,
        parameters: &[ScalarType], arguments: &[&BigInt], value: &BigInt,
    ) {
        self.scalar.observe_initializer(target, site, parameters, arguments, value);
    }

    fn proved_recursive_return(
        &mut self, target: PreparedCallable, arguments: &[&BigInt], expected: &ScalarType,
    ) -> Option<BigInt> {
        self.scalar.proved_recursive_return(target, arguments, expected)
    }

    fn watches_fixed_array(&self, target: PreparedCallable) -> bool {
        self.array.watches_fixed_array(target)
    }

    fn observe_fixed_array_initializer(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, arguments: &[&BigInt], call: wosy_syntax::SourceSpan,
    ) -> Result<Vec<BigInt>, ProjectCheck> {
        self.array.observe_fixed_array_initializer(target, body, arguments, call)
    }
}

struct SymbolicWithArrays<Scalar, Array> {
    scalar: Scalar,
    array: Array,
}

impl<Scalar: SymbolicRootProof, Array: SymbolicRootProof> SymbolicRootProof
    for SymbolicWithArrays<Scalar, Array>
{
    const ENABLED: bool = Scalar::ENABLED || Array::ENABLED;
    const INTEGER_RETURN_PARTITIONS: bool = Scalar::INTEGER_RETURN_PARTITIONS;
    const FIXED_ARRAY_RETURN: bool = Array::FIXED_ARRAY_RETURN;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        self.scalar.seed(roots, reached);
        self.array.seed(roots, reached);
    }

    fn watches_literal_leaf(&self, target: PreparedCallable) -> bool {
        self.scalar.watches_literal_leaf(target)
    }

    fn watches_result_pair(&self, target: PreparedCallable) -> bool {
        self.scalar.watches_result_pair(target)
    }

    fn try_collect(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, effects: &mut OrderedFailures, runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        if matches!(body.boundary.results.as_slice(), [output] if matches!(output.ty, ScalarType::Array { .. })) {
            self.array.try_collect(target, body, function, source, effects, runtime)
        } else {
            self.scalar.try_collect(target, body, function, source, effects, runtime)
        }
    }
}

impl<'plan, 'proof, Scalar: ScalarCatalogEvidence> OrderedRunEvidence for WithArrays<'plan, 'proof, Scalar> {
    type Symbolic<'run> =
        SymbolicWithArrays<Scalar::Symbolic<'run>, <array_quantifier::Evidence<'plan, 'proof> as OrderedRunEvidence>::Symbolic<'run>>
        where Self: 'run;
    type SymbolicReport =
        (Scalar::SymbolicReport, <array_quantifier::Evidence<'plan, 'proof> as OrderedRunEvidence>::SymbolicReport);
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        Ok(self.scalar.complete_startup(reached)? | self.array.complete_startup(reached)?)
    }

    fn symbolic(&self) -> Self::Symbolic<'_> {
        SymbolicWithArrays { scalar: self.scalar.symbolic(), array: self.array.symbolic() }
    }

    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run {
        (Scalar::finish_symbolic(proof.scalar), array_quantifier::Evidence::finish_symbolic(proof.array))
    }
}

pub(super) fn run<Scalar: ScalarCatalogEvidence>(
    prepared: &PreparedProofModule, scope: ScalarCheckerScope, startup_count: usize,
    scalar: Scalar, arrays: Option<&array_quantifier::Cohort<'_>>,
) -> Result<MainFlowProofReceipt, ProjectCheck> {
    let (active, directives, scalar, aggregate_diagnostics, entry_evidence) = if let Some(arrays) = arrays {
        let evidence = WithArrays { scalar, array: array_quantifier::Evidence::new(arrays)? };
        let run = run_prepared_ordered_check(&prepared.prepared, scope, startup_count, OrderedReachPolicy::Closed, evidence)?;
        let (scalar, array) = run.symbolic;
        let aggregate = run.actual.array.into_diagnostics(scope, array)?;
        (run.active, run.directives, run.actual.scalar.into_diagnostics(scalar), aggregate, run.entry_evidence)
    } else {
        let run = run_prepared_ordered_check(&prepared.prepared, scope, startup_count, OrderedReachPolicy::Closed, scalar)?;
        (run.active, run.directives, run.actual.into_diagnostics(run.symbolic), Vec::new(), run.entry_evidence)
    };
    Ok(MainFlowProofReceipt {
        active, directives, diagnostics: scalar.actual, contextual_diagnostic: None,
        aggregate_diagnostics, symbolic_diagnostic: scalar.symbolic,
        additional_symbolic_diagnostics: scalar.additional, entry_evidence,
    })
}
