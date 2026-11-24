use std::{borrow::Cow, cell::RefCell, rc::Rc, collections::{HashMap, HashSet}};

use num_bigint::BigInt;
#[cfg(feature = "internal-checker")]
use std::hash::{Hash, Hasher};

use super::{
    checker_directive::{CheckedLiteralOutput, CheckerOperation, InitializerIntegerOperand, InitializerPredicate, PreparedCheckerDirective},
    checker_flow::{
        check_integer_requirement_activity, check_integer_requirements_with_calls, check_integer_return_bound_with_shows, collect_integer_requirement_failures_with_calls, MAX_ACTUAL_CALL_DEPTH, ExactCallCursor, ExactCallEffect, ExactCallProgress, concrete_integer_literal, exact_typed_call_argument_at,
        exact_integer_return_at_entry, ActualCallResult, CheckerPredicate, ExactIntegerArgument, IntegerRequirementContinuation, IntegerRequirementEntry, NestedIntegerArgument, ProjectedCompanion, ProjectedIntegerArgument,
        proves_requirement_with_imported_root_local, ImportedEntryGuard, ImportedIntegerValues, ImportedLocal, IntegerShowProof, IntegerCheckFailure, IntegerComparison, IntegerInputs, IntegerReturn, ProofCoverage,
    },
    control::{BindingInitialization, BlockItemKind, CheckedCallSignature, ConditionalRegion, InitializationName, InitializationStorage, ModuleInitializationCallTarget, ModuleInitializationDetail, ModuleInitializationStepKind, ModuleInitializationTopology, NamespaceTarget},
    derive_block, derive_expression, project_from_declarations,
    reference::{CheckerIntegerCallOrigin, CheckerTargetQuery, DirectDestinationIdentity, FunctionCallSites, OccurrenceOrdinal, OccurrenceStructure, ProjectCallSiteResults, ResolvedCheckerTarget},
    static_calls::prepare_project_occurrences,
    symbolic_integer::{IntegerRelation, IntegerSet},
    typecheck::{integer_literal_fits_type, is_integer_type, is_null_literal_name, typecheck_proof_where},
    typecheck_scalar_project_in_mode, ByteSpan, ConcreteCallSelection, ReferenceBindingId, ResolvedCallTarget,
    BinaryOperator, ScalarBlock, ScalarBlockItem, ScalarExpression, ScalarFunction, ScalarItem, ScalarModule, ScalarNamespaceBinding, ScalarPreparationInputs,
    ScalarProject, ScalarSourceDeclarations, ScalarTargetLayout, ScalarType, UnaryOperator,
};
#[cfg(test)]
use super::{
    checker_flow::{
        check_integer_requirements, check_integer_return_bound, check_integer_return_bound_where,
    },
};
#[cfg(feature = "internal-checker")]
use super::{checker_flow::{holds_actual_mixed_entry, evaluate_borrowed_return_proof, evaluate_mixed_entry_output, observe_authored_show_returns, integer_predicate_implies, observe_arbitrary_integer_returns, observe_unused_module_proof_returns, ActualFlatShow, GuardedOutputClaims, ImmutableEntryRange, ImmutableProofOutputComparison, IntegerShowOutput}, symbolic_integer::{IntegerIntersection, IntegerShowConstraints, OwnedIntegerUnion}, typecheck::integer_bounds};
#[cfg(not(feature = "internal-checker"))]
use super::checker_flow::collect_integer_requirement_failures;
use crate::Diagnostic;
use wosy_syntax::SourceIdentity;
use wosy_syntax::{CanonicalCstRoot, CstNode, CstToken, SyntaxKind};
#[cfg(test)]
use wosy_syntax::{byte_span, ProofFragmentParse};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScalarCheckerScope {
    #[default]
    CurrentPackage,
    MainFlowOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarCheckerOperation {
    Require,
    Narrow,
}

#[cfg(feature = "internal-checker")]
#[derive(Debug, PartialEq)]
pub struct ScalarCheckerFailure {
    pub source: SourceIdentity,
    pub operation: ScalarCheckerOperation,
    pub directive: ByteSpan,
    pub condition: ByteSpan,
    pub stdout_span: ByteSpan,
    pub stderr_span: ByteSpan,
    pub code_span: ByteSpan,
    pub files_span: ByteSpan,
    pub files: Box<[super::checker_directive::ScalarCheckerTextFile]>,
    pub stdout: Box<[u8]>,
    pub stderr: Box<[u8]>,
    pub code: BigInt,
    pub witness: ScalarCheckerIntegerWitness,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScalarCheckerIntegerWitness {
    equal_axes: Vec<(usize, usize)>,
    axes: Vec<(ReferenceBindingId, ScalarType)>,
    state: IntegerInputs,
}

#[cfg(feature = "internal-checker")]
impl ScalarCheckerIntegerWitness {
    fn remove_snapshot_axes(&mut self, work: &mut usize) -> Option<()> {
        let hidden = |id: &ReferenceBindingId| matches!(id.kind, super::ReferenceBindingKind::CheckerSnapshotField(_));
        super::checker_flow::charge_call_work(work, self.axes.len().checked_mul(2)?)?;
        if !self.axes.iter().any(|(id, _)| hidden(id)) { return Some(()); }
        let mut next = 0;
        let mapping = self.axes.iter().map(|(id, _)| (!hidden(id)).then(|| {
            let index = next;
            next += 1;
            index
        })).collect::<Vec<_>>();
        self.state.remove_snapshot_axes(&mapping, &mut self.equal_axes, work)?;
        self.axes.retain(|(id, _)| !hidden(id));
        Some(())
    }

    pub fn axes(&self) -> impl ExactSizeIterator<Item = (&ReferenceBindingId, &ScalarType)> {
        self.axes.iter().map(|(binding, ty)| (binding, ty))
    }

    pub fn row_count(&self) -> usize {
        self.state.row_count()
    }

    pub fn equal_axes(&self) -> &[(usize, usize)] {
        &self.equal_axes
    }

    pub fn row_equal_axes(&self, row: usize) -> Option<&[(usize, usize)]> {
        self.state.row_equal_axes(row)
    }

    pub fn intervals_at(
        &self,
        row: usize,
        axis: usize,
    ) -> Option<impl ExactSizeIterator<Item = (&BigInt, &BigInt)>> {
        self.state.intervals_at(row, axis)
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Debug, PartialEq)]
pub enum ScalarCheckerOutcome {
    Passed,
    NoChecks,
    Unsupported,
    Invalid(Vec<Diagnostic>),
    Failed(ScalarCheckerFailure),
}

#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub struct ScalarOrderedCheckerFailure {
    pub source: SourceIdentity,
    pub operation: ScalarCheckerOperation,
    pub directive: ByteSpan,
    pub condition: ByteSpan,
    pub witness: ScalarCheckerIntegerWitness,
    output: Rc<CheckedLiteralOutput>,
}

#[cfg(feature = "internal-checker")]
impl ScalarOrderedCheckerFailure {
    pub fn stdout(&self) -> &[u8] { &self.output.stdout_bytes }
    pub fn stderr(&self) -> &[u8] { &self.output.stderr_bytes }
    pub fn code(&self) -> &BigInt { &self.output.code_value }
    pub fn files(&self) -> &[super::checker_directive::ScalarCheckerTextFile] { &self.output.text_files }
    pub fn stdout_span(&self) -> ByteSpan { self.output.stdout }
    pub fn stderr_span(&self) -> ByteSpan { self.output.stderr }
    pub fn code_span(&self) -> ByteSpan { self.output.code }
    pub fn files_span(&self) -> ByteSpan { self.output.files }
}

impl From<CheckerOperation> for ScalarCheckerOperation {
    fn from(operation: CheckerOperation) -> Self {
        match operation {
            CheckerOperation::Require => Self::Require,
            CheckerOperation::Narrow => Self::Narrow,
        }
    }
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub enum ScalarOrderedCheckerOutcome {
    Passed,
    NoChecks,
    Unsupported,
    Invalid(Vec<Diagnostic>),
    Failed(Vec<ScalarOrderedCheckerFailure>),
}

#[cfg(feature = "internal-checker")]
pub fn check_scalar_package(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
) -> ScalarCheckerOutcome {
    check_scalar_package_with_dormant_sources(declarations, initialization_order, Vec::new(), inputs, scope)
}

#[cfg(feature = "internal-checker")]
pub fn check_scalar_package_with_dormant_sources(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    dormant: Vec<ScalarSourceDeclarations>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
) -> ScalarCheckerOutcome {
    let result = prepare_checked_package_scope(declarations, initialization_order, dormant, inputs, scope)
        .and_then(|(prepared, startup_count)| check_package_roots_with_activity(&prepared, scope, startup_count));
    match result {
        Ok(true) => ScalarCheckerOutcome::Passed,
        Ok(false) => ScalarCheckerOutcome::NoChecks,
        Err(ProjectCheck::Invalid(diagnostics)) => ScalarCheckerOutcome::Invalid(diagnostics),
        Err(error) => materialize_checker_failure(error)
            .map_or(ScalarCheckerOutcome::Unsupported, ScalarCheckerOutcome::Failed),
    }
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_scalar_package_ordered_with_dormant_sources(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    dormant: Vec<ScalarSourceDeclarations>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
) -> ScalarOrderedCheckerOutcome {
    let result = prepare_checked_package_scope(declarations, initialization_order, dormant, inputs, scope)
        .and_then(|(prepared, startup_count)| collect_ordered_checker_records(&prepared, scope, startup_count));
    match result {
        Ok((active, failures)) if failures.is_empty() => {
            if active { ScalarOrderedCheckerOutcome::Passed } else { ScalarOrderedCheckerOutcome::NoChecks }
        }
        Ok((_, failures)) => ScalarOrderedCheckerOutcome::Failed(failures),
        Err(ProjectCheck::Invalid(diagnostics)) => ScalarOrderedCheckerOutcome::Invalid(diagnostics),
        Err(_) => ScalarOrderedCheckerOutcome::Unsupported,
    }
}

#[cfg(feature = "internal-checker")]
fn prepare_checked_package_scope(
    mut declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    dormant: Vec<ScalarSourceDeclarations>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
) -> Result<(PreparedCheckerProject, usize), ProjectCheck> {
    let mut raw_sources = HashSet::with_capacity(initialization_order.len());
    if initialization_order.is_empty()
        || initialization_order.len() != declarations.len()
        || !initialization_order.iter().all(|source| raw_sources.insert(source))
    {
        return Err(ProjectCheck::Unsupported);
    }
    let startup_count = declarations.len();
    if scope == ScalarCheckerScope::CurrentPackage && !dormant.is_empty() {
        validate_dormant_source_boundaries(&declarations, &initialization_order[0], |source| raw_sources.contains(source), dormant.iter())?;
        declarations.extend(dormant.into_iter().map(|source| (source, Vec::new())));
    }

    Ok((prepare_checker_project(declarations, initialization_order, inputs)?, startup_count))
}

fn validate_dormant_source_boundaries<'a>(
    declarations: &[(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)],
    root: &SourceIdentity,
    in_startup: impl Fn(&SourceIdentity) -> bool,
    dormant: impl IntoIterator<Item = &'a ScalarSourceDeclarations>,
) -> Result<(), ProjectCheck> {
    fn logical_source(source: &SourceIdentity) -> (&str, &str, &str) {
        (&source.project, &source.package, &source.path)
    }
    let mut startup_keys = HashSet::new();
    if !declarations.iter().all(|(source, _)| {
        in_startup(source.source()) && startup_keys.insert(logical_source(source.source()))
    }) {
        return Err(ProjectCheck::Unsupported);
    }
    let mut dormant_keys = HashSet::new();
    for source in dormant {
        let identity = source.source();
        let key = logical_source(identity);
        if identity.project != root.project || identity.package != root.package
            || startup_keys.contains(&key) || !dormant_keys.insert(key)
            || source.namespaces().next().is_some()
        {
            return Err(ProjectCheck::Unsupported);
        }
    }
    if declarations.iter().flat_map(|(_, edges)| edges).any(|edge| {
        dormant_keys.contains(&logical_source(&edge.target))
    }) {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(())
}

#[cfg(feature = "internal-checker")]
fn materialize_checker_failure(error: ProjectCheck) -> Option<ScalarCheckerFailure> {
    let ProjectCheck::Failed { source, operation, directive, condition, output, axes, state } = error else {
        return None;
    };
    Some(ScalarCheckerFailure {
        source,
        operation: operation.into(),
        directive,
        condition,
        stdout_span: output.stdout,
        stderr_span: output.stderr,
        code_span: output.code,
        files_span: output.files,
        files: output.text_files,
        stdout: output.stdout_bytes,
        stderr: output.stderr_bytes,
        code: output.code_value,
        witness: ScalarCheckerIntegerWitness { axes, state, equal_axes: Vec::new() },
    })
}

#[derive(Debug, PartialEq)]
enum ProjectCheck {
    Invalid(Vec<Diagnostic>),
    InvalidProof,
    MissingRoot,
    AmbiguousRoot,
    RootSourceOutsideInitOrder,
    MissingRootOccurrences,
    Unsupported,
    #[cfg(test)]
    NoChecks,
    NoApplicableInputs,
    Proved,
    Refuted,
    Failed {
        source: SourceIdentity,
        operation: CheckerOperation,
        directive: ByteSpan,
        condition: ByteSpan,
        state: IntegerInputs,
        axes: Vec<(ReferenceBindingId, ScalarType)>,
        output: CheckedLiteralOutput,
    },
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
struct VerifiedReturnBound {
    proof_venue: ProofVenue,
    proof_source: SourceIdentity,
    proof_span: ByteSpan,
    proof_name: String,
    target: ResolvedCallTarget,
    target_layout: ScalarTargetLayout,
    parameters: Vec<ScalarType>,
    output_slot: usize,
    output_type: ScalarType,
    operator: BinaryOperator,
    bound: BigInt,
    guards: Vec<VerifiedEntryGuard>,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
enum ProofVenue {
    DetachedFragment,
    ModuleDeclaration,
}

#[cfg(test)]
enum ProofSummaryOrigin<'a> {
    DetachedFragment(&'a SourceIdentity),
    ModuleDeclaration {
        source: &'a SourceIdentity,
        target: &'a ResolvedCallTarget,
    },
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
enum VerifiedEntryGuard {
    Boolean(bool),
    Integer {
        slot: usize,
        ty: ScalarType,
        operator: BinaryOperator,
        bound: BigInt,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitializationVisit {
    Unseen,
    Started,
    Completed,
}

#[derive(Debug, PartialEq, Eq)]
struct CheckedInitialization<'a> {
    visits: Vec<InitializationVisit>,
    reached: HashSet<(usize, usize)>,
    ordinals: HashMap<&'a SourceIdentity, usize>,
    had_obligations: bool,
}

fn has_callable_obligations(body: &FunctionCallSites) -> bool {
    !body.structure.checker_directives.is_empty() || body.arms.iter().any(has_callable_obligations)
}

#[cfg(test)]
fn check_package_roots(prepared: &PreparedCheckerProject) -> Result<(), ProjectCheck> {
    check_package_roots_with_activity(
        prepared, ScalarCheckerScope::CurrentPackage, prepared.parts().0.modules.len(),
    ).map(drop)
}

#[cfg(feature = "internal-checker")]
fn collect_ordered_checker_records(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<(bool, Vec<ScalarOrderedCheckerFailure>), ProjectCheck> {
    let run = run_prepared_ordered_check(prepared, scope, startup_count, OrderedReachPolicy::Legacy, ())?;
    Ok((run.active, run.directives))
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum OrderedReachPolicy {
    Legacy,
    Closed,
}

#[cfg(feature = "internal-checker")]
trait OrderedRunEvidence: ActualIntegerObserver {
    type Symbolic<'a>: SymbolicRootProof where Self: 'a;
    type SymbolicReport: Default;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck>;
    fn symbolic(&self) -> Self::Symbolic<'_>;
    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run;
}

#[cfg(feature = "internal-checker")]
impl OrderedRunEvidence for () {
    type Symbolic<'a> = ();
    type SymbolicReport = ();

    fn complete_startup(&self, _: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> { Ok(false) }
    fn symbolic(&self) -> Self::Symbolic<'_> {}
    fn finish_symbolic<'run>(_: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run {}
}

#[cfg(feature = "internal-checker")]
struct ProofFreeOrderedEvidence;

#[cfg(feature = "internal-checker")]
impl ActualIntegerObserver for ProofFreeOrderedEvidence {
    const ENABLED: bool = false;
    const PROOF_FREE_CONTEXTUAL_LOCAL_TRANSFERS: bool = true;
    const EXACT_SELF_REENTRY: bool = true;
    const EXACT_INTEGER_CALL_CONTINUATIONS: bool = true;
    const EXACT_INTEGER_NARROW_CONTINUATIONS: bool = true;
    const EXACT_INTEGER_LOCAL_WRITES: bool = true;
    const ORDINARY_INTEGER_LOCALS: bool = true;

    fn observe(&mut self, _: (usize, usize), _: &[&BigInt], _: &BigInt) {}
}

#[cfg(feature = "internal-checker")]
impl OrderedRunEvidence for ProofFreeOrderedEvidence {
    type Symbolic<'a> = ProofFreeSymbolic;
    type SymbolicReport = ();

    fn complete_startup(&self, _: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> { Ok(false) }
    fn symbolic(&self) -> Self::Symbolic<'_> { ProofFreeSymbolic { ordinary_projected_effects: false } }
    fn finish_symbolic<'run>(_: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run {}
}

#[cfg(feature = "internal-checker")]
struct PreparedOrderedRun<E: OrderedRunEvidence> {
    active: bool,
    directives: Vec<ScalarOrderedCheckerFailure>,
    actual: E,
    symbolic: E::SymbolicReport,
    entry_evidence: Option<Box<ScalarOrderedEntryEvidence>>,
}

#[cfg(feature = "internal-checker")]
fn run_prepared_ordered_check<E: OrderedRunEvidence>(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
    policy: OrderedReachPolicy, evidence: E,
) -> Result<PreparedOrderedRun<E>, ProjectCheck> {
    let (project, _) = prepared.parts();
    let (root, ordinals) = package_startup_order(project, &project.modules[..startup_count])?;
    let mut effects = OrderedFailures::default();
    let authored_roots = if E::CONTEXTUAL && scope == ScalarCheckerScope::MainFlowOnly {
        HashMap::new()
    } else {
        let (potential_obligations, authored_roots) = package_obligation_roots(prepared, scope, startup_count, root)?;
        if !potential_obligations && !E::CONTEXTUAL {
            return Ok(PreparedOrderedRun { active: false, directives: effects.records, actual: evidence, symbolic: E::SymbolicReport::default(), entry_evidence: None });
        }
        authored_roots
    };
    let (initialization, evidence) = match policy {
        OrderedReachPolicy::Legacy => check_package_initialization_observing_with_policy::<_, true, false>(
            prepared, root, ordinals, evidence, Some(&mut effects),
        ),
        OrderedReachPolicy::Closed => check_package_initialization_observing_with_policy::<_, true, true>(
            prepared, root, ordinals, evidence, Some(&mut effects),
        ),
    }?;
    let mut active = evidence.complete_startup(&initialization.reached)? || initialization.had_obligations;
    let symbolic = if scope == ScalarCheckerScope::CurrentPackage {
        let mut proof = evidence.symbolic();
        active |= match policy {
            OrderedReachPolicy::Legacy => check_package_symbolic_roots::<true, false, _>(
                prepared, startup_count, root, initialization, authored_roots, Some(&mut effects), &mut proof,
            ),
            OrderedReachPolicy::Closed => check_package_symbolic_roots::<true, true, _>(
                prepared, startup_count, root, initialization, authored_roots, Some(&mut effects), &mut proof,
            ),
        }?;
        E::finish_symbolic(proof)
    } else {
        E::SymbolicReport::default()
    };
    // Materialize authored state only for a completed check, before any
    // client-visible stream/file effect can be planned.
    if effects.has_authored_state || !effects.snapshot_records.is_empty() {
        let mut work = 0;
        let mut hidden = effects.snapshot_records.into_iter().peekable();
        for (index, record) in effects.records.iter_mut().enumerate() {
            if effects.has_authored_state && !record.output.state_files.is_empty() {
                record.output = Rc::new(render_authored_state(record, &mut work).ok_or(ProjectCheck::Unsupported)?);
            }
            if hidden.peek() == Some(&index) {
                hidden.next();
                record.witness.remove_snapshot_axes(&mut work).ok_or(ProjectCheck::Unsupported)?;
            }
        }
    }
    Ok(PreparedOrderedRun {
        active, directives: effects.records, actual: evidence, symbolic, entry_evidence: effects.entry_evidence,
    })
}

#[cfg(all(test, feature = "internal-checker"))]
fn collect_ordered_package_failures(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<(bool, Vec<ProjectCheck>), ProjectCheck> {
    let (active, failures) = collect_ordered_checker_records(prepared, scope, startup_count)?;
    Ok((active, failures.into_iter().map(ordered_failure_as_legacy).collect()))
}

fn check_package_roots_with_activity(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<bool, ProjectCheck> {
    let (project, _) = prepared.parts();
    let (root, ordinals) = package_startup_order(project, &project.modules[..startup_count])?;
    let (potential_obligations, authored_roots) = package_obligation_roots(prepared, scope, startup_count, root)?;
    if !potential_obligations {
        return Ok(false);
    }
    let initialization = check_package_initialization_with_order(prepared, root, ordinals)?;
    if scope == ScalarCheckerScope::MainFlowOnly {
        return Ok(initialization.had_obligations);
    }
    check_package_symbolic_roots::<false, false, _>(
        prepared, startup_count, root, initialization, authored_roots, None, &mut (),
    )
}

fn package_obligation_roots(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize, root: &SourceIdentity,
) -> Result<(bool, HashMap<usize, Vec<usize>>), ProjectCheck> {
    let (project, calls) = prepared.parts();
    let mut potential_obligations = false;
    let mut authored_roots = HashMap::<usize, Vec<usize>>::new();
    for (module, source_module) in project.modules.iter().enumerate() {
        if module < startup_count {
            let region = calls.initialization_region(module).ok_or(ProjectCheck::MissingRootOccurrences)?;
            let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
                return Err(ProjectCheck::MissingRootOccurrences);
            };
            potential_obligations |= !topology.structure.checker_directives.is_empty();
        }
        let is_current_symbolic_package = scope == ScalarCheckerScope::CurrentPackage
            && source_module.source.project == root.project && source_module.source.package == root.package;
        for (item, declaration) in source_module.items.iter().enumerate() {
            if matches!(declaration, ScalarItem::Function(_)) {
                let body = calls.function_body(module, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
                if has_callable_obligations(body) {
                    potential_obligations = true;
                    if is_current_symbolic_package {
                        authored_roots.entry(module).or_default().push(item);
                    }
                }
            }
        }
    }
    Ok((potential_obligations, authored_roots))
}

fn check_package_symbolic_roots<const COLLECT: bool, const ENTRY_EVIDENCE: bool, Proof: SymbolicRootProof>(
    prepared: &PreparedCheckerProject, startup_count: usize, root: &SourceIdentity,
    initialization: CheckedInitialization<'_>, mut authored_roots: HashMap<usize, Vec<usize>>,
    mut failures: Option<&mut OrderedFailures>, proof: &mut Proof,
) -> Result<bool, ProjectCheck> {
    let (project, calls) = prepared.parts();
    let dormant = &project.modules[startup_count..];
    let CheckedInitialization { visits, reached, ordinals, mut had_obligations } = initialization;
    let mut ordinary_local_callers = HashSet::new();
    let mut ordered_roots = if COLLECT {
        close_ordered_symbolic_roots(std::mem::take(&mut authored_roots), || {
            let mut ordinary_local_seeds = Vec::new();
            let callers = ordered_symbolic_callers(project, calls, root,
                Proof::INTEGER_RETURN_PARTITIONS.then_some(&mut ordinary_local_seeds))?;
            // The same prepared reverse call graph supplies both root selection
            // and ordinary-local cursor eligibility, including transitive callers.
            ordinary_local_callers.extend(ordinary_local_seeds.iter().copied());
            while let Some(callee) = ordinary_local_seeds.pop() {
                if let Some(edges) = callers.get(&callee) {
                    ordinary_local_seeds.extend(edges.iter().copied().filter(|caller| ordinary_local_callers.insert(*caller)));
                }
            }
            Ok(callers)
        })?
    } else {
        HashMap::new()
    };
    if COLLECT && Proof::ENABLED {
        proof.seed(&mut ordered_roots, &reached);
    }
    let mut module_roots = |ordinal: usize, module: &ScalarModule| {
        if COLLECT {
            Ok(ordered_roots.remove(&ordinal))
        } else {
            authored_roots.remove(&ordinal).map(|seeds| symbolic_obligation_roots(calls, module, ordinal, seeds)).transpose()
        }
    };
    for source in &project.initialization_order {
        if source.project != root.project || source.package != root.package {
            continue;
        }
        let ordinal = *ordinals.get(source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
        let completed = visits[ordinal] == InitializationVisit::Completed;
        if !COLLECT && !completed {
            continue;
        }
        let module = &project.modules[ordinal];
        if let Some(selected) = module_roots(ordinal, module)? {
            if !completed {
                if selected.into_iter().any(|item| !reached.contains(&(ordinal, item))) {
                    return Err(ProjectCheck::Unsupported);
                }
                continue;
            }
            had_obligations |= check_unused_module::<COLLECT, ENTRY_EVIDENCE, _>(
                project, calls, module, ordinal, &reached, selected, &ordinary_local_callers, failures.as_deref_mut(), Some(&visits), proof,
            )?;
        }
    }
    for (index, module) in dormant.iter().enumerate() {
        let ordinal = startup_count + index;
        if let Some(selected) = module_roots(ordinal, module)? {
            had_obligations |= check_unused_module::<COLLECT, ENTRY_EVIDENCE, _>(
                project, calls, module, ordinal, &reached, selected, &ordinary_local_callers, failures.as_deref_mut(), None, proof,
            )?;
        }
    }
    Ok(had_obligations)
}

type PreparedCallable = (usize, usize);

fn ordered_symbolic_callers(
    project: &ScalarProject,
    calls: &ProjectCallSiteResults,
    root: &SourceIdentity,
    mut ordinary_local_seeds: Option<&mut Vec<PreparedCallable>>,
) -> Result<HashMap<PreparedCallable, Vec<PreparedCallable>>, ProjectCheck> {
    #[cfg(not(feature = "internal-checker"))]
    let _ = &mut ordinary_local_seeds;
    let mut callers = HashMap::new();
    for (module, source_module) in project.modules.iter().enumerate() {
        if source_module.source.project != root.project || source_module.source.package != root.package {
            continue;
        }
        for (item, declaration) in source_module.items.iter().enumerate() {
            if !matches!(declaration, ScalarItem::Function(_)) {
                continue;
            }
            let body = calls.function_body(module, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
            #[cfg(feature = "internal-checker")]
            if let Some(seeds) = ordinary_local_seeds.as_deref_mut() {
                if body.structure.has_ordinary_integer_operations() {
                    seeds.push((module, item));
                }
            }
            record_ordered_symbolic_callable_edges(body, (module, item), project, root, &mut callers);
        }
    }
    Ok(callers)
}

fn record_ordered_symbolic_callable_edges(
    body: &FunctionCallSites,
    caller: PreparedCallable,
    project: &ScalarProject,
    root: &SourceIdentity,
    callers: &mut HashMap<PreparedCallable, Vec<PreparedCallable>>,
) {
    for (occurrence, &target) in &body.structure.checker_direct_targets {
        if target.0 != caller.0 {
            let Some(signature) = body.checked_call_signatures.get(occurrence) else { continue; };
            if !matches!(signature.outputs.outputs.as_slice(), [output] if output.ty == ScalarType::Unit) {
                continue;
            }
            let Some(owner) = project.modules.get(target.0) else { continue; };
            if root.project != owner.source.project || root.package != owner.source.package {
                continue;
            }
        }
        let edges = callers.entry(target).or_default();
        if edges.last() != Some(&caller) {
            edges.push(caller);
        }
    }
    for arm in &body.arms {
        record_ordered_symbolic_callable_edges(arm, caller, project, root, callers);
    }
}

fn close_ordered_symbolic_roots(
    authored: HashMap<usize, Vec<usize>>,
    build_callers: impl FnOnce() -> Result<HashMap<PreparedCallable, Vec<PreparedCallable>>, ProjectCheck>,
) -> Result<HashMap<usize, HashSet<usize>>, ProjectCheck> {
    let mut selected = HashMap::new();
    if authored.is_empty() {
        return Ok(selected);
    }
    let mut callers = build_callers()?;
    let mut pending = Vec::new();
    for (module, items) in authored {
        selected.insert(module, items.iter().copied().collect::<HashSet<_>>());
        pending.extend(items.into_iter().map(|item| (module, item)));
    }
    while let Some(target) = pending.pop() {
        if let Some(edges) = callers.remove(&target) {
            for (module, item) in edges {
                if selected.entry(module).or_default().insert(item) {
                    pending.push((module, item));
                }
            }
        }
    }
    Ok(selected)
}

fn record_symbolic_callable_edges(
    body: &FunctionCallSites,
    ordinal: usize,
    caller: usize,
    callers: &mut HashMap<usize, Vec<usize>>,
) {
    for &(target_module, target_item) in body.structure.checker_direct_targets.values() {
        if target_module == ordinal {
            let edges = callers.entry(target_item).or_default();
            if edges.last() != Some(&caller) {
                edges.push(caller);
            }
        }
    }
    for arm in &body.arms {
        record_symbolic_callable_edges(arm, ordinal, caller, callers);
    }
}

fn symbolic_obligation_roots(
    calls: &ProjectCallSiteResults,
    module: &ScalarModule,
    ordinal: usize,
    mut pending: Vec<usize>,
) -> Result<HashSet<usize>, ProjectCheck> {
    let mut callers = HashMap::new();
    for (item, declaration) in module.items.iter().enumerate() {
        if !matches!(declaration, ScalarItem::Function(_)) {
            continue;
        }
        let body = calls.function_body(ordinal, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
        record_symbolic_callable_edges(body, ordinal, item, &mut callers);
    }
    let mut roots: HashSet<_> = pending.iter().copied().collect();
    if !callers.is_empty() {
        while let Some(target) = pending.pop() {
            if let Some(edges) = callers.remove(&target) {
                pending.extend(edges.into_iter().filter(|caller| roots.insert(*caller)));
            }
        }
    }
    Ok(roots)
}

#[cfg(feature = "internal-checker")]
struct ProofFreeSymbolic<const ORDINARY_PROJECTIONS: bool = false> {
    ordinary_projected_effects: bool,
}

#[cfg(feature = "internal-checker")]
impl SymbolicRootProof for ProofFreeSymbolic {
    const ENABLED: bool = false;
    const INTEGER_RETURN_PARTITIONS: bool = true;
}

#[cfg(feature = "internal-checker")]
impl<const ORDINARY_PROJECTIONS: bool> ActualIntegerObserver for ProofFreeSymbolic<ORDINARY_PROJECTIONS> {
    const ENABLED: bool = false;
    const EXACT_INTEGER_CALL_CONTINUATIONS: bool = true;
    const INTEGER_RETURN_PARTITIONS: bool = true;

    const PROJECTED_FORMAL_WRITES: bool = ORDINARY_PROJECTIONS;
    const ORDINARY_PROJECTED_PAIR: bool = ORDINARY_PROJECTIONS;
    const ORDINARY_INTEGER_LOCALS: bool = ORDINARY_PROJECTIONS;

    fn permits_ordinary_projected_effects(&self) -> bool {
        ORDINARY_PROJECTIONS && self.ordinary_projected_effects
    }

    fn observe(&mut self, _: (usize, usize), _: &[&BigInt], _: &BigInt) {}
}

#[derive(Clone, Copy)]
struct SymbolicProofRuntime<'a> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    visits: Option<&'a [InitializationVisit]>,
}

trait SymbolicRootProof {
    const ENABLED: bool;
    const INTEGER_RETURN_PARTITIONS: bool = false;

    fn seed(&self, _: &mut HashMap<usize, HashSet<usize>>, _: &HashSet<PreparedCallable>) {}

    fn try_collect(
        &mut self, _: PreparedCallable, _: &FunctionCallSites, _: &ScalarFunction, _: &SourceIdentity,
        _: &mut OrderedFailures, _: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        Ok(None)
    }
}

impl SymbolicRootProof for () {
    const ENABLED: bool = false;
}

fn check_unused_module<const COLLECT: bool, const ENTRY_EVIDENCE: bool, Proof: SymbolicRootProof>(
    project: &ScalarProject,
    calls: &ProjectCallSiteResults,
    module: &ScalarModule,
    ordinal: usize,
    reached: &HashSet<(usize, usize)>,
    symbolic_roots: HashSet<usize>,
    ordinary_local_callers: &HashSet<PreparedCallable>,
    mut failures: Option<&mut OrderedFailures>,
    completed_visits: Option<&[InitializationVisit]>,
    proof: &mut Proof,
) -> Result<bool, ProjectCheck> {
    #[cfg(not(feature = "internal-checker"))]
    let _ = ordinary_local_callers;
    let mut active = false;
    for (item, declaration) in module.items.iter().enumerate() {
        let ScalarItem::Function(function) = declaration else {
            continue;
        };
        if reached.contains(&(ordinal, item)) || !symbolic_roots.contains(&item) {
            continue;
        }
        let body = calls.function_body(ordinal, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        if !matches!(body.boundary.results.as_slice(), [result]
            if result.ty == ScalarType::Unit || is_integer_type(&result.ty)
                || (COLLECT && Proof::INTEGER_RETURN_PARTITIONS && result.ty == ScalarType::Bool))
        {
            return Err(ProjectCheck::Unsupported);
        }
        if COLLECT && Proof::ENABLED {
            if let Some(observed) = proof.try_collect(
                (ordinal, item), body, function, &module.source, failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?,
                SymbolicProofRuntime { project, calls, visits: completed_visits },
            )? {
                active |= observed;
                continue;
            }
        }
        #[cfg(feature = "internal-checker")]
        if COLLECT && Proof::INTEGER_RETURN_PARTITIONS
            && matches!(body.boundary.results.as_slice(), [result] if matches!(result.ty, ScalarType::Unit | ScalarType::Bool))
            && (!body.structure.checker_direct_targets.is_empty() || !body.structure.checker_ordinary_directive_calls.is_empty())
            && (ordinary_local_callers.contains(&(ordinal, item))
                || !body.structure.checker_integer_call_locals.is_empty()
                || body.structure.checker_direct_targets.keys().any(|occurrence| {
                    body.checked_call_signatures.get(occurrence).is_some_and(|signature|
                        matches!(signature.parameters.as_slice(), [left, right] if is_integer_type(left) && is_integer_type(right))
                        && matches!(signature.outputs.outputs.as_slice(), [output] if output.ty == ScalarType::Unit))
                }))
        {
            let sink = RefCell::new(failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?);
            let collection = OrderedCallCollection { failures: &sink, visits: completed_visits, outer: None };
            let mut symbolic = CheckedCalls::<ProofFreeSymbolic<true>, false> {
                project, calls, active: HashSet::from([(ordinal, item)]), active_depth: 1, reached: HashSet::new(),
                had_obligations: false, observer: ProofFreeSymbolic { ordinary_projected_effects: !Proof::ENABLED }, symbolic_module: ordinal,
            };
            let cursor = ExactCallCursor::arbitrary_root(body, &body.boundary.results[0].ty)
                .ok_or(ProjectCheck::Unsupported)?;
            symbolic.run_call_cursor(body, &module.source, cursor, None, Some(collection))?;
            active |= symbolic.had_obligations;
            continue;
        }
        if body.structure.checker_direct_targets.is_empty() {
            if COLLECT {
                #[cfg(feature = "internal-checker")]
                let collected = super::checker_flow::collect_ordered_root_requirement_failures(
                    body, &function.body, Proof::INTEGER_RETURN_PARTITIONS,
                );
                #[cfg(not(feature = "internal-checker"))]
                let collected = collect_integer_requirement_failures(body);
                let (observed, selected) = collected.ok_or(ProjectCheck::Unsupported)?;
                active |= observed;
                let failures = failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?;
                for failure in selected {
                    #[cfg(feature = "internal-checker")]
                    if ENTRY_EVIDENCE {
                        failures.capture_entry_evidence(body, function, &module.source, &failure);
                    }
                    failures.push_callable(body, &module.source, failure);
                }
            } else {
                match check_integer_requirement_activity(body) {
                    Some(Ok(observed)) => active |= observed,
                    Some(Err(failure)) => return Err(project_failure(body, &module.source, failure)),
                    None => return Err(ProjectCheck::Unsupported),
                }
            }
        } else {
            let mut symbolic = CheckedCalls::<(), false> {
                project, calls, active: HashSet::from([(ordinal, item)]), active_depth: 1, reached: HashSet::new(),
                had_obligations: false, observer: (), symbolic_module: ordinal,
            };
            if COLLECT {
                let sink = RefCell::new(failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?);
                let collection = OrderedCallCollection { failures: &sink, visits: completed_visits, #[cfg(feature = "internal-checker")] outer: None };
                symbolic.collect_requirements(body, &module.source, IntegerRequirementEntry::Arbitrary, collection)?;
                active |= symbolic.had_obligations;
            } else {
                let observed = symbolic.requirements(body, &module.source, None)?;
                active |= observed || symbolic.had_obligations;
            }
        }
    }
    Ok(active)
}

fn package_initialization_order(
    project: &ScalarProject,
) -> Result<(&SourceIdentity, HashMap<&SourceIdentity, usize>), ProjectCheck> {
    package_startup_order(project, &project.modules)
}

fn package_startup_order<'a>(
    project: &'a ScalarProject,
    startup: &'a [ScalarModule],
) -> Result<(&'a SourceIdentity, HashMap<&'a SourceIdentity, usize>), ProjectCheck> {
    let ordinals: HashMap<_, _> = startup.iter().enumerate()
        .map(|(ordinal, module)| (&module.source, ordinal)).collect();
    if ordinals.len() != startup.len() {
        return Err(ProjectCheck::AmbiguousRoot);
    }
    let root = project.initialization_order.first().ok_or(ProjectCheck::MissingRoot)?;
    if project.initialization_order.len() != ordinals.len() {
        return Err(ProjectCheck::RootSourceOutsideInitOrder);
    }
    let mut seen = vec![false; ordinals.len()];
    for source in &project.initialization_order {
        let ordinal = *ordinals.get(source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
        if std::mem::replace(&mut seen[ordinal], true) {
            return Err(ProjectCheck::AmbiguousRoot);
        }
    }
    Ok((root, ordinals))
}

#[cfg(test)]
fn check_package_initialization(prepared: &PreparedCheckerProject) -> Result<CheckedInitialization<'_>, ProjectCheck> {
    let (root, ordinals) = package_initialization_order(prepared.parts().0)?;
    check_package_initialization_with_order(prepared, root, ordinals)
}

fn check_package_initialization_with_order<'a>(
    prepared: &'a PreparedCheckerProject,
    root: &'a SourceIdentity,
    ordinals: HashMap<&'a SourceIdentity, usize>,
) -> Result<CheckedInitialization<'a>, ProjectCheck> {
    check_package_initialization_observing(prepared, root, ordinals, ()).map(|(initialization, ())| initialization)
}

#[cfg(all(test, feature = "internal-checker"))]
fn collect_package_startup_failures(
    prepared: &PreparedCheckerProject,
) -> Result<Vec<ProjectCheck>, ProjectCheck> {
    let (root, ordinals) = package_initialization_order(prepared.parts().0)?;
    let mut failures = OrderedFailures::default();
    check_package_initialization_observing_with_policy::<(), true, false>(
        prepared, root, ordinals, (), Some(&mut failures),
    )?;
    Ok(failures.records.into_iter().map(ordered_failure_as_legacy).collect())
}

fn check_package_initialization_observing<'a, Observer: ActualIntegerObserver>(
    prepared: &'a PreparedCheckerProject,
    root: &'a SourceIdentity,
    ordinals: HashMap<&'a SourceIdentity, usize>,
    observer: Observer,
) -> Result<(CheckedInitialization<'a>, Observer), ProjectCheck> {
    check_package_initialization_observing_with_policy::<Observer, false, false>(prepared, root, ordinals, observer, None)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupLane {
    Feasible,
    Empty,
}

fn check_package_initialization_observing_with_policy<'a, Observer: ActualIntegerObserver, const COLLECT: bool, const PRIVATE_NESTED_SCALAR: bool>(
    prepared: &'a PreparedCheckerProject,
    root: &'a SourceIdentity,
    ordinals: HashMap<&'a SourceIdentity, usize>,
    observer: Observer,
    mut failures: Option<&mut OrderedFailures>,
) -> Result<(CheckedInitialization<'a>, Observer), ProjectCheck> {
    let (project, calls) = prepared.parts();
    let mut visits = vec![InitializationVisit::Unseen; project.modules.len()];
    let mut actual = CheckedCalls {
        project, calls, active: HashSet::new(), active_depth: 0, reached: HashSet::new(),
        had_obligations: false, observer, symbolic_module: 0,
    };
    walk_package_initialization::<COLLECT, PRIVATE_NESTED_SCALAR>(project, calls, &ordinals, &mut visits, &mut actual, root, &mut failures)?;
    Ok((CheckedInitialization { visits, reached: actual.reached, ordinals, had_obligations: actual.had_obligations }, actual.observer))
}

fn walk_package_initialization<const COLLECT: bool, const PRIVATE_NESTED_SCALAR: bool>(
    project: &ScalarProject,
    calls: &ProjectCallSiteResults,
    ordinals: &HashMap<&SourceIdentity, usize>,
    visits: &mut [InitializationVisit],
    actual: &mut CheckedCalls<'_, impl ActualIntegerObserver>,
    source: &SourceIdentity,
    failures: &mut Option<&mut OrderedFailures>,
) -> Result<StartupLane, ProjectCheck> {
    let ordinal = *ordinals.get(source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
    match visits[ordinal] {
        InitializationVisit::Completed => return Ok(StartupLane::Feasible),
        InitializationVisit::Started => return Err(ProjectCheck::Unsupported),
        InitializationVisit::Unseen => visits[ordinal] = InitializationVisit::Started,
    }
    let module = &project.modules[ordinal];
    let region = calls.initialization_region(ordinal).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
        return Err(ProjectCheck::MissingRootOccurrences);
    };
    let structure = &topology.structure;
    let mut checked_items = structure.blocks[structure.root.0].items.iter();
    let mut storage = HashMap::new();
    for step in &region.steps {
        if let ModuleInitializationStepKind::NamespaceDispatch { target, .. } = &step.kind {
            let NamespaceTarget::Bound(target) = target else {
                return Err(ProjectCheck::Unsupported);
            };
            if walk_package_initialization::<COLLECT, PRIVATE_NESTED_SCALAR>(project, calls, ordinals, visits, actual, target, failures)? == StartupLane::Empty {
                return Ok(StartupLane::Empty);
            }
            continue;
        }
        let checked_item = checked_items.next().filter(|item| item.position == step.item)
            .ok_or(ProjectCheck::MissingRootOccurrences)?;
        if let (ModuleInitializationStepKind::BindingInitialization,
            Some(ScalarItem::Binding(binding)), BlockItemKind::Binding(checked)) =
            (&step.kind, module.items.get(step.item), &checked_item.kind)
        {
            let ([receiver], BindingInitialization::Outputs(outputs)) =
                (checked.receivers(), &checked.initialization) else {
                    return Err(ProjectCheck::Unsupported);
                };
            if !outputs.secondary.is_empty()
                || structure.checked_types.expressions.get(&outputs.primary) != Some(&receiver.ty)
            {
                return Err(ProjectCheck::Unsupported);
            }
            let value = match (&receiver.ty, &binding.value) {
                (ScalarType::Bool, ScalarExpression::Boolean { value, .. }) => InitializerValue::Boolean(*value),
                (ty, expression) if is_integer_type(ty) => {
                    let value = if let Some(value) = concrete_integer_literal(expression) {
                        value
                    } else if matches!(expression, ScalarExpression::Call { .. }) {
                        #[cfg(feature = "internal-checker")]
                        let proof_site = actual.initializer_proof_site(topology, source, outputs.primary, failures.as_deref());
                        #[cfg(feature = "internal-checker")]
                        let sink = failures.as_deref_mut().map(RefCell::new);
                        #[cfg(feature = "internal-checker")]
                        let collection = sink.as_ref().map(|failures| (source, OrderedCallCollection {
                            failures, visits: None, outer: None,
                        }));
                        #[cfg(not(feature = "internal-checker"))]
                        let collection = None;
                        match run_initializer_call(
                            topology, actual, outputs.primary, ty,
                            |argument, argument_type| initialized_integer_argument(topology, &storage, argument, argument_type),
                            collection,
                            #[cfg(feature = "internal-checker")]
                            proof_site,
                        )? {
                            ActualCallResult::Integer(value) => Cow::Owned(value),
                            #[cfg(feature = "internal-checker")]
                            ActualCallResult::EmptyInteger if COLLECT => return Ok(StartupLane::Empty),
                            _ => return Err(ProjectCheck::Unsupported),
                        }
                    } else {
                        return Err(ProjectCheck::Unsupported);
                    };
                    InitializerValue::Integer { ty, value }
                }
                _ => return Err(ProjectCheck::Unsupported),
            };
            storage.insert(receiver.span, value);
            continue;
        }
        let (ModuleInitializationStepKind::ExecutableItem, BlockItemKind::Evaluation { expression }) =
            (&step.kind, &checked_item.kind) else {
                return Err(ProjectCheck::Unsupported);
            };
        if check_initializer_expression::<COLLECT, PRIVATE_NESTED_SCALAR>(topology, actual, source, &storage, *expression, 0, failures, visits)? == StartupLane::Empty {
            return Ok(StartupLane::Empty);
        }
    }
    visits[ordinal] = InitializationVisit::Completed;
    Ok(StartupLane::Feasible)
}

fn check_initializer_expression<const COLLECT: bool, const PRIVATE_NESTED_SCALAR: bool>(
    topology: &ModuleInitializationTopology,
    actual: &mut CheckedCalls<'_, impl ActualIntegerObserver>,
    source: &SourceIdentity,
    storage: &HashMap<ByteSpan, InitializerValue<'_>>,
    expression: OccurrenceOrdinal,
    depth: usize,
    failures: &mut Option<&mut OrderedFailures>,
    visits: &[InitializationVisit],
) -> Result<StartupLane, ProjectCheck> {
    let structure = &topology.structure;
    if let Some(directive) = structure.checker_directives.get(&expression) {
        let predicate = directive.initializer_predicate.as_ref().ok_or(ProjectCheck::Unsupported)?;
        let value = evaluate_initializer_predicate(predicate, storage).ok_or(ProjectCheck::Unsupported)?;
        actual.had_obligations = true;
        if directive.operation.rejects(value, !value) {
            let span = topology.occurrences.get(expression.0).ok_or(ProjectCheck::MissingRootOccurrences)?.span;
            if !COLLECT {
                return Err(selected_failure(source, directive, span, IntegerInputs::closed(), Vec::new()));
            }
            failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?
                .push(source, directive, span, IntegerInputs::closed(), Vec::new(), Vec::new());
            if directive.operation == CheckerOperation::Narrow {
                return Ok(StartupLane::Empty);
            }
        }
        return Ok(StartupLane::Feasible);
    }
    if let Some(region) = topology.conditionals.get(&expression) {
        let (condition, then_block, else_block) = match region {
            ConditionalRegion::Unit { condition, then_block, .. } => (condition, *then_block, None),
            ConditionalRegion::Value { condition, then_block, else_block, .. }
                if structure.checked_types.expressions.get(&expression) == Some(&ScalarType::Unit) =>
            {
                (condition, *then_block, Some(*else_block))
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        let value = topology.initializer_branches.get(condition)
            .and_then(|predicate| evaluate_initializer_predicate(predicate, storage))
            .ok_or(ProjectCheck::Unsupported)?;
        let selected = if value { Some(then_block) } else { else_block };
        if let Some(selected) = selected {
            if depth == MAX_INITIALIZER_BRANCH_DEPTH {
                return Err(ProjectCheck::Unsupported);
            }
            let block = structure.blocks.get(selected.0).ok_or(ProjectCheck::MissingRootOccurrences)?;
            for item in &block.items {
                let BlockItemKind::Evaluation { expression } = item.kind else {
                    return Err(ProjectCheck::Unsupported);
                };
                if check_initializer_expression::<COLLECT, PRIVATE_NESTED_SCALAR>(topology, actual, source, storage, expression, depth + 1, failures, visits)?
                    == StartupLane::Empty
                {
                    return Ok(StartupLane::Empty);
                }
            }
        }
        return Ok(StartupLane::Feasible);
    }
    let sink;
    let collection = if COLLECT {
        sink = RefCell::new(failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?);
        Some((source, OrderedCallCollection {
            failures: &sink, visits: Some(visits),
            #[cfg(feature = "internal-checker")]
            outer: (PRIVATE_NESTED_SCALAR && depth == 0).then_some(OrderedInitializerCall { topology, occurrence: expression }),
        }))
    } else { None };
    match run_initializer_call(topology, actual, expression, &ScalarType::Unit, |argument, ty| {
        if COLLECT { initialized_integer_argument(topology, storage, argument, ty) } else { None }
    }, collection, #[cfg(feature = "internal-checker")] None)? {
        ActualCallResult::Unit => Ok(StartupLane::Feasible),
        ActualCallResult::EmptyUnit if COLLECT => Ok(StartupLane::Empty),
        _ => Err(ProjectCheck::Unsupported),
    }
}

fn run_initializer_call<'a, Observer: ActualIntegerObserver>(
    topology: &'a ModuleInitializationTopology,
    actual: &mut CheckedCalls<'_, Observer>,
    occurrence: OccurrenceOrdinal,
    expected: &ScalarType,
    initialized_argument: impl Fn(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
    collection: Option<(&SourceIdentity, OrderedCallCollection<'_, '_>)>,
    #[cfg(feature = "internal-checker")] proof_site: Option<InitializerProofSite<'_>>,
) -> Result<ActualCallResult, ProjectCheck> {
    let Some(ModuleInitializationCallTarget::Direct {
        declaration,
        concrete: ConcreteCallSelection::Declaration,
    }) = topology.call_targets.get(&occurrence) else {
        return Err(ProjectCheck::Unsupported);
    };
    let signature = topology.checked_call_signatures.get(&occurrence).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let collection = collection.filter(|(source, _)| !is_integer_type(expected) || *source == &declaration.source);
    let allow_bool = *expected == ScalarType::Unit && collection.is_some() && Observer::ORDINARY_INTEGER_LOCALS;
    let arguments = if signature.parameters.len() == 2
        && (is_integer_type(expected) || (*expected == ScalarType::Unit && collection.is_some()))
    {
        let first = exact_typed_call_argument_at(&topology.structure, occurrence, 0, allow_bool, &initialized_argument)
            .ok_or(ProjectCheck::Unsupported)?;
        let second = exact_typed_call_argument_at(&topology.structure, occurrence, 1, allow_bool, &initialized_argument)
            .ok_or(ProjectCheck::Unsupported)?;
        ActualProjectedArguments::ExactPair(first, second)
    } else {
        let argument = exact_typed_call_argument_at(&topology.structure, occurrence, 0, allow_bool, initialized_argument);
        if *expected != ScalarType::Unit && argument.is_none() {
            return Err(ProjectCheck::Unsupported);
        }
        ActualProjectedArguments::Single(argument)
    };
    actual.run(
        &topology.structure, occurrence, signature, arguments, &declaration.source, declaration.range,
        expected, collection, #[cfg(feature = "internal-checker")] proof_site,
    )
}

enum InitializerValue<'a> {
    Boolean(bool),
    Integer { ty: &'a ScalarType, value: Cow<'a, BigInt> },
}

fn initialized_integer_argument<'a>(
    topology: &ModuleInitializationTopology,
    storage: &'a HashMap<ByteSpan, InitializerValue<'_>>,
    argument: OccurrenceOrdinal,
    argument_type: &ScalarType,
) -> Option<&'a BigInt> {
    let InitializationName::Storage(InitializationStorage::Module(span)) =
        topology.identities.names.get(&argument)? else {
            return None;
        };
    let InitializerValue::Integer { ty, value } = storage.get(span)? else {
        return None;
    };
    (*ty == argument_type).then_some(value.as_ref())
}

fn evaluate_initializer_predicate(
    predicate: &InitializerPredicate,
    storage: &HashMap<ByteSpan, InitializerValue<'_>>,
) -> Option<bool> {
    match predicate {
        InitializerPredicate::Boolean(value) => Some(*value),
        InitializerPredicate::BooleanStorage(span) => match storage.get(span)? {
            InitializerValue::Boolean(value) => Some(*value),
            InitializerValue::Integer { .. } => None,
        },
        InitializerPredicate::IntegerComparison { relation, operands } => {
            fn integer<'a>(
                operand: &'a InitializerIntegerOperand,
                storage: &'a HashMap<ByteSpan, InitializerValue<'_>>,
            ) -> Option<&'a BigInt> {
                match operand {
                    InitializerIntegerOperand::Literal(value) => Some(value),
                    InitializerIntegerOperand::Storage(span) => match storage.get(span)? {
                        InitializerValue::Integer { value, .. } => Some(value.as_ref()),
                        InitializerValue::Boolean(_) => None,
                    },
                }
            }
            let [left, right] = operands.as_ref();
            Some(relation.accepts(integer(left, storage)?.cmp(integer(right, storage)?)))
        }
    }
}

const MAX_INITIALIZER_BRANCH_DEPTH: usize = 64;

#[derive(Clone, Copy)]
struct OrderedCallCollection<'a, 'visits> {
    failures: &'a RefCell<&'a mut OrderedFailures>,
    visits: Option<&'visits [InitializationVisit]>,
    #[cfg(feature = "internal-checker")]
    outer: Option<OrderedInitializerCall<'a>>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
struct OrderedInitializerCall<'a> {
    topology: &'a ModuleInitializationTopology,
    occurrence: OccurrenceOrdinal,
}

enum ActualProjectedArguments<'a> {
    Single(Option<ExactIntegerArgument<'a>>),
    ExactPair(ExactIntegerArgument<'a>, ExactIntegerArgument<'a>),
    SymbolicAxis(ProjectedIntegerArgument<'a>),
}

struct ExactCallActivation<'a> {
    body: &'a FunctionCallSites,
    source: &'a SourceIdentity,
    cursor: ExactCallCursor<'a>,
    entry: Option<BigInt>,
    followed_self_edge: bool,
    target: Option<((usize, usize), bool, ExactCallEffect<'a>, Option<usize>)>,
}

struct CheckedCalls<'a, Observer, const ACTUAL_STARTUP: bool = true> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    active: HashSet<(usize, usize)>,
    active_depth: usize,
    reached: HashSet<(usize, usize)>,
    had_obligations: bool,
    observer: Observer,
    symbolic_module: usize,
}

#[cfg(feature = "internal-checker")]
struct InitializerProofSite<'a> {
    source: &'a SourceIdentity,
    call: InitializerProofOrigin<'a>,
}

#[cfg(feature = "internal-checker")]
enum InitializerProofOrigin<'a> {
    Direct { topology: &'a ModuleInitializationTopology, occurrence: OccurrenceOrdinal, directive_position: usize },
    Nested { outer: OrderedInitializerCall<'a>, caller: &'a FunctionCallSites, inner: OccurrenceOrdinal, directive_position: usize },
}

trait ActualIntegerObserver {
    const ENABLED: bool;
    const PROOF_FREE_CONTEXTUAL_LOCAL_TRANSFERS: bool = false;
    const EXACT_SELF_REENTRY: bool = false;
    const EXACT_INTEGER_CALL_CONTINUATIONS: bool = false;
    #[cfg(feature = "internal-checker")]
    const EXACT_INTEGER_NARROW_CONTINUATIONS: bool = false;
    #[cfg(feature = "internal-checker")]
    const EXACT_INTEGER_LOCAL_WRITES: bool = false;
    const INTEGER_RETURN_PARTITIONS: bool = false;
    const PROJECTED_FORMAL_WRITES: bool = false;
    const ORDINARY_PROJECTED_PAIR: bool = false;
    const ORDINARY_INTEGER_LOCALS: bool = false;

    fn observe(&mut self, target: (usize, usize), arguments: &[&BigInt], value: &BigInt);

    #[cfg(feature = "internal-checker")]
    fn permits_ordinary_projected_effects(&self) -> bool { false }

    const CONTEXTUAL: bool = false;

    #[cfg(feature = "internal-checker")]
    fn observe_initializer(
        &mut self, _target: (usize, usize), _site: Option<InitializerProofSite<'_>>,
        _parameters: &[ScalarType], _arguments: &[&BigInt], _value: &BigInt,
    ) {}

    #[cfg(feature = "internal-checker")]
    fn proved_recursive_return(
        &mut self, _target: (usize, usize), _arguments: &[&BigInt], _expected: &ScalarType,
    ) -> Option<BigInt> {
        None
    }
}

impl ActualIntegerObserver for () {
    const ENABLED: bool = false;

    #[inline(always)]
    fn observe(&mut self, _target: (usize, usize), _arguments: &[&BigInt], _value: &BigInt) {}
}

impl<'project, Observer: ActualIntegerObserver, const ACTUAL_STARTUP: bool> CheckedCalls<'project, Observer, ACTUAL_STARTUP> {
    #[cfg(feature = "internal-checker")]
    fn initializer_proof_site<'a>(
        &self, topology: &'a ModuleInitializationTopology, source: &'a SourceIdentity,
        occurrence: OccurrenceOrdinal, failures: Option<&OrderedFailures>,
    ) -> Option<InitializerProofSite<'a>> {
        if !ACTUAL_STARTUP || !Observer::CONTEXTUAL {
            return None;
        }
        Some(InitializerProofSite { source, call: InitializerProofOrigin::Direct { topology, occurrence, directive_position: failures?.records.len() } })
    }

    fn nested(
        &mut self,
        caller: &FunctionCallSites,
        call: OccurrenceOrdinal,
        signature: &CheckedCallSignature,
        argument: NestedIntegerArgument<'_>,
        expected: &ScalarType,
        collection: Option<(&SourceIdentity, OrderedCallCollection<'_, '_>)>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let site = caller.resolved_call_site(call).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let ResolvedCallTarget::ModuleCallable {
            source,
            declaration_span,
            concrete: ConcreteCallSelection::Declaration,
        } = &site.target else {
            return Err(ProjectCheck::Unsupported);
        };
        #[cfg(feature = "internal-checker")]
        let contextual_value = matches!(&argument, NestedIntegerArgument::ContextualValue(_));
        #[cfg(feature = "internal-checker")]
        if contextual_value && (!ACTUAL_STARTUP || Observer::ENABLED || Observer::CONTEXTUAL) {
            return Err(ProjectCheck::Unsupported);
        }
        #[cfg(feature = "internal-checker")]
        let mut proof_site = None;
        let collection = match collection {
            #[cfg(feature = "internal-checker")]
            Some((caller_source, ordered)) if contextual_value => {
                if caller_source != source || ordered.outer.is_none()
                    || caller.structure.checker_integer_call_locals.len() != 1 || !is_integer_type(expected)
                {
                    return Err(ProjectCheck::Unsupported);
                }
                Some((caller_source, OrderedCallCollection { outer: None, ..ordered }))
            }
            #[cfg(feature = "internal-checker")]
            None if contextual_value => return Err(ProjectCheck::Unsupported),
            Some((caller_source, ordered)) if *expected == ScalarType::Unit => {
                #[cfg(feature = "internal-checker")]
                let ordered = OrderedCallCollection { outer: None, ..ordered };
                Some((caller_source, ordered))
            }
            Some((_caller_source, _ordered)) => {
                #[cfg(feature = "internal-checker")]
                {
                    let outer = _ordered.outer.ok_or(ProjectCheck::Unsupported)?;
                    if !ACTUAL_STARTUP || !is_integer_type(expected) || _caller_source != source
                        || (caller.structure.checker_integer_call_locals.len() != 1
                            && !matches!(&argument, NestedIntegerArgument::RootLexical(_)))
                        || matches!(&argument, NestedIntegerArgument::Projected(_))
                    {
                        return Err(ProjectCheck::Unsupported);
                    }
                    if Observer::CONTEXTUAL {
                        let directive_position = _ordered.failures.borrow().records.len();
                        proof_site = Some(InitializerProofSite { source, call: InitializerProofOrigin::Nested {
                            outer, caller, inner: call, directive_position,
                        } });
                    }
                    None
                }
                #[cfg(not(feature = "internal-checker"))]
                return Err(ProjectCheck::Unsupported);
            }
            None => None,
        };
        let projected = match argument {
            NestedIntegerArgument::Single(exact) => ActualProjectedArguments::Single(exact),
            #[cfg(feature = "internal-checker")]
            NestedIntegerArgument::RootLexical(exact) | NestedIntegerArgument::ContextualValue(exact) =>
                ActualProjectedArguments::Single(Some(exact)),
            NestedIntegerArgument::Pair(first, second) => ActualProjectedArguments::ExactPair(first, second),
            NestedIntegerArgument::Projected(projection) => ActualProjectedArguments::SymbolicAxis(projection),
        };
        self.run(
            &caller.structure, call, signature, projected, source, *declaration_span, expected, collection,
            #[cfg(feature = "internal-checker")] proof_site,
        )
    }

    #[cfg(feature = "internal-checker")]
    fn computed_self_descent(
        &self,
        caller: &FunctionCallSites,
        call: OccurrenceOrdinal,
        parameter: &ScalarType,
        previous: &BigInt,
    ) -> Result<BigInt, ProjectCheck> {
        use super::reference::{OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};
        let structure = &caller.structure;
        if !caller.assignments.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        let Some(OccurrenceNode::Expression(expression)) = structure.edges.get(&(
            OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(0),
        )) else { return Err(ProjectCheck::Unsupported); };
        let (left, decrement) = structure.checker_integer_call_subtractions.get(expression).ok_or(ProjectCheck::Unsupported)?;
        let Some([formal]) = caller.boundary.checker_parameter_bindings.as_deref() else {
            return Err(ProjectCheck::Unsupported);
        };
        if !matches!(caller.prepared_plain_name(*left), Some(PlainNameIdentity::Lexical(binding)) if binding == formal)            || structure.checked_types.expressions.get(expression) != Some(parameter)
            || previous < decrement
        {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(previous - decrement)
    }

    fn requirements(
        &mut self,
        body: &FunctionCallSites,
        source: &SourceIdentity,
        exact: Option<&[&BigInt]>,
    ) -> Result<bool, ProjectCheck> {
        let mut nested_error = None;
        let checked = check_integer_requirements_with_calls(body, exact, &mut |caller, occurrence, signature, argument, expected| {
            match self.nested(caller, occurrence, signature, argument, expected, None) {
                Ok(result) => Some(result),
                Err(error) => {
                    nested_error = Some(error);
                    None
                }
            }
        });
        match checked {
            Some(Ok(active)) => Ok(active),
            Some(Err(failure)) => Err(project_failure(body, source, failure)),
            None => Err(nested_error.unwrap_or(ProjectCheck::Unsupported)),
        }
    }

    fn collect_requirements(
        &mut self,
        body: &FunctionCallSites,
        source: &SourceIdentity,
        entry: IntegerRequirementEntry<'_>,
        collection: OrderedCallCollection<'_, '_>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let terminal_projection = matches!(&entry, IntegerRequirementEntry::Projected { .. });
        let allow_contextual_local_transfers = ACTUAL_STARTUP && Observer::PROOF_FREE_CONTEXTUAL_LOCAL_TRANSFERS
            && matches!(&entry, IntegerRequirementEntry::Exact(_));
        let mut sink = |failure| collection.failures.borrow_mut().push_callable(body, source, failure);
        let mut nested_error = None;
        let mut nested = |caller: &FunctionCallSites, occurrence, signature: &CheckedCallSignature, argument: NestedIntegerArgument<'_>, expected: &ScalarType| {
            match self.nested(caller, occurrence, signature, argument, expected, Some((source, collection))) {
                Ok(result) => Some(result),
                Err(error) => {
                    nested_error = Some(error);
                    None
                }
            }
        };
        let handler: Option<&mut super::checker_flow::ActualCallHandler<'_, '_>> =
            if terminal_projection { None } else { Some(&mut nested) };
        let Some((active, continuation)) = collect_integer_requirement_failures_with_calls(
            body, entry, handler, !ACTUAL_STARTUP && !terminal_projection, allow_contextual_local_transfers, &mut sink,
        ) else {
            return Err(nested_error.unwrap_or(ProjectCheck::Unsupported));
        };
        self.had_obligations |= active;
        Ok(match continuation {
            IntegerRequirementContinuation::Feasible => ActualCallResult::Unit,
            IntegerRequirementContinuation::Empty => ActualCallResult::EmptyUnit,
            IntegerRequirementContinuation::Projected(values) => ActualCallResult::ProjectedUnit(values),
        })
    }

    fn exact_self_reentry_allowed(&self, expected: &ScalarType, collecting: bool) -> bool {
        ACTUAL_STARTUP && Observer::EXACT_SELF_REENTRY
            && (*expected == ScalarType::Unit
                || (collecting && Observer::EXACT_INTEGER_CALL_CONTINUATIONS && is_integer_type(expected)))
    }

    fn exact_integer_narrow_profile(&self, collecting: bool) -> bool {
        #[cfg(feature = "internal-checker")]
        { ACTUAL_STARTUP && collecting && Observer::EXACT_INTEGER_CALL_CONTINUATIONS
            && Observer::EXACT_INTEGER_NARROW_CONTINUATIONS }
        #[cfg(not(feature = "internal-checker"))]
        { let _ = collecting; false }
    }

    fn projected_integer_narrow_shape(
        &self, body: &FunctionCallSites, effect: ExactCallEffect<'_>, collecting: bool,
    ) -> bool {
        #[cfg(feature = "internal-checker")]
        {
            !ACTUAL_STARTUP && collecting && Observer::INTEGER_RETURN_PARTITIONS
                && self.observer.permits_ordinary_projected_effects()
                && matches!(effect, ExactCallEffect::Scalar { binding: Some(_), ty, .. } if is_integer_type(ty))
                && body.structure.checker_has_simple_narrow && !body.structure.checker_has_nonsimple_directive
                && body.structure.blocks.len() == 1 && body.boundary.parameters.len() == 1
                && body.call_occurrences.is_empty() && body.allocation_length_calls.is_empty()
                && body.assignments.is_empty() && body.arms.is_empty()
                && body.structure.checker_contextual_occurrences.is_empty()
        }
        #[cfg(not(feature = "internal-checker"))]
        { let _ = (body, effect, collecting); false }
    }

    fn exact_integer_narrow_shape(&self, body: &FunctionCallSites, expected: &ScalarType, collecting: bool) -> bool {
        #[cfg(feature = "internal-checker")]
        {
            self.exact_integer_narrow_profile(collecting) && is_integer_type(expected)
                && body.structure.checker_has_simple_narrow && !body.structure.checker_has_nonsimple_directive
                && body.allocation_length_calls.is_empty() && body.assignments.is_empty()
        }
        #[cfg(not(feature = "internal-checker"))]
        { let _ = (body, expected, collecting); false }
    }

    fn exact_integer_local_write_shape(&self, body: &FunctionCallSites, expected: &ScalarType, collecting: bool) -> bool {
        #[cfg(feature = "internal-checker")]
        {
            ACTUAL_STARTUP && collecting && Observer::EXACT_INTEGER_CALL_CONTINUATIONS
                && Observer::EXACT_INTEGER_LOCAL_WRITES && is_integer_type(expected)
                && !body.assignments.is_empty() && body.allocation_length_calls.is_empty()
                && !body.structure.checker_has_nonsimple_directive
                && (!body.structure.checker_has_simple_narrow || Observer::EXACT_INTEGER_NARROW_CONTINUATIONS)
        }
        #[cfg(not(feature = "internal-checker"))]
        { let _ = (body, expected, collecting); false }
    }

    fn exact_self_entry<'entry>(&self, exact: Option<&[&'entry BigInt]>) -> Option<&'entry BigInt> {
        if ACTUAL_STARTUP && Observer::EXACT_SELF_REENTRY {
            if let Some([value]) = exact {
                return Some(*value);
            }
        }
        None
    }

    fn validated_direct_body(
        &self, structure: &OccurrenceStructure, call: OccurrenceOrdinal,
        target_source: &SourceIdentity, target_span: ByteSpan,
    ) -> Result<((usize, usize), &'project ScalarModule, &'project ScalarFunction, &'project FunctionCallSites), ProjectCheck> {
        let &target = structure.checker_direct_targets.get(&call)
            .ok_or(ProjectCheck::MissingRootOccurrences)?;
        self.validated_prepared_body(target, target_source, target_span)
    }

    fn validated_prepared_body(
        &self, target: (usize, usize), target_source: &SourceIdentity, target_span: ByteSpan,
    ) -> Result<((usize, usize), &'project ScalarModule, &'project ScalarFunction, &'project FunctionCallSites), ProjectCheck> {
        let (target_module, target_item) = target;
        let owner = self.project.modules.get(target_module).ok_or(ProjectCheck::MissingRoot)?;
        let Some(ScalarItem::Function(function)) = owner.items.get(target_item) else {
            return Err(ProjectCheck::MissingRoot);
        };
        if &owner.source != target_source || function.name_span != target_span {
            return Err(ProjectCheck::MissingRoot);
        }
        let body = self.calls.function_body(target_module, target_item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
        Ok((target, owner, function, body))
    }

    fn run_exact_calls(
        &mut self, body: &'project FunctionCallSites, source: &'project SourceIdentity,
        exact: &[&BigInt], expected: &ScalarType, collection: Option<OrderedCallCollection<'_, '_>>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        #[allow(unused_mut)]
        let mut cursor = ExactCallCursor::new(body, exact, expected).ok_or(ProjectCheck::Unsupported)?;
        #[cfg(feature = "internal-checker")]
        if self.exact_integer_narrow_profile(collection.is_some()) { cursor.allow_empty_integer(); }
        #[cfg(feature = "internal-checker")]
        if self.exact_integer_local_write_shape(body, expected, collection.is_some()) {
            cursor.restrict_returning_writes_to_direct_locals();
        }
        self.run_call_cursor(body, source, cursor, self.exact_self_entry(Some(exact)).cloned(), collection)
    }

    fn run_call_cursor(
        &mut self, body: &'project FunctionCallSites, source: &'project SourceIdentity,
        cursor: ExactCallCursor<'project>, entry: Option<BigInt>, collection: Option<OrderedCallCollection<'_, '_>>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let mut frames = vec![ExactCallActivation { body, source, cursor, entry, followed_self_edge: false, target: None }];
        let result = self.drive_exact_calls(&mut frames, collection);
        for frame in frames.into_iter().rev() {
            if let Some((target, first_activation, _, _)) = frame.target {
                self.active_depth -= 1;
                if first_activation { self.active.remove(&target); }
            }
        }
        result
    }

    fn drive_exact_calls(
        &mut self, frames: &mut Vec<ExactCallActivation<'project>>,
        collection: Option<OrderedCallCollection<'_, '_>>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        loop {
            #[cfg(feature = "internal-checker")]
            let active_collection = if frames.len() == 1 { collection } else {
                collection.map(|collection| OrderedCallCollection { outer: None, ..collection })
            };
            #[cfg(not(feature = "internal-checker"))]
            let active_collection = collection;
            let frame = frames.last_mut().ok_or(ProjectCheck::Unsupported)?;
            let (body, source) = (frame.body, frame.source);
            let mut sink = |failure| {
                if let Some(collection) = active_collection {
                    collection.failures.borrow_mut().push_callable(body, source, failure);
                }
            };
            let mut nested_error = None;
            let mut nested_value = |caller: &FunctionCallSites, occurrence, signature: &CheckedCallSignature, argument: NestedIntegerArgument<'_>, expected: &ScalarType| {
                match self.nested(caller, occurrence, signature, argument, expected,
                    active_collection.map(|active| (source, active)))
                {
                    Ok(result) => Some(result),
                    Err(error) => { nested_error = Some(error); None }
                }
            };
            let handler = Some(&mut nested_value as &mut super::checker_flow::ActualCallHandler<'_, '_>);
            let progress = if active_collection.is_some() {
                frame.cursor.advance::<true>(body, handler, Observer::PROOF_FREE_CONTEXTUAL_LOCAL_TRANSFERS,
                    Observer::EXACT_INTEGER_CALL_CONTINUATIONS, Observer::ORDINARY_PROJECTED_PAIR,
                    Observer::ORDINARY_INTEGER_LOCALS, &mut sink)
            } else {
                frame.cursor.advance::<false>(body, handler, false, false, false, false, &mut sink)
            }.ok_or_else(|| nested_error.unwrap_or(ProjectCheck::Unsupported))?;
            match progress {
                ExactCallProgress::Pending(call) => {
                    let child = self.prepare_exact_child(frame, call, active_collection)?;
                    frames.push(child);
                }
                ExactCallProgress::Failed(failure) => return Err(project_failure(body, source, failure)),
                ExactCallProgress::Done { active, result } => {
                    self.had_obligations |= active;
                    let completed = frames.pop().unwrap();
                    let Some((target, first_activation, effect, caller_axis)) = completed.target else { return Ok(result); };
                    #[cfg(feature = "internal-checker")]
                    let hypothetical = frames.last().is_some_and(|parent|
                        parent.body.structure.checker_ordinary_directive_calls.contains_key(&effect.occurrence()));
                    #[cfg(not(feature = "internal-checker"))]
                    let hypothetical = false;
                    if !hypothetical { self.reached.insert(target); }
                    self.active_depth -= 1;
                    if first_activation { self.active.remove(&target); }
                    frames.last_mut().ok_or(ProjectCheck::Unsupported)?.cursor.resume(effect, caller_axis, result)
                        .ok_or(ProjectCheck::Unsupported)?;
                }
            }
        }
    }

    fn prepare_exact_child(
        &mut self, parent: &mut ExactCallActivation<'project>, effect: ExactCallEffect<'project>,
        collection: Option<OrderedCallCollection<'_, '_>>,
    ) -> Result<ExactCallActivation<'project>, ProjectCheck> {
        if self.active_depth == MAX_ACTUAL_CALL_DEPTH { return Err(ProjectCheck::Unsupported); }
        let caller = parent.body;
        let call = effect.occurrence();
        let expected = effect.expected();
        let prepared = {
            #[cfg(feature = "internal-checker")]
            if let Some(receipt) = caller.structure.checker_ordinary_directive_calls.get(&call) {
                if !Observer::ORDINARY_INTEGER_LOCALS || collection.is_none() || *expected != ScalarType::Bool {
                    return Err(ProjectCheck::Unsupported);
                }
                let (target, owner, _, body) = self.validated_prepared_body(receipt.target, &receipt.source, receipt.declaration)?;
                if parent.source != &owner.source || !body.structure.checker_directives.is_empty()
                    || !body.call_occurrences.is_empty() || !body.allocation_length_calls.is_empty()
                    || !body.assignments.is_empty() || !body.arms.is_empty()
                    || !body.structure.checker_contextual_occurrences.is_empty()
                {
                    return Err(ProjectCheck::Unsupported);
                }
                let signature = caller.structure.checker_directives.get(&call)
                    .and_then(|directive| directive.call_signature.as_ref()).ok_or(ProjectCheck::MissingRootOccurrences)?;
                Some((target, owner, body, signature))
            } else { None }
            #[cfg(not(feature = "internal-checker"))]
            { None }
        };
        let (target, owner, body, signature) = if let Some(prepared) = prepared {
            prepared
        } else {
            let site = caller.resolved_call_site(call).ok_or(ProjectCheck::MissingRootOccurrences)?;
            let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration } =
                &site.target else { return Err(ProjectCheck::Unsupported); };
            let (target, owner, _, body) = self.validated_direct_body(&caller.structure, call, source, *declaration_span)?;
            let signature = caller.checked_call_signatures.get(&call).ok_or(ProjectCheck::Unsupported)?;
            (target, owner, body, signature)
        };
        if collection.is_some_and(|collection| parent.source != &owner.source
            && (*expected != ScalarType::Unit || parent.source.project != owner.source.project || parent.source.package != owner.source.package
                || collection.visits.and_then(|visits| visits.get(target.0)) != Some(&InitializationVisit::Completed)))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let projected_narrow = self.projected_integer_narrow_shape(body, effect, collection.is_some());
        if !matches!(signature.outputs.outputs.as_slice(), [output] if &output.ty == expected)
            || (*expected != ScalarType::Unit && (!Observer::EXACT_INTEGER_CALL_CONTINUATIONS
                || !(supported_integer_continuation_shape_for(body, expected, Observer::ORDINARY_INTEGER_LOCALS)
                    || self.exact_integer_narrow_shape(body, expected, collection.is_some())
                    || self.exact_integer_local_write_shape(body, expected, collection.is_some())
                    || projected_narrow)))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let arguments = parent.cursor.arguments(
            caller, call, signature.parameters.len(), collection.is_some(), Observer::INTEGER_RETURN_PARTITIONS,
            Observer::ORDINARY_PROJECTED_PAIR,
            Observer::ORDINARY_PROJECTED_PAIR,
            Observer::ORDINARY_INTEGER_LOCALS && matches!(*expected, ScalarType::Unit | ScalarType::Bool),
        )
            .ok_or(ProjectCheck::Unsupported)?;
        if let NestedIntegerArgument::Projected(mut projection) = arguments {
            if !Observer::INTEGER_RETURN_PARTITIONS
                || (!Observer::PROJECTED_FORMAL_WRITES && !body.assignments.is_empty())
                || self.active.contains(&target)
                || (projected_narrow && !matches!(projection.companion, ProjectedCompanion::Unary))
                || !projected_call_types_match(signature, body, &projection)
                || (!matches!(projection.companion, ProjectedCompanion::Unary) && !Observer::ORDINARY_PROJECTED_PAIR)
                || (matches!(projection.companion, ProjectedCompanion::Equal | ProjectedCompanion::Independent { .. })
                    && !body.assignments.is_empty())
            {
                return Err(ProjectCheck::Unsupported);
            }
            #[allow(unused_mut)]
            let mut cursor = if let ProjectedCompanion::Independent { axis, .. } = projection.companion {
                ExactCallCursor::projected_pair(body, &mut projection, axis, expected)
            } else {
                let values = projection.materialize_bounded().ok_or(ProjectCheck::Unsupported)?;
                ExactCallCursor::projected_arguments(body, values, &projection, expected)
            }.ok_or(ProjectCheck::Unsupported)?;
            #[cfg(feature = "internal-checker")]
            if projected_narrow { cursor.allow_empty_projected_integer().ok_or(ProjectCheck::Unsupported)?; }
            #[cfg(feature = "internal-checker")]
            if !ACTUAL_STARTUP && collection.is_some() && self.observer.permits_ordinary_projected_effects()
                && matches!(effect, ExactCallEffect::Scalar { binding: Some(_), ty, .. } if is_integer_type(ty))
                && matches!(projection.companion, ProjectedCompanion::Unary)
            {
                cursor.allow_projected_scalar_local_write().ok_or(ProjectCheck::Unsupported)?;
            }
            self.active.insert(target);
            self.active_depth += 1;
            return Ok(ExactCallActivation {
                body, source: &owner.source, cursor, entry: None, followed_self_edge: false,
                target: Some((target, true, effect, Some(projection.caller_axis))),
            });
        }
        // Only the unary projected evaluator has this partial-return contract.
        if projected_narrow { return Err(ProjectCheck::Unsupported); }
        let restricted_local_writes = self.exact_integer_local_write_shape(body, expected, collection.is_some());
        if *expected != ScalarType::Unit && !body.assignments.is_empty() && !restricted_local_writes {
            return Err(ProjectCheck::Unsupported);
        }
        // Mutating integer-return bodies cannot use the ranked self proof.
        let ranked = !restricted_local_writes && self.exact_self_reentry_allowed(expected, collection.is_some());
        let argument;
        let pair;
        #[cfg(feature = "internal-checker")]
        let computed;
        let exact: &[&BigInt] = match (signature.parameters.as_slice(), body.boundary.parameters.as_slice(), arguments) {
            ([], [], NestedIntegerArgument::Single(None)) => &[],
            ([parameter], [slot], NestedIntegerArgument::Single(Some((ty, value)))) if *parameter == slot.ty && parameter == ty => {
                argument = [value];
                &argument
            }
            #[cfg(feature = "internal-checker")]
            ([parameter], [slot], NestedIntegerArgument::Single(None)) if *parameter == slot.ty => {
                if !ranked || !std::ptr::eq(caller, body) || parent.followed_self_edge {
                    return Err(ProjectCheck::Unsupported);
                }
                computed = self.computed_self_descent(caller, call, parameter,
                    parent.entry.as_ref().ok_or(ProjectCheck::Unsupported)?)?;
                argument = [&computed];
                &argument
            }
            ([first_param, second_param], [first_slot, second_slot], NestedIntegerArgument::Pair((first_ty, first), (second_ty, second)))
                if *first_ty == first_slot.ty && *second_ty == second_slot.ty
                    && first_ty == first_param && second_ty == second_param =>
            {
                pair = [first, second];
                &pair
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        let first_activation = !self.active.contains(&target);
        if !first_activation && !matches!(
            (parent.entry.as_ref(), exact),
            (Some(previous), [value]) if ranked && !parent.followed_self_edge && std::ptr::eq(caller, body)
                && value.sign() != num_bigint::Sign::Minus && *value < previous
        ) {
            return Err(ProjectCheck::Unsupported);
        }
        #[allow(unused_mut)]
        let mut cursor = ExactCallCursor::new(body, exact, expected).ok_or(ProjectCheck::Unsupported)?;
        #[cfg(feature = "internal-checker")]
        if self.exact_integer_narrow_profile(collection.is_some()) { cursor.allow_empty_integer(); }
        #[cfg(feature = "internal-checker")]
        if restricted_local_writes { cursor.restrict_returning_writes_to_direct_locals(); }
        if first_activation {
            self.active.insert(target);
        } else {
            parent.followed_self_edge = true;
        }
        self.active_depth += 1;
        Ok(ExactCallActivation {
            body, source: &owner.source, cursor, entry: self.exact_self_entry(Some(exact)).cloned(),
            followed_self_edge: false, target: Some((target, first_activation, effect, None)),
        })
    }

    fn run(
        &mut self,
        structure: &OccurrenceStructure,
        call: OccurrenceOrdinal,
        signature: &CheckedCallSignature,
        projected: ActualProjectedArguments<'_>,
        target_source: &SourceIdentity,
        target_span: ByteSpan,
        expected: &ScalarType,
        collection: Option<(&SourceIdentity, OrderedCallCollection<'_, '_>)>,
        #[cfg(feature = "internal-checker")] proof_site: Option<InitializerProofSite<'_>>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let (target, owner, function, body) = self.validated_direct_body(structure, call, target_source, target_span)?;
        let target_module = target.0;
        if !ACTUAL_STARTUP && (*expected != ScalarType::Unit
            || (collection.is_none() && target_module != self.symbolic_module))
        {
            return Err(ProjectCheck::Unsupported);
        }
        if !matches!(signature.outputs.outputs.as_slice(), [output] if &output.ty == expected) {
            return Err(ProjectCheck::Unsupported);
        }
        let collection = if let Some((caller_source, collection)) = collection {
            if caller_source != &owner.source
                && (*expected != ScalarType::Unit || matches!(&projected, ActualProjectedArguments::SymbolicAxis(..))
                    || caller_source.project != owner.source.project || caller_source.package != owner.source.package
                    || collection.visits.and_then(|visits| visits.get(target_module)) != Some(&InitializationVisit::Completed))
            {
                return Err(ProjectCheck::Unsupported);
            }
            #[cfg(feature = "internal-checker")]
            let collection = if caller_source != &owner.source {
                OrderedCallCollection { outer: None, ..collection }
            } else {
                collection
            };
            Some(collection)
        } else {
            None
        };
        let projected = match projected {
            ActualProjectedArguments::SymbolicAxis(projection) => {
                let Some(collection) = collection else { return Err(ProjectCheck::Unsupported); };
                let slot = projection.slot;
                let fixed = match projection.companion {
                    ProjectedCompanion::Unary => None,
                    ProjectedCompanion::Fixed((_, value)) => Some(value),
                    ProjectedCompanion::Equal | ProjectedCompanion::Independent { .. } => return Err(ProjectCheck::Unsupported),
                };
                if ACTUAL_STARTUP || *expected != ScalarType::Unit || !projected_call_types_match(signature, body, &projection)
                    || !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
                    || !body.structure.checker_direct_targets.is_empty()
                    || !body.call_occurrences.is_empty() || !body.allocation_length_calls.is_empty()
                    || !body.assignments.is_empty() || !body.arms.is_empty()
                    || body.structure.checker_directives.values().any(|directive| directive.call_signature.is_some())
                    || self.active_depth == MAX_ACTUAL_CALL_DEPTH || self.active.contains(&target)
                {
                    return Err(ProjectCheck::Unsupported);
                }
                let values = projection.materialize().ok_or(ProjectCheck::Unsupported)?;
                let outcome = self.collect_requirements(
                    body, &owner.source, IntegerRequirementEntry::Projected {
                        slot, values: Rc::clone(&values), fixed,
                    }, collection,
                )?;
                return Ok(match outcome {
                    ActualCallResult::ProjectedUnit(survivors) if Rc::ptr_eq(&values, &survivors) => ActualCallResult::Unit,
                    outcome => outcome,
                });
            }
            exact => exact,
        };
        let argument;
        let pair;
        let exact: &[&BigInt] = match (signature.parameters.as_slice(), body.boundary.parameters.as_slice(), projected) {
            ([], [], ActualProjectedArguments::Single(None)) => &[],
            ([parameter], [slot], ActualProjectedArguments::Single(Some((ty, value))))
                if *parameter == slot.ty && parameter == ty =>
            {
                argument = [value];
                &argument
            }
            ([first, second], [first_slot, second_slot],
                ActualProjectedArguments::ExactPair((first_ty, first_value), (second_ty, second_value)))
                if (is_integer_type(expected) || (*expected == ScalarType::Unit && collection.is_some()))
                    && *first == first_slot.ty && first == first_ty
                    && *second == second_slot.ty && second == second_ty =>
            {
                pair = [first_value, second_value];
                &pair
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        if self.active_depth == MAX_ACTUAL_CALL_DEPTH {
            return Err(ProjectCheck::Unsupported);
        }
        if !self.active.insert(target) {
            return Err(ProjectCheck::Unsupported);
        }
        self.active_depth += 1;
        let result = if *expected == ScalarType::Unit {
            if ACTUAL_STARTUP && !Observer::ENABLED && !Observer::CONTEXTUAL {
                let result = self.run_exact_calls(body, &owner.source, exact, expected, collection);
                self.active_depth -= 1;
                self.active.remove(&target);
                if result.is_ok() { self.reached.insert(target); }
                return result;
            }
            if let Some(collection) = collection {
                self.collect_requirements(body, &owner.source, IntegerRequirementEntry::Exact(exact), collection)
            } else {
                self.requirements(body, &owner.source, Some(exact)).map(|active| {
                    self.had_obligations |= active;
                    ActualCallResult::Unit
                })
            }
        } else if (ACTUAL_STARTUP && Observer::EXACT_INTEGER_CALL_CONTINUATIONS && collection.is_some()
            && (!body.call_occurrences.is_empty() || {
                #[cfg(feature = "internal-checker")]
                { Observer::ORDINARY_INTEGER_LOCALS && body.structure.has_ordinary_integer_operations() }
                #[cfg(not(feature = "internal-checker"))]
                { false }
            })
            && supported_integer_continuation_shape_for(body, expected, Observer::ORDINARY_INTEGER_LOCALS)
            && body.assignments.is_empty())
            || self.exact_integer_narrow_shape(body, expected, collection.is_some())
            || self.exact_integer_local_write_shape(body, expected, collection.is_some())
        {
            self.run_exact_calls(body, &owner.source, exact, expected, collection)
        } else if is_integer_type(expected)
            && body.structure.checker_directives.is_empty()
            && body.call_occurrences.is_empty()
            && body.allocation_length_calls.is_empty()
            && body.assignments.is_empty()
        {
            exact_integer_return_at_entry(body, expected, exact).map(ActualCallResult::Integer)
                .ok_or(ProjectCheck::Unsupported)
        } else {
            #[cfg(feature = "internal-checker")]
            let collected = collection.filter(|_| ACTUAL_STARTUP && is_integer_type(expected)
                && !body.structure.checker_directives.is_empty()
                && !body.structure.checker_has_nonexact_require
                && body.call_occurrences.is_empty() && body.allocation_length_calls.is_empty() && body.assignments.is_empty()
            ).map(|collection| {
                let mut failures = collection.failures.borrow_mut();
                let mut sink = |failure| failures.push_callable(body, &owner.source, failure);
                super::checker_flow::collect_exact_integer_return_requirements(body, expected, exact, &mut sink)
                    .map(|(active, value)| {
                        self.had_obligations |= active;
                        ActualCallResult::Integer(value)
                    }).ok_or(ProjectCheck::Unsupported)
            });
            #[cfg(not(feature = "internal-checker"))]
            let collected = None;
            if let Some(result) = collected {
                result
            } else {
                #[cfg(feature = "internal-checker")]
                let proved = if ACTUAL_STARTUP && Observer::ENABLED {
                    self.observer.proved_recursive_return(target, exact, expected)
                } else {
                    None
                };
                #[cfg(not(feature = "internal-checker"))]
                let proved = None;
                proved.map(ActualCallResult::Integer).ok_or(ProjectCheck::Unsupported)
            }
        };
        self.active_depth -= 1;
        self.active.remove(&target);
        if ACTUAL_STARTUP && Observer::ENABLED {
            if let Ok(ActualCallResult::Integer(value)) = &result {
                self.observer.observe(target, exact, value);
            }
        }
        #[cfg(feature = "internal-checker")]
        if ACTUAL_STARTUP && Observer::CONTEXTUAL {
            if let Ok(ActualCallResult::Integer(value)) = &result {
                let mut proof_site = proof_site;
                if let (Some(InitializerProofSite { call: InitializerProofOrigin::Direct { directive_position, .. }, .. }), Some(collection)) =
                    (&mut proof_site, collection)
                {
                    *directive_position = collection.failures.borrow().records.len();
                }
                self.observer.observe_initializer(target, proof_site, &signature.parameters, exact, value);
            }
        }
        if ACTUAL_STARTUP && result.is_ok() {
            self.reached.insert(target);
        }
        result
    }
}

fn projected_call_types_match(
    signature: &CheckedCallSignature, body: &FunctionCallSites, projection: &ProjectedIntegerArgument<'_>,
) -> bool {
    let ProjectedIntegerArgument { ty, slot, companion, .. } = *projection;
    match (signature.parameters.as_slice(), body.boundary.parameters.as_slice(), companion) {
        ([parameter], [entry], ProjectedCompanion::Unary) => slot == 0 && parameter == ty && entry.ty == *ty,
        (parameters @ [_, _], entries @ [_, _], ProjectedCompanion::Fixed((fixed_ty, _))) if slot < 2 =>
            parameters[slot] == *ty && entries[slot].ty == *ty
                && parameters[1 - slot] == *fixed_ty && entries[1 - slot].ty == *fixed_ty,
        ([first, second], [left, right], ProjectedCompanion::Equal) =>
            slot == 0 && first == ty && second == ty && left.ty == *ty && right.ty == *ty,
        ([first, second], [left, right], ProjectedCompanion::Independent { ty: other, .. }) =>
            slot == 0 && first == ty && second == other && left.ty == *ty && right.ty == *other,
        _ => false,
    }
}

fn supported_integer_continuation(body: &FunctionCallSites, expected: &ScalarType) -> bool {
    supported_integer_continuation_shape(body, expected) && body.assignments.is_empty()
}

fn supported_integer_continuation_shape(body: &FunctionCallSites, expected: &ScalarType) -> bool {
    #[cfg(feature = "internal-checker")]
    let exact_requirements = !body.structure.checker_has_nonexact_require;
    #[cfg(not(feature = "internal-checker"))]
    let exact_requirements = false;
    is_integer_type(expected) && exact_requirements
        && body.allocation_length_calls.is_empty()
}

fn supported_integer_continuation_shape_for(body: &FunctionCallSites, expected: &ScalarType, ordinary: bool) -> bool {
    if supported_integer_continuation_shape(body, expected) { return true; }
    #[cfg(feature = "internal-checker")]
    if ordinary && (is_integer_type(expected) || *expected == ScalarType::Bool)
        && body.allocation_length_calls.is_empty()
        && (body.structure.has_ordinary_predicates() || *expected == ScalarType::Bool)
    {
        return body.structure.checker_directives.iter().all(|(occurrence, directive)| {
            directive.operation == CheckerOperation::Require
                && (matches!(directive.predicate, Some(CheckerPredicate::Boolean(_) | CheckerPredicate::Integer(_)))
                    || body.structure.checker_ordinary_integer_comparisons.contains_key(occurrence)
                    || body.structure.checker_ordinary_booleans.contains_key(occurrence)
                    || body.structure.checker_ordinary_directive_calls.contains_key(occurrence))
        });
    }
    #[cfg(not(feature = "internal-checker"))]
    let _ = ordinary;
    false
}

#[cfg(test)]
fn check_closed_project_callable(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    root_source: &SourceIdentity,
    root_name: &str,
) -> ProjectCheck {
    let prepared = match prepare_checker_project(declarations, initialization_order, inputs) {
        Ok(prepared) => prepared,
        Err(error) => return error,
    };
    let (project, calls) = prepared.parts();
    let (module, item, function) = match select_checked_root(project, root_source, root_name) {
        Ok(root) => root,
        Err(error) => return error,
    };
    let ScalarType::Callable { outputs, parameters } = &function.signature else {
        return ProjectCheck::Unsupported;
    };
    if !parameters.is_empty()
        || !matches!(outputs.outputs.as_slice(), [output] if output.ty == ScalarType::Unit)
        || !function.generic_parameters.is_empty()
        || !function.overload_arms.is_empty()
    {
        return ProjectCheck::Unsupported;
    }
    let Some(body) = calls.function_body(module, item, None) else {
        return ProjectCheck::MissingRootOccurrences;
    };
    match check_integer_requirements(body) {
        None => ProjectCheck::Unsupported,
        Some(Ok(())) if body.structure.checker_directives.is_empty() => ProjectCheck::NoChecks,
        Some(Ok(())) => ProjectCheck::Proved,
        Some(Err(failure)) => project_failure(body, root_source, failure),
    }
}

use prepared::PreparedCheckerProject;

mod prepared {
    use super::*;

    pub(super) struct PreparedCheckerProject {
        project: ScalarProject,
        calls: ProjectCallSiteResults,
    }

    impl PreparedCheckerProject {
        pub(super) fn parts(&self) -> (&ScalarProject, &ProjectCallSiteResults) {
            (&self.project, &self.calls)
        }
    }

    pub(super) fn prepare_checker_project(
        declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
    ) -> Result<PreparedCheckerProject, ProjectCheck> {
        prepare_checker_project_impl(declarations, initialization_order, inputs, &[])
    }

    pub(super) fn prepare_checker_project_with_queries(
        declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
        queries: &[CheckerTargetQuery<'_>],
    ) -> Result<PreparedCheckerProject, ProjectCheck> {
        prepare_checker_project_impl(declarations, initialization_order, inputs, queries)
    }

    fn prepare_checker_project_impl(
        declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
        queries: &[CheckerTargetQuery<'_>],
    ) -> Result<PreparedCheckerProject, ProjectCheck> {
        let (project, calls) = project_from_declarations(declarations, initialization_order, inputs)
            .and_then(|project| prepare_project_occurrences(
                typecheck_scalar_project_in_mode(project, true),
                queries,
            ))
            .map_err(|rejected| ProjectCheck::Invalid(rejected.into_inspection().diagnostics))?;
        Ok(PreparedCheckerProject { project, calls })
    }
}

use prepared::prepare_checker_project;

fn select_checked_root<'a>(
    project: &'a ScalarProject,
    source: &SourceIdentity,
    name: &str,
) -> Result<(usize, usize, &'a ScalarFunction), ProjectCheck> {
    let mut roots = project
        .modules
        .iter()
        .enumerate()
        .filter(|(_, module)| &module.source == source)
        .flat_map(|(module, source)| {
            source.items.iter().enumerate().filter_map(move |(item, value)| {
                let ScalarItem::Function(function) = value else {
                    return None;
                };
                (function.name == name).then_some((module, item, function))
            })
        });
    let root = roots.next().ok_or(ProjectCheck::MissingRoot)?;
    if roots.next().is_some() {
        return Err(ProjectCheck::AmbiguousRoot);
    }
    if !project.initialization_order.contains(source) {
        return Err(ProjectCheck::RootSourceOutsideInitOrder);
    }
    Ok(root)
}

#[derive(Default)]
struct OrderedFailures {
    records: Vec<ScalarOrderedCheckerFailure>,
    payloads: OrderedFailurePayloads,
    #[cfg(feature = "internal-checker")]
    has_authored_state: bool,
    #[cfg(feature = "internal-checker")]
    snapshot_records: Vec<usize>,
    #[cfg(feature = "internal-checker")]
    entry_evidence: Option<Box<ScalarOrderedEntryEvidence>>,
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub struct ScalarOrderedEntryEvidence {
    pub callable: wosy_syntax::SourceSpan,
    pub directive_index: usize,
    pub ty: ScalarType,
    pub entry_lower: BigInt,
    pub entry_upper: BigInt,
    pub failure_value: BigInt,
}

#[derive(Default)]
enum OrderedFailurePayloads {
    #[default]
    Empty,
    One(*const PreparedCheckerDirective, Rc<CheckedLiteralOutput>),
    Many(HashMap<*const PreparedCheckerDirective, Rc<CheckedLiteralOutput>>),
}

impl OrderedFailurePayloads {
    fn shared(&mut self, directive: &PreparedCheckerDirective) -> Rc<CheckedLiteralOutput> {
        let origin = std::ptr::from_ref(directive);
        match self {
            Self::Empty => {
                let output = Rc::new(directive.output.clone());
                *self = Self::One(origin, Rc::clone(&output));
                output
            }
            Self::One(prior, output) if *prior == origin => Rc::clone(output),
            Self::One(prior, previous) => {
                let next = Rc::new(directive.output.clone());
                *self = Self::Many(HashMap::from([(*prior, Rc::clone(previous)), (origin, Rc::clone(&next))]));
                next
            }
            Self::Many(payloads) => Rc::clone(payloads.entry(origin).or_insert_with(|| Rc::new(directive.output.clone()))),
        }
    }
}

impl OrderedFailures {
    fn push(
        &mut self,
        source: &SourceIdentity,
        directive: &PreparedCheckerDirective,
        span: ByteSpan,
        state: IntegerInputs,
        axes: Vec<(ReferenceBindingId, ScalarType)>,
        equal_axes: Vec<(usize, usize)>,
    ) {
        #[cfg(feature = "internal-checker")]
        { self.has_authored_state |= !directive.output.state_files.is_empty(); }
        self.records.push(ScalarOrderedCheckerFailure {
            source: source.clone(),
            operation: directive.operation.into(),
            directive: span,
            condition: directive.condition,
            witness: ScalarCheckerIntegerWitness { axes, state, equal_axes },
            output: self.payloads.shared(directive),
        });
    }

    #[cfg(feature = "internal-checker")]
    fn capture_entry_evidence(
        &mut self, body: &FunctionCallSites, function: &ScalarFunction, source: &SourceIdentity, failure: &IntegerCheckFailure<'_>,
    ) {
        if self.entry_evidence.is_some() {
            return;
        }
        let Some((ty, entry_lower, entry_upper, failure_value)) =
            super::checker_flow::backward_literal_overwrite_failure(body, failure) else { return; };
        self.entry_evidence = Some(Box::new(ScalarOrderedEntryEvidence {
            callable: wosy_syntax::SourceSpan { source: source.clone(), range: function.name_span },
            directive_index: self.records.len(), ty, entry_lower, entry_upper, failure_value,
        }));
    }

    fn push_callable(&mut self, body: &FunctionCallSites, source: &SourceIdentity, failure: IntegerCheckFailure<'_>) {
        #[cfg(feature = "internal-checker")]
        if !body.structure.checker_selected_snapshots.is_empty() {
            self.snapshot_records.push(self.records.len());
        }
        self.push(
            source,
            &body.structure.checker_directives[&failure.directive],
            body.occurrences[failure.directive.0].span,
            failure.state,
            failure.axes.into_iter().map(|(binding, ty)| (binding.clone(), ty.clone())).collect(),
            failure.equal_axes,
        );
    }
}

#[cfg(feature = "internal-checker")]
fn render_authored_state(record: &ScalarOrderedCheckerFailure, work: &mut usize) -> Option<CheckedLiteralOutput> {
    use super::{checker_directive::{CheckerStateSelection, CheckerStateType, ScalarCheckerTextFile}, checker_flow::charge_call_work};
    let prepared = &record.output;
    let count = prepared.text_files.len().checked_add(prepared.state_files.len())?;
    let mut copied_bytes = prepared.stdout_bytes.len().checked_add(prepared.stderr_bytes.len())?;
    for file in prepared.text_files.iter() {
        charge_call_work(work, 1)?;
        copied_bytes = copied_bytes.checked_add(file.path.len())?.checked_add(file.text.len())?;
    }
    for file in prepared.state_files.iter() {
        charge_call_work(work, 1)?;
        copied_bytes = copied_bytes.checked_add(file.path.len())?;
    }
    charge_call_work(work, count.checked_add(copied_bytes.div_ceil(32))?)?;
    let mut output = (**prepared).clone();
    let mut text = std::mem::take(&mut output.text_files).into_vec().into_iter();
    let mut selected = std::mem::take(&mut output.state_files).into_vec().into_iter().peekable();
    let mut files = Vec::with_capacity(count);
    // Output paths may select the same authored binding. Reuse the exact
    // projection and encoding only within this one immutable witness.
    let mut encoded = HashMap::new();
    let mut encoded_shapes = HashMap::new();
    for position in 0..count {
        if selected.peek().is_some_and(|file| file.position == position) {
            let file = selected.next()?;
            let bytes: &Box<[u8]> = match (&file.selection, &file.ty) {
                (CheckerStateSelection::Nominal(shape), CheckerStateType::Nominal(_)) => {
                    // Shape addresses are stable: the prepared directive owns
                    // every shape for this entire materialization.
                    match encoded_shapes.entry(Rc::as_ptr(shape)) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            let mut json = String::new();
                            encode_nominal_state_node(&shape.root, shape, record, work, &mut json)?;
                            json.push('\n');
                            entry.insert(json.into_bytes().into_boxed_slice())
                        }
                    }
                }
                (CheckerStateSelection::Binding(binding), CheckerStateType::Scalar(file_ty)) => {
                    let (ty, bytes) = match encoded.entry(binding.clone()) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            let json = encode_authored_scalar(record, binding, file_ty, work)?;
                            entry.insert((file_ty.clone(), json.into_bytes().into_boxed_slice()))
                        }
                    };
                    if ty != file_ty { return None; }
                    bytes
                }
                _ => return None,
            };
            // Each file owns its bytes; meter both copying and publication.
            charge_call_work(work, bytes.len().div_ceil(32).checked_mul(2)?)?;
            files.push(ScalarCheckerTextFile {
                path: file.path, path_span: file.path_span, text_span: file.state_span, text: bytes.clone(),
            });
        } else {
            files.push(text.next()?);
        }
    }
    output.text_files = files.into_boxed_slice();
    Some(output)
}

#[cfg(feature = "internal-checker")]
fn encode_authored_scalar(
    record: &ScalarOrderedCheckerFailure, binding: &ReferenceBindingId, ty: &ScalarType, work: &mut usize,
) -> Option<String> {
    use super::checker_flow::charge_call_work;
    charge_call_work(work, record.witness.axes.len().checked_add(1)?)?;
    let axis = record.witness.axes.iter().position(|(id, axis_ty)| id == binding && axis_ty == ty)?;
    let values = record.witness.state.project_axis_bounded(axis, work)?;
    charge_call_work(work, values.interval_count())?;
    serialize_scalar_analysis_state(ty, &values)
}

#[cfg(feature = "internal-checker")]
fn encode_nominal_state_node(
    node: &super::checker_directive::CheckerSnapshotNode,
    shape: &super::checker_directive::CheckerSnapshotShape,
    record: &ScalarOrderedCheckerFailure, work: &mut usize, json: &mut String,
) -> Option<()> {
    use super::{checker_directive::CheckerSnapshotNode, checker_flow::charge_call_work};
    match node {
        CheckerSnapshotNode::Leaf(index) => {
            let leaf = shape.leaves.get(*index)?;
            let node = encode_authored_scalar(record, &leaf.capture, &leaf.ty, work)?;
            json.push_str(node.strip_suffix('\n')?);
        }
        CheckerSnapshotNode::Nominal { name, fields } => {
            charge_call_work(work, 1usize.checked_add(name.len().div_ceil(32).checked_mul(6)?)?)?;
            json.push_str(r#"{"type":{"kind":"struct","name":"#);
            json.push_str(&serde_json::to_string(name.as_ref()).ok()?);
            json.push_str(r#"},"value":{"#);
            for (index, (name, child)) in fields.iter().enumerate() {
                charge_call_work(work, 1usize.checked_add(name.len().div_ceil(32).checked_mul(6)?)?)?;
                if index != 0 { json.push(','); }
                json.push_str(&serde_json::to_string(name.as_ref()).ok()?);
                json.push(':');
                encode_nominal_state_node(child, shape, record, work, json)?;
            }
            json.push_str("}}");
        }
    }
    Some(())
}

#[cfg(feature = "internal-checker")]
fn serialize_scalar_analysis_state(ty: &ScalarType, values: &IntegerSet) -> Option<String> {
    use std::fmt::Write as _;
    let (min, max) = values.extremes()?;
    let kind = match ty {
        ScalarType::Bool => "bool",
        ScalarType::I8 => "i8", ScalarType::I16 => "i16", ScalarType::I32 => "i32",
        ScalarType::I64 => "i64", ScalarType::I128 => "i128",
        ScalarType::U8 => "u8", ScalarType::U16 => "u16", ScalarType::U32 => "u32",
        ScalarType::U64 => "u64", ScalarType::U128 => "u128",
        _ => return None,
    };
    let mut result = format!(r#"{{"type":{{"kind":"{kind}"}},"value":"#);
    if *ty == ScalarType::Bool {
        let zero = BigInt::from(0u8);
        let one = BigInt::from(1u8);
        if min < &zero || max > &one { return None; }
        result.push_str(match (min == &zero, max == &one) {
            (true, true) => r#"{"kind":"alternatives","values":[false,true]}"#,
            (true, false) => "false",
            (false, true) => "true",
            _ => return None,
        });
    } else {
        let (floor, ceiling) = super::typecheck::integer_bounds(ty)?;
        if min < &floor || max > &ceiling { return None; }
        let alternatives = values.interval_count() > 1;
        if alternatives { result.push_str(r#"{"kind":"alternatives","values":["#); }
        for (i, (lo, hi)) in values.intervals().enumerate() {
            if i != 0 { result.push(','); }
            if lo == hi {
                write!(result, "{lo}").ok()?;
            } else {
                write!(result, r#"{{"kind":"range","minimum":{lo},"maximum":{hi}}}"#).ok()?;
            }
        }
        if alternatives { result.push_str("]}"); }
    }
    result.push_str("}\n");
    Some(result)
}

#[cfg(all(test, feature = "internal-checker"))]
fn ordered_failure_as_legacy(failure: ScalarOrderedCheckerFailure) -> ProjectCheck {
    let ScalarOrderedCheckerFailure {
        source, operation, directive, condition, witness: ScalarCheckerIntegerWitness { axes, state, .. }, output,
    } = failure;
    let operation = match operation {
        ScalarCheckerOperation::Require => CheckerOperation::Require,
        ScalarCheckerOperation::Narrow => CheckerOperation::Narrow,
    };
    ProjectCheck::Failed { source, operation, directive, condition, axes, state, output: Rc::unwrap_or_clone(output) }
}

fn project_failure(
    body: &FunctionCallSites,
    source: &SourceIdentity,
    failure: IntegerCheckFailure<'_>,
) -> ProjectCheck {
    selected_failure(
        source,
        &body.structure.checker_directives[&failure.directive],
        body.occurrences[failure.directive.0].span,
        failure.state,
        failure.axes.into_iter().map(|(binding, ty)| (binding.clone(), ty.clone())).collect(),
    )
}

fn selected_failure(
    source: &SourceIdentity,
    directive: &PreparedCheckerDirective,
    span: ByteSpan,
    state: IntegerInputs,
    axes: Vec<(ReferenceBindingId, ScalarType)>,
) -> ProjectCheck {
    ProjectCheck::Failed {
        source: source.clone(),
        operation: directive.operation,
        directive: span,
        condition: directive.condition,
        state,
        axes,
        output: directive.output.clone(),
    }
}

fn child(parent: &CstNode, kind: SyntaxKind) -> Result<CstNode, ProjectCheck> {
    parent.children().find(|node| node.kind() == kind).ok_or(ProjectCheck::InvalidProof)
}

fn identifiers(parent: &CstNode) -> impl Iterator<Item = String> + '_ {
    parent.children_with_tokens().filter_map(|element| {
        let token = element.into_token()?;
        (token.kind() == SyntaxKind::Identifier).then(|| token.text().to_owned())
    })
}

#[doc(hidden)]
pub struct ScalarProofModuleInput {
    pub source: SourceIdentity,
    pub text: String,
    pub namespaces: Vec<ScalarNamespaceBinding>,
}

#[doc(hidden)]
pub struct ScalarProofDeclarations {
    declarations: ScalarSourceDeclarations,
    definitions: Vec<CstNode>,
}

#[cfg(feature = "internal-checker")]
impl ScalarProofDeclarations {
    pub fn declarations(&self) -> &ScalarSourceDeclarations {
        &self.declarations
    }
}

pub(crate) fn derive_scalar_proof_declarations_from_cst(canonical: &CanonicalCstRoot) -> ScalarProofDeclarations {
    let mut definitions = Vec::new();
    let declarations = super::derive_scalar_declarations(canonical, Some(&mut definitions));
    ScalarProofDeclarations { declarations, definitions }
}

struct ModuleProofCatalog {
    source: SourceIdentity,
    definitions: Vec<CstNode>,
}

struct ModuleProofs {
    definitions: Vec<CstNode>,
    first_query: usize,
}

struct PreparedProofModule {
    source: SourceIdentity,
    proofs: HashMap<SourceIdentity, ModuleProofs>,
    prepared: PreparedCheckerProject,
}

struct SelectedModuleProof<'a> {
    node: &'a CstNode,
    function: (usize, usize, &'a ScalarFunction),
    target: &'a ResolvedCallTarget,
}

#[cfg(test)]
struct VerifiedModuleReturnBound<'a> {
    issuer: &'a PreparedProofModule,
    proof: &'a CstNode,
    bound: VerifiedReturnBound,
}

fn module_proof_name(proof: &CstNode) -> Option<CstToken> {
    proof.children_with_tokens().filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::Identifier)
}

impl PreparedProofModule {
    #[cfg(test)]
    fn prepare(source: SourceIdentity, text: String, inputs: ScalarPreparationInputs) -> Result<Self, ProjectCheck> {
        Self::prepare_project(
            source.clone(),
            vec![ScalarProofModuleInput { source: source.clone(), text, namespaces: Vec::new() }],
            vec![source],
            inputs,
        )
    }

    fn prepare_project(
        source: SourceIdentity,
        modules: Vec<ScalarProofModuleInput>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
    ) -> Result<Self, ProjectCheck> {
        let mut declarations = Vec::with_capacity(modules.len());
        let mut proofs = Vec::new();
        for module in modules {
            let parsed = wosy_syntax::parse_proof_module(module.source, module.text, &[]);
            if !parsed.is_valid() {
                return Err(ProjectCheck::InvalidProof);
            }
            let ScalarProofDeclarations { declarations: module_declarations, definitions } =
                derive_scalar_proof_declarations_from_cst(parsed.canonical_cst());
            if !definitions.is_empty() {
                proofs.push(ModuleProofCatalog { source: module_declarations.source().clone(), definitions });
            }
            declarations.push((module_declarations, module.namespaces));
        }
        Self::prepare_declarations(source, declarations, proofs, initialization_order, inputs, Vec::new())
    }

    fn prepare_declarations(
        source: SourceIdentity,
        mut declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        catalogs: Vec<ModuleProofCatalog>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
        dormant: Vec<ScalarProofDeclarations>,
    ) -> Result<Self, ProjectCheck> {
        let mut graph_sources: HashMap<_, _> = initialization_order.iter().map(|source| (source, false)).collect();
        if graph_sources.len() != initialization_order.len() {
            return Err(ProjectCheck::AmbiguousRoot);
        }
        if !graph_sources.contains_key(&source) {
            return Err(ProjectCheck::MissingRoot);
        }
        for (module, namespaces) in &declarations {
            let seen = graph_sources.get_mut(module.source()).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
            if std::mem::replace(seen, true) {
                return Err(ProjectCheck::AmbiguousRoot);
            }
            if namespaces.iter().any(|edge| !graph_sources.contains_key(&edge.target)) {
                return Err(ProjectCheck::RootSourceOutsideInitOrder);
            }
        }
        if graph_sources.values().any(|seen| !seen) {
            return Err(ProjectCheck::MissingRoot);
        }
        if !dormant.is_empty() {
            validate_dormant_source_boundaries(
                &declarations, &source, |source| graph_sources.contains_key(source),
                dormant.iter().map(|proof| &proof.declarations),
            )?;
            declarations.reserve(dormant.len());
        }
        let mut proofs = HashMap::new();
        for catalog in catalogs {
            if !graph_sources.contains_key(&catalog.source) {
                return Err(ProjectCheck::RootSourceOutsideInitOrder);
            }
            if proofs.insert(catalog.source, ModuleProofs { definitions: catalog.definitions, first_query: 0 }).is_some() {
                return Err(ProjectCheck::AmbiguousRoot);
            }
        }
        for ScalarProofDeclarations { declarations: source, definitions } in dormant {
            if !definitions.is_empty() {
                proofs.insert(source.source().clone(), ModuleProofs { definitions, first_query: 0 });
            }
            declarations.push((source, Vec::new()));
        }
        let mut targets = Vec::new();
        for (source, proofs) in &mut proofs {
            proofs.first_query = targets.len();
            for definition in &proofs.definitions {
                let (receiver, name) = proof_target_tokens(definition)?;
                targets.push((source, receiver, name));
            }
        }
        let queries = targets.iter().map(|(source, receiver, name)| CheckerTargetQuery {
            source,
            receiver: receiver.as_ref().map(CstToken::text),
            name: name.text(),
        }).collect::<Vec<_>>();
        let prepared = prepared::prepare_checker_project_with_queries(declarations, initialization_order, inputs, &queries)?;
        Ok(Self { source, proofs, prepared })
    }

    fn select(&self, source: &SourceIdentity, proof_name: &str) -> Result<(&CstNode, usize), ProjectCheck> {
        let proofs = self.proofs.get(source).ok_or(ProjectCheck::MissingRoot)?;
        let mut selected = proofs.definitions.iter().enumerate().filter(|(_, proof)| {
            module_proof_name(proof).is_some_and(|token| token.text() == proof_name)
        });
        let (index, proof) = selected.next().ok_or(ProjectCheck::MissingRoot)?;
        if selected.next().is_some() {
            return Err(ProjectCheck::AmbiguousRoot);
        }
        Ok((proof, proofs.first_query + index))
    }

    #[cfg(test)]
    fn check(&self, proof_name: &str) -> Result<(ByteSpan, ProjectCheck), ProjectCheck> {
        self.check_at(&self.source, proof_name)
    }

    #[cfg(test)]
    fn check_at(&self, source: &SourceIdentity, proof_name: &str) -> Result<(ByteSpan, ProjectCheck), ProjectCheck> {
        let proof = self.select_resolved(source, proof_name)?;
        let (outcome, _) = check_selected_proof(&self.prepared, proof.function, proof.node, source, None)?;
        Ok((byte_span(proof.node), outcome))
    }

    #[cfg(test)]
    fn type_where(&self, proof_name: &str) -> Result<(), ProjectCheck> {
        let proof = self.select_resolved(&self.source, proof_name)?;
        let (module, item, _) = proof.function;
        let (project, calls) = self.prepared.parts();
        let body = calls.function_body(module, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let target = child(proof.node, SyntaxKind::ProofTarget)?;
        let inputs = identifiers(&child(&target, SyntaxKind::Parameters)?).collect::<Vec<_>>();
        let layout = project.modules[module].target_layout;
        if inputs.len() != body.boundary.parameters.len()
            || inputs.iter().collect::<HashSet<_>>().len() != inputs.len()
        {
            return Err(ProjectCheck::InvalidProof);
        }
        let _ = typed_proof_where(proof.node, &self.source, layout, &inputs, body)?;
        Ok(())
    }

    #[cfg(test)]
    fn verify(&self, proof_name: &str) -> Option<VerifiedModuleReturnBound<'_>> {
        let proof = self.select_resolved(&self.source, proof_name).ok()?;
        let (_, verified) = check_selected_proof(
            &self.prepared,
            proof.function,
            proof.node,
            &self.source,
            Some(ProofSummaryOrigin::ModuleDeclaration {
                source: &self.source,
                target: proof.target,
            }),
        ).ok()?;
        Some(VerifiedModuleReturnBound { issuer: self, proof: proof.node, bound: verified? })
    }

    fn select_resolved(
        &self,
        proof_source: &SourceIdentity,
        proof_name: &str,
    ) -> Result<SelectedModuleProof<'_>, ProjectCheck> {
        let (node, query) = self.select(proof_source, proof_name)?;
        self.resolve_selected_proof(node, query)
    }

    fn resolve_selected_proof<'a>(
        &'a self, node: &'a CstNode, query: usize,
    ) -> Result<SelectedModuleProof<'a>, ProjectCheck> {
        let (project, calls) = self.prepared.parts();
        let ResolvedCheckerTarget { target, declaration: (module, item) } =
            calls.checker_targets.get(query).and_then(Option::as_ref).ok_or(ProjectCheck::MissingRoot)?;
        let owner = project.modules.get(*module).ok_or(ProjectCheck::MissingRoot)?;
        let Some(ScalarItem::Function(function)) = owner.items.get(*item) else {
            return Err(ProjectCheck::MissingRoot);
        };
        let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration } = target
        else {
            return Err(ProjectCheck::Unsupported);
        };
        if source != &owner.source || *declaration_span != function.name_span {
            return Err(ProjectCheck::MissingRoot);
        }
        Ok(SelectedModuleProof { node, function: (*module, *item, function), target })
    }

    #[cfg(test)]
    fn apply(
        &self,
        caller_name: &str,
        proof_name: &str,
        verified: &VerifiedModuleReturnBound<'_>,
    ) -> Option<ImportedRequirement> {
        let bound = &verified.bound;
        if !std::ptr::eq(self, verified.issuer)
            || bound.proof_venue != ProofVenue::ModuleDeclaration
            || bound.proof_source != self.source
            || bound.proof_name != proof_name
            || bound.proof_span != byte_span(verified.proof)
        {
            return None;
        }
        apply_verified_return_bound(&self.prepared, &self.source, caller_name, bound)
    }
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub enum ScalarModuleProofOutcome {
    Proved,
    Refuted,
    NoApplicableInputs,
    Invalid(Vec<Diagnostic>),
    Unsupported,
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_scalar_module_proof(
    proof_source: SourceIdentity,
    proof_name: &str,
    modules: Vec<ScalarProofModuleInput>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> ScalarModuleProofOutcome {
    match check_scalar_module_proofs(proof_source, &[proof_name], modules, initialization_order, inputs) {
        Ok(mut outcomes) => outcomes.pop().unwrap_or(ScalarModuleProofOutcome::Unsupported),
        Err(outcome) => outcome,
    }
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_scalar_module_proofs(
    proof_source: SourceIdentity,
    proof_names: &[&str],
    modules: Vec<ScalarProofModuleInput>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> Result<Vec<ScalarModuleProofOutcome>, ScalarModuleProofOutcome> {
    if proof_names.is_empty() {
        return Ok(Vec::new());
    }
    let prepared = PreparedProofModule::prepare_project(proof_source, modules, initialization_order, inputs)
        .map_err(|error| module_proof_outcome(Err(error)))?;
    Ok(prepared.check_module_proofs(proof_names))
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_loaded_scalar_module_proofs(
    proof_names: &[&str],
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proof: (ScalarProofDeclarations, Vec<ScalarNamespaceBinding>),
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> Result<Vec<ScalarModuleProofOutcome>, ScalarModuleProofOutcome> {
    if proof_names.is_empty() {
        return Ok(Vec::new());
    }
    let source = proof.0.declarations.source().clone();
    let selections = proof_names.iter().map(|&name| (&source, name)).collect::<Vec<_>>();
    check_loaded_scalar_module_proof_selections(&selections, declarations, vec![proof], initialization_order, inputs)
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_loaded_scalar_module_proof_selections(
    selections: &[(&SourceIdentity, &str)],
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proofs: Vec<(ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> Result<Vec<ScalarModuleProofOutcome>, ScalarModuleProofOutcome> {
    if selections.is_empty() {
        return Ok(Vec::new());
    }
    let prepared = prepare_loaded_proof_project(declarations, proofs, initialization_order, inputs, Vec::new())
        .map_err(|error| module_proof_outcome(Err(error)))?;
    Ok(prepared.check_module_proof_selections(selections))
}

#[cfg(feature = "internal-checker")]
fn prepare_loaded_proof_project(
    mut declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proofs: Vec<(ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    dormant: Vec<ScalarProofDeclarations>,
) -> Result<PreparedProofModule, ProjectCheck> {
    let mut catalogs = Vec::with_capacity(proofs.len());
    declarations.reserve(proofs.len());
    for (proof, namespaces) in proofs {
        let ScalarProofDeclarations { declarations: proof_declarations, definitions } = proof;
        catalogs.push(ModuleProofCatalog { source: proof_declarations.source().clone(), definitions });
        declarations.push((proof_declarations, namespaces));
    }
    let root = initialization_order.first().cloned().ok_or(ProjectCheck::MissingRoot)?;
    PreparedProofModule::prepare_declarations(root, declarations, catalogs, initialization_order, inputs, dormant)
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[doc(hidden)]
pub struct MainFlowProofReceipt {
    pub active: bool,
    pub directives: Vec<ScalarOrderedCheckerFailure>,
    pub diagnostics: Vec<MainFlowProofDiagnostic>,
    pub symbolic_diagnostic: Option<SymbolicProofDiagnostic>,
    pub additional_symbolic_diagnostics: Vec<SymbolicProofDiagnostic>,
    pub entry_evidence: Option<Box<ScalarOrderedEntryEvidence>>,
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[doc(hidden)]
pub struct MainFlowProofDiagnostic {
    pub proof: wosy_syntax::SourceSpan,
    pub goal: wosy_syntax::SourceSpan,
    pub failed_step: Option<wosy_syntax::SourceSpan>,
    pub invocation: Rc<MainFlowProofInvocation>,
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[doc(hidden)]
pub struct MainFlowProofInvocation {
    pub outer_call: Option<wosy_syntax::SourceSpan>,
    pub call: wosy_syntax::SourceSpan,
    pub directive_position: usize,
    pub arguments: Vec<(ScalarType, BigInt)>,
    pub returned: BigInt,
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
#[doc(hidden)]
pub struct SymbolicProofDiagnostic {
    pub proof: wosy_syntax::SourceSpan,
    pub goal: wosy_syntax::SourceSpan,
    pub failed_step: Option<wosy_syntax::SourceSpan>,
    pub callable: wosy_syntax::SourceSpan,
    pub output: wosy_syntax::SourceSpan,
    pub directive_position: usize,
    pub witness: ScalarCheckerIntegerWitness,
    pub returned: BigInt,
}

#[cfg(feature = "internal-checker")]
struct SelectedSymbolicProof<'a, 'case> {
    watched: &'case MainFlowProofObserver<'a>,
    equalities: SymbolicEqualityIndex<'a>,
    failure: Option<SymbolicProofDiagnostic>,
    additional_failures: Vec<SymbolicProofDiagnostic>,
    snapshot_work: usize,
}

#[cfg(feature = "internal-checker")]
struct VerifiedSymbolicEquality<'a> {
    target: PreparedCallable,
    canonical_target: &'a ResolvedCallTarget,
    layout: ScalarTargetLayout,
    body: &'a FunctionCallSites,
    bound: &'a BigInt,
    shared: std::cell::OnceCell<Rc<IntegerSet>>,
    guards: VerifiedSymbolicGuards<'a>,
}

#[cfg(feature = "internal-checker")]
enum VerifiedSymbolicGuards<'a> {
    Bare(Option<ImportedEntryGuard<'a>>),
    Ranked(CheckedIntegerGuardSlots<'a>),
}

#[cfg(feature = "internal-checker")]
impl<'a> VerifiedSymbolicEquality<'a> {
    fn consume<'scope>(
        &self, find_additional: impl Fn(&PreparedCallable) -> Option<&'scope Self>,
        target: PreparedCallable, calls: &FunctionCallSites, source: &SourceIdentity, effects: &mut OrderedFailures,
    ) -> Result<Option<bool>, ProjectCheck>
    where 'a: 'scope,
    {
        let first = self;
        if target.0 != first.target.0 || target.1 <= first.target.1
            || !calls.assignments.is_empty() || !calls.allocation_length_calls.is_empty()
            || !matches!(calls.boundary.results.as_slice(), [result] if result.ty == ScalarType::Unit)
        {
            return Ok(None);
        }
        let occurrences = calls.call_occurrences.as_slice();
        let locals = &calls.structure.checker_integer_call_locals;
        if occurrences.is_empty() || locals.len() != occurrences.len()
            || calls.structure.checker_directives.is_empty()
        {
            return Ok(None);
        }
        let import = |&(block, position), origin: &CheckerIntegerCallOrigin, occurrence: &super::reference::CallOccurrence| {
            if block != calls.structure.root {
                return Ok(None);
            }
            let Some(callee) = calls.structure.checker_direct_targets.get(&origin.call) else { return Ok(None); };
            let summary = if *callee == first.target {
                first
            } else if let Some(summary) = find_additional(callee) {
                summary
            } else {
                return Ok(None);
            };
            if target.0 != summary.target.0 || target.1 <= summary.target.1 {
                return Ok(None);
            }
            let site = calls.sites.get(occurrence.site).ok_or(ProjectCheck::MissingRootOccurrences)?;
            let signature = calls.checked_call_signatures.get(&origin.call).ok_or(ProjectCheck::MissingRootOccurrences)?;
            let [output] = summary.body.boundary.results.as_slice() else { return Err(ProjectCheck::Unsupported); };
            if output.ordinal != 0 || !production_bound_reuse::matching_call(
                summary.canonical_target, summary.layout, &summary.body.boundary.parameters, &output.ty,
                summary.layout, calls, origin, &site.target, signature,
            ) {
                return Ok(None);
            }
            let (guards, domain) = match &summary.guards {
                VerifiedSymbolicGuards::Bare(guard) => (guard.as_slice(), None),
                VerifiedSymbolicGuards::Ranked(guards) => (guards.comparisons.as_slice(), guards.domain),
            };
            let values = ImportedIntegerValues::Shared { values: &summary.shared, bound: summary.bound, guards, domain };
            Ok(Some(ImportedLocal { position, values, guards: Vec::new() }))
        };
        let mut emit = |failure| effects.push_callable(calls, source, failure);
        if occurrences.len() > 2 {
            let mut imports = Vec::with_capacity(occurrences.len());
            imports.resize_with(occurrences.len(), || None);
            for (position, origin) in locals {
                let Ok(index) = occurrences.binary_search_by_key(&origin.call.0, |site| site.ordinal.0) else {
                    return Ok(None);
                };
                if imports[index].is_some() {
                    return Ok(None);
                }
                let Some(local) = import(position, origin, &occurrences[index])? else { return Ok(None); };
                imports[index] = Some(local);
            }
            debug_assert!(imports.windows(2).all(|adjacent|
                adjacent[0].as_ref().map(|local| local.position) < adjacent[1].as_ref().map(|local| local.position)
            ));
            let active = super::checker_flow::collect_ordered_imported_root_locals(calls, &mut imports, &mut emit)
                .ok_or(ProjectCheck::Unsupported)?;
            return Ok(Some(active));
        }
        let mut locals = locals.iter();
        let Some((position, first_origin)) = locals.next() else { return Ok(None); };
        let Some(first_site) = occurrences.iter().find(|site| site.ordinal == first_origin.call) else { return Ok(None); };
        let Some(first_import) = import(position, first_origin, first_site)? else { return Ok(None); };
        let active = if let Some((position, second_origin)) = locals.next() {
            if first_origin.call == second_origin.call {
                return Ok(None);
            }
            let Some(second_site) = occurrences.iter().find(|site| site.ordinal == second_origin.call) else { return Ok(None); };
            let Some(second_import) = import(position, second_origin, second_site)? else { return Ok(None); };
            let mut imports = if first_import.position < second_import.position {
                [Some(first_import), Some(second_import)]
            } else {
                [Some(second_import), Some(first_import)]
            };
            super::checker_flow::collect_ordered_imported_root_locals(calls, &mut imports, &mut emit)
        } else {
            super::checker_flow::collect_ordered_imported_root_locals(calls, &mut [Some(first_import)], &mut emit)
        }.ok_or(ProjectCheck::Unsupported)?;
        Ok(Some(active))
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct SymbolicEqualityIndex<'a> {
    first: Option<VerifiedSymbolicEquality<'a>>,
    other: HashMap<PreparedCallable, VerifiedSymbolicEquality<'a>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> SymbolicEqualityIndex<'a> {
    fn insert(&mut self, summary: VerifiedSymbolicEquality<'a>) {
        if self.first.is_none() {
            self.first = Some(summary);
        } else {
            self.other.insert(summary.target, summary);
        }
    }

    fn consume(
        &self, target: PreparedCallable, calls: &FunctionCallSites, source: &SourceIdentity, effects: &mut OrderedFailures,
    ) -> Result<Option<bool>, ProjectCheck> {
        let Some(first) = &self.first else { return Ok(None); };
        first.consume(|callee| self.other.get(callee), target, calls, source, effects)
    }
}

#[cfg(feature = "internal-checker")]
impl SelectedSymbolicProof<'_, '_> {
    fn record(
        &mut self, proof: &MainFlowProofCase<'_>, function: &ScalarFunction, source: &SourceIdentity,
        position: usize, failure: super::checker_flow::SymbolicLiteralFailure<'_>, failed_step: Option<ByteSpan>,
    ) -> Result<(), ProjectCheck> {
        let proof_span = |range| wosy_syntax::SourceSpan { source: proof.proof_source.clone(), range };
        let callable_span = |range| wosy_syntax::SourceSpan { source: source.clone(), range };
        let mut hidden = false;
        let mut diagnostic = SymbolicProofDiagnostic {
            proof: proof_span(proof.proof), goal: proof_span(proof.goal), failed_step: failed_step.map(proof_span),
            callable: callable_span(function.name_span), output: callable_span(failure.output),
            directive_position: position,
            witness: ScalarCheckerIntegerWitness {
                equal_axes: Vec::new(),
                axes: failure.axes.into_iter().map(|(binding, ty)| {
                    hidden |= matches!(binding.kind, super::ReferenceBindingKind::CheckerSnapshotField(_));
                    (binding.clone(), ty.clone())
                }).collect(),
                state: failure.state,
            },
            returned: failure.returned,
        };
        if hidden { diagnostic.witness.remove_snapshot_axes(&mut self.snapshot_work).ok_or(ProjectCheck::Unsupported)?; }
        if self.failure.is_none() {
            self.failure = Some(diagnostic);
        } else {
            self.additional_failures.push(diagnostic);
        }
        Ok(())
    }
}

#[cfg(feature = "internal-checker")]
impl SelectedSymbolicProof<'_, '_> {
    fn verify_calling_producer(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, effects: &mut OrderedFailures, runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        let watched = self.watched;
        let group = if watched.first.0 == target { &watched.first.1 } else {
            watched.other.get(&target).ok_or(ProjectCheck::Unsupported)?
        };
        let MainFlowProofCases::Bare { first, rest } = &group.cases else { return Err(ProjectCheck::Unsupported); };
        let [parameter] = body.boundary.parameters.as_slice() else { return Err(ProjectCheck::Unsupported); };
        let [output] = body.boundary.results.as_slice() else { return Err(ProjectCheck::Unsupported); };
        use super::checker_flow::PartitionedProofGoal;
        let issuer = group.issuer.as_ref().filter(|issuer| std::ptr::eq(issuer.body, body));
        let goal = match first.bound {
            MainFlowProofBound::Literal(bound) => PartitionedProofGoal::Literal(first.relation, bound),
            MainFlowProofBound::Entry(0) if parameter.ty == output.ty
                && Some(first.relation) == IntegerRelation::from_operator(&BinaryOperator::Equal) =>
                    PartitionedProofGoal::EntryEquality,
            _ => return Err(ProjectCheck::Unsupported),
        };
        let literal_bound = first.bound.literal();
        if first.proof_source != source || (literal_bound.is_some() && issuer.is_none() && !first.guard.always_rejects)
            || !rest.is_empty() || !supported_calling_proof_body(body, &output.ty)
            || first.guard.domain.is_some() || first.guard.comparisons.len() > 1
        {
            return Err(ProjectCheck::Unsupported);
        }
        let domain = IntegerSet::from_type(&parameter.ty).ok_or(ProjectCheck::Unsupported)?;
        let proof_domain = if first.guard.always_rejects {
            None
        } else {
            Some(match first.guard.comparisons.as_slice() {
                [] => domain.clone(),
                [comparison] if comparison.slot == 0 => domain.keep_relation(comparison.relation, comparison.bound),
                _ => return Err(ProjectCheck::Unsupported),
            })
        };
        let cursor = ExactCallCursor::projected(body, Rc::new(domain), &output.ty).ok_or(ProjectCheck::Unsupported)?;
        let mut evaluator = CheckedCalls::<ProofFreeSymbolic, false> {
            project: runtime.project, calls: runtime.calls, active: HashSet::from([target]),
            active_depth: 1, reached: HashSet::new(), had_obligations: false,
            observer: ProofFreeSymbolic { ordinary_projected_effects: false }, symbolic_module: target.0,
        };
        let first_directive = effects.records.len();
        let (result, active) = {
            let sink = RefCell::new(&mut *effects);
            let collection = OrderedCallCollection { failures: &sink, visits: runtime.visits, outer: None };
            let result = evaluator.run_call_cursor(body, source, cursor, None, Some(collection))?;
            (result, evaluator.had_obligations)
        };
        let Some(proof_domain) = proof_domain else { return Ok(Some(active)); };
        let elidable = literal_bound.is_some() && effects.records.len() == first_directive
            && summary_elision_safe(body)
            && evaluator.reached.iter().all(|&(module, item)| {
                runtime.calls.function_body(module, item, None).is_some_and(summary_elision_safe)
            });
        let verified = super::checker_flow::verify_partitioned_call_return(
            body, &result, &proof_domain, elidable, goal,
        ).ok_or(ProjectCheck::Unsupported)?;
        if let Some(failure) = verified.failure {
            self.record(first, function, source, effects.records.len(), failure, None)?;
        } else if verified.covers_guard {
            let bound = literal_bound.ok_or(ProjectCheck::Unsupported)?;
            let issuer = issuer.ok_or(ProjectCheck::Unsupported)?;
            self.equalities.insert(VerifiedSymbolicEquality {
                target, canonical_target: issuer.canonical_target, layout: issuer.layout,
                body: issuer.body, bound, shared: std::cell::OnceCell::new(),
                guards: VerifiedSymbolicGuards::Bare(first.guard.comparisons.first().map(|guard| ImportedEntryGuard {
                    slot: guard.slot, relation: guard.relation, bound: guard.bound,
                })),
            });
        }
        Ok(Some(active || verified.applicable))
    }
}

#[cfg(feature = "internal-checker")]
impl SymbolicRootProof for SelectedSymbolicProof<'_, '_> {
    const ENABLED: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        for &(module, item) in std::iter::once(&self.watched.first.0).chain(self.watched.other.keys()) {
            if !reached.contains(&(module, item)) {
                roots.entry(module).or_default().insert(item);
            }
        }
    }

    fn try_collect(
        &mut self, target: PreparedCallable, calls: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, effects: &mut OrderedFailures,
        runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        let watched = self.watched;
        let group = if watched.first.0 == target {
            &watched.first.1
        } else if let Some(group) = watched.other.get(&target) {
            group
        } else {
            return self.equalities.consume(target, calls, source, effects);
        };
        if !calls.call_occurrences.is_empty() || !calls.structure.checker_direct_targets.is_empty() {
            return self.verify_calling_producer(target, calls, function, source, effects, runtime);
        }
        let tautological = |proof: &MainFlowProofCase<'_>| !proof.guard.always_rejects
            && proof.guard.comparisons.is_empty() && proof.guard.domain.is_none();
        if !calls.allocation_length_calls.is_empty() || !calls.assignments.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        let (first, rest) = match &group.cases {
            MainFlowProofCases::Bare { first, rest } => (first, rest),
            MainFlowProofCases::Authored(cases) => {
                let [(proof, Some(shows))] = cases.as_slice() else {
                    return Err(ProjectCheck::Unsupported);
                };
                if !tautological(proof) {
                    return Err(ProjectCheck::Unsupported);
                }
                let proof_bound = proof.bound.literal().ok_or(ProjectCheck::Unsupported)?;
                let MainFlowAuthoredShows::Single(step) = shows else {
                    let MainFlowAuthoredShows::Multiple { first_span, groups } = shows else {
                        return Err(ProjectCheck::Unsupported);
                    };
                    let mut failed_step: Option<ByteSpan> = None;
                    let mut failed_goal = false;
                    let mut equalities = None;
                    let mut select_witness = |lo: &BigInt, hi: &BigInt, domain: Option<&IntegerSet>| {
                        use super::checker_flow::SymbolicProofPoint::{Bound, Lower, Upper};
                        if failed_step.is_some_and(|prior| prior.start == first_span.start) {
                            return None;
                        }
                        let failed = if lo == hi {
                            first_failed_show(groups, lo).map(|span| (span, Lower))
                        } else {
                            let left = groups.partition_point(|group| group.bound.as_ref() <= lo);
                            let right = groups.partition_point(|group| group.bound.as_ref() < hi);
                            let mut candidate = groups.get(left).and_then(|group| group.failures[0]).map(|span| (span, Lower));
                            if let Some(span) = right.checked_sub(1).and_then(|index| groups[index].failures[2]) {
                                if candidate.as_ref().is_none_or(|(prior, _)| span.start < prior.start) {
                                    candidate = Some((span, Upper));
                                }
                            }
                            if candidate.as_ref().is_none_or(|(span, _)| span.start != first_span.start) {
                                let domain = domain?;
                                let equalities = equalities.get_or_insert_with(|| ShowEqualityIndex::from_groups(groups));
                                if let Some(index) = equalities.first_failure(groups, domain) {
                                    let group = &groups[index];
                                    let span = group.failures[1]?;
                                    if candidate.as_ref().is_none_or(|(prior, _)| span.start < prior.start) {
                                        candidate = Some((span, Bound(group.bound.as_ref())));
                                    }
                                }
                            }
                            candidate
                        };
                        if let Some((candidate, point)) = failed {
                            if failed_step.is_none_or(|prior| candidate.start < prior.start) {
                                failed_step = Some(candidate);
                                return Some(point);
                            }
                        } else if failed_step.is_none() && !failed_goal {
                            let rejected = proof.relation.complement();
                            let point = if rejected.accepts(lo.cmp(proof_bound)) {
                                Some(Lower)
                            } else if lo != hi && rejected.accepts(hi.cmp(proof_bound)) {
                                Some(Upper)
                            } else if rejected.accepts(std::cmp::Ordering::Equal) && lo < proof_bound && proof_bound < hi
                                && domain.is_some_and(|domain| domain.covers_closed(proof_bound, proof_bound))
                            {
                                Some(Bound(proof_bound))
                            } else {
                                None
                            };
                            failed_goal = point.is_some();
                            return point;
                        }
                        None
                    };
                    let (active, directives, returned, failure) = super::checker_flow::collect_ordered_root_multiple_proof(
                        calls, &function.body, &mut select_witness,
                    ).ok_or(ProjectCheck::Unsupported)?;
                    for directive in directives {
                        effects.push_callable(calls, source, directive);
                    }
                    if let Some(failure) = failure {
                        self.record(proof, function, source, effects.records.len(), failure, failed_step)?;
                    }
                    return Ok(Some(active || returned));
                };
                let mut checks = [
                    super::checker_flow::SymbolicLiteralCheck {
                        relation: step.relation, bound: &step.bound, failure: None,
                    },
                    super::checker_flow::SymbolicLiteralCheck {
                        relation: proof.relation, bound: proof_bound, failure: None,
                    },
                ];
                let count = if step.same_bound && step.relation == proof.relation { 1 } else { 2 };
                let (active, directives, returned) = super::checker_flow::collect_ordered_root_literal_proofs::<true>(
                    calls, &function.body, &mut checks[..count],
                ).ok_or(ProjectCheck::Unsupported)?;
                for directive in directives {
                    effects.push_callable(calls, source, directive);
                }
                let [step_check, goal_check] = checks;
                let position = effects.records.len();
                if let Some(failure) = step_check.failure {
                    self.record(proof, function, source, position, failure, Some(step.span))?;
                } else if let Some(failure) = goal_check.failure {
                    self.record(proof, function, source, position, failure, None)?;
                }
                return Ok(Some(active || returned));
            }
        };
        let predicate = match first.filter {
            [CheckerPredicate::Integer(predicate)] if rest.is_empty() => Some(predicate),
            _ if tautological(first) && rest.iter().all(tautological) => None,
            _ => return Err(ProjectCheck::Unsupported),
        };
        if rest.is_empty() {
            if let MainFlowProofBound::Entry(slot) = first.bound {
                if Some(first.relation) != IntegerRelation::from_operator(&BinaryOperator::Equal) {
                    return Err(ProjectCheck::Unsupported);
                }
                let (active, directives, returned, failure) = super::checker_flow::collect_ordered_root_entry_equality(
                    calls, &function.body, slot, predicate,
                ).ok_or(ProjectCheck::Unsupported)?;
                for directive in directives {
                    effects.push_callable(calls, source, directive);
                }
                if let Some(failure) = failure {
                    self.record(first, function, source, effects.records.len(), failure, None)?;
                }
                return Ok(Some(active || returned));
            }
            let first_bound = first.bound.literal().ok_or(ProjectCheck::Unsupported)?;
            let candidate = group.issuer.as_ref().filter(|_| first.proof_source == source);
            let (active, directives, returned, failure) = if candidate.is_some() {
                super::checker_flow::collect_ordered_root_literal_proof::<true>(
                    calls, &function.body, first.relation, first_bound, None, Some(first.filter),
                )
            } else {
                super::checker_flow::collect_ordered_root_literal_proof::<false>(
                    calls, &function.body, first.relation, first_bound, predicate, None,
                )
            }.ok_or(ProjectCheck::Unsupported)?;
            if returned && failure.is_none() {
                if let Some(plan) = candidate {
                    let summary = VerifiedSymbolicEquality {
                        target, canonical_target: plan.canonical_target, layout: plan.layout,
                        body: plan.body, bound: first_bound, shared: std::cell::OnceCell::new(),
                        guards: VerifiedSymbolicGuards::Bare(first.guard.comparisons.first().map(|guard| ImportedEntryGuard {
                            slot: guard.slot, relation: guard.relation, bound: guard.bound,
                        })),
                    };
                    self.equalities.insert(summary);
                }
            }
            for directive in directives {
                effects.push_callable(calls, source, directive);
            }
            if let Some(failure) = failure {
                self.record(first, function, source, effects.records.len(), failure, None)?;
            }
            return Ok(Some(active || returned));
        }
        let proofs = std::iter::once(first).chain(rest);
        let mut checks = proofs.clone().map(|proof| {
            let bound = proof.bound.literal().ok_or(ProjectCheck::Unsupported)?;
            Ok(super::checker_flow::SymbolicLiteralCheck { relation: proof.relation, bound, failure: None })
        }).collect::<Result<Vec<_>, ProjectCheck>>()?;
        let (active, directives, returned) = super::checker_flow::collect_ordered_root_literal_proofs::<false>(
            calls, &function.body, &mut checks,
        ).ok_or(ProjectCheck::Unsupported)?;
        for directive in directives {
            effects.push_callable(calls, source, directive);
        }
        let position = effects.records.len();
        for (proof, check) in proofs.zip(checks) {
            if let Some(failure) = check.failure {
                self.record(proof, function, source, position, failure, None)?;
            }
        }
        Ok(Some(active || returned))
    }
}

#[cfg(feature = "internal-checker")]
struct MainFlowLiteralShow<'a> {
    span: ByteSpan,
    relation: IntegerRelation,
    bound: Cow<'a, BigInt>,
    same_bound: bool,
}

#[cfg(feature = "internal-checker")]
enum MainFlowAuthoredShows<'a> {
    Single(MainFlowLiteralShow<'a>),
    Entry { span: ByteSpan, slot: usize, relation: IntegerRelation },
    Multiple { first_span: ByteSpan, groups: Vec<MainFlowShowBound<'a>> },
}

#[cfg(feature = "internal-checker")]
struct MainFlowShowBound<'a> {
    bound: Cow<'a, BigInt>,
    failures: [Option<ByteSpan>; 3],
}

#[cfg(feature = "internal-checker")]
fn earliest_show_failure(first: Option<ByteSpan>, second: Option<ByteSpan>) -> Option<ByteSpan> {
    match (first, second) {
        (Some(first), Some(second)) if second.start < first.start => Some(second),
        (first, second) => first.or(second),
    }
}

#[cfg(feature = "internal-checker")]
fn first_failed_show(groups: &[MainFlowShowBound<'_>], value: &BigInt) -> Option<ByteSpan> {
    let (below, equal, above) = match groups.binary_search_by(|group| group.bound.as_ref().cmp(value)) {
        Ok(index) => (index, groups[index].failures[1], index + 1),
        Err(index) => (index, None, index),
    };
    let lower_failure = below.checked_sub(1).and_then(|index| groups[index].failures[2]);
    let upper_failure = groups.get(above).and_then(|group| group.failures[0]);
    earliest_show_failure(equal, earliest_show_failure(lower_failure, upper_failure))
}

#[cfg(feature = "internal-checker")]
enum ShowEqualityIndex {
    Empty,
    Single(usize),
    Tree { count: usize, indices: Vec<usize> },
}

#[cfg(feature = "internal-checker")]
impl ShowEqualityIndex {
    const EMPTY: usize = usize::MAX;

    fn earliest(groups: &[MainFlowShowBound<'_>], a: usize, b: usize) -> usize {
        let key = |index| groups.get(index).and_then(|group: &MainFlowShowBound<'_>|
            group.failures[1].map(|span| (span.start, index)));
        match (key(a), key(b)) {
            (Some(a_key), Some(b_key)) if b_key < a_key => b,
            (Some(_), _) => a,
            _ => b,
        }
    }

    fn from_groups(groups: &[MainFlowShowBound<'_>]) -> Self {
        let mut equalities = groups.iter().enumerate().filter_map(|(index, group)| group.failures[1].map(|_| index));
        let Some(first) = equalities.next() else { return Self::Empty; };
        let Some(second) = equalities.next() else { return Self::Single(first); };
        let mut indices = Vec::with_capacity(4);
        indices.extend([first, second]);
        indices.extend(equalities);
        let count = indices.len();
        indices.resize(count * 2, Self::EMPTY);
        indices.copy_within(..count, count);
        for node in (1..count).rev() {
            indices[node] = Self::earliest(groups, indices[2 * node], indices[2 * node + 1]);
        }
        Self::Tree { count, indices }
    }

    fn first_failure(&self, groups: &[MainFlowShowBound<'_>], domain: &IntegerSet) -> Option<usize> {
        match self {
            Self::Empty => None,
            Self::Single(index) => {
                let bound = groups[*index].bound.as_ref();
                domain.covers_closed(bound, bound).then_some(*index)
            }
            Self::Tree { count, indices } => {
                let leaves = &indices[*count..];
                let mut best = Self::EMPTY;
                for (lo, hi) in domain.intervals() {
                    let mut start = *count + leaves.partition_point(|&index| groups[index].bound.as_ref() < lo);
                    let mut end = *count + leaves.partition_point(|&index| groups[index].bound.as_ref() <= hi);
                    while start < end {
                        if start & 1 == 1 {
                            best = Self::earliest(groups, best, indices[start]);
                            start += 1;
                        }
                        if end & 1 == 1 {
                            end -= 1;
                            best = Self::earliest(groups, best, indices[end]);
                        }
                        start /= 2;
                        end /= 2;
                    }
                }
                (best != Self::EMPTY).then_some(best)
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
enum MainFlowProofBound<'a> {
    Literal(&'a BigInt),
    Entry(usize),
}

#[cfg(feature = "internal-checker")]
impl<'a> MainFlowProofBound<'a> {
    fn literal(&self) -> Option<&'a BigInt> {
        match self {
            Self::Literal(value) => Some(value),
            Self::Entry(_) => None,
        }
    }

    fn for_actual<'call>(&self, arguments: &[&'call BigInt]) -> Option<&'call BigInt>
    where 'a: 'call,
    {
        match self {
            Self::Literal(value) => Some(value),
            Self::Entry(slot) => arguments.get(*slot).copied(),
        }
    }
}

#[cfg(feature = "internal-checker")]
struct MainFlowProofCase<'a> {
    proof_source: &'a SourceIdentity,
    proof: ByteSpan,
    goal: ByteSpan,
    guard: CheckedIntegerGuardSlots<'a>,
    filter: &'a [CheckerPredicate],
    relation: IntegerRelation,
    bound: MainFlowProofBound<'a>,
}

#[cfg(feature = "internal-checker")]
impl<'a> MainFlowProofCase<'a> {
    fn prepare(plan: &'a PlannedModuleProof<'a>, root: &SourceIdentity) -> Result<MainFlowProofPlan<'a>, ProjectCheck> {
        let (relation, bound) = match &plan.typed.goal {
            TypedProofGoal::Literal { relation, bound, .. } => (relation, MainFlowProofBound::Literal(bound)),
            TypedProofGoal::Universal { relation, bound } if plan.steps.items.is_empty() => (relation, MainFlowProofBound::Literal(bound)),
            TypedProofGoal::EntryComparison { slot, relation } if plan.steps.items.is_empty() =>
                (relation, MainFlowProofBound::Entry(*slot)),
            _ => return Err(ProjectCheck::Unsupported),
        };
        if &plan.typed.target.source != root {
            return Err(ProjectCheck::Unsupported);
        }
        let show_claim = |step: &'a ScalarBlockItem, terminated: bool| {
            let ScalarBlockItem::Expression(expression) = step else {
                return Err(ProjectCheck::Unsupported);
            };
            if !terminated {
                return Err(ProjectCheck::Unsupported);
            }
            if is_null_literal_name(&plan.typed.output) {
                return Err(ProjectCheck::InvalidProof);
            }
            Ok((super::typecheck::expression_span(expression), proof_show_expression(step, true)?))
        };
        let checked_literal = |claim: &'a ScalarExpression| {
            let comparison = IntegerComparison::from_expression(claim).ok_or(ProjectCheck::Unsupported)?;
            let (_, shown, literal) = checked_literal_output_goal_borrowed(
                comparison, &plan.typed.output, &plan.typed.input_names, plan.typed.result_type,
            )?;
            Ok((shown, literal))
        };
        let step = match (plan.steps.items.as_slice(), plan.steps.terminated_items.as_slice()) {
            ([], []) => None,
            ([step], [true]) => {
                let (span, claim) = show_claim(step, true)?;
                if let Some(checked) = checked_old_output_relation(
                    claim, &plan.typed.output, &plan.typed.input_names, plan.typed.body, plan.typed.result_type,
                ) {
                    let (slot, relation) = checked?;
                    Some(MainFlowAuthoredShows::Entry { span, slot, relation })
                } else {
                    let (relation, literal) = checked_literal(claim)?;
                    let same_bound = bound.literal() == Some(literal.as_ref());
                    Some(MainFlowAuthoredShows::Single(MainFlowLiteralShow { span, relation, bound: literal, same_bound }))
                }
            }
            (steps, terminated) if steps.len() >= 2 && steps.len() == terminated.len() => {
                use std::cmp::Ordering::{Equal, Greater, Less};
                let mut groups = Vec::with_capacity(steps.len());
                let mut first_span = None;
                for (step, &terminated) in steps.iter().zip(terminated) {
                    let (span, claim) = show_claim(step, terminated)?;
                    let (relation, bound) = checked_literal(claim)?;
                    if groups.is_empty() {
                        first_span = Some(span);
                    }
                    let failures = [Less, Equal, Greater].map(|ordering| (!relation.accepts(ordering)).then_some(span));
                    groups.push(MainFlowShowBound { bound, failures });
                }
                groups.sort_unstable_by(|left, right| left.bound.cmp(&right.bound));
                groups.dedup_by(|later, retained| {
                    if retained.bound != later.bound {
                        return false;
                    }
                    for (retained, later) in retained.failures.iter_mut().zip(later.failures) {
                        *retained = earliest_show_failure(*retained, later);
                    }
                    true
                });
                let mut prefix = None;
                for group in &mut groups {
                    prefix = earliest_show_failure(prefix, group.failures[2]);
                    group.failures[2] = prefix;
                }
                let mut suffix = None;
                for group in groups.iter_mut().rev() {
                    suffix = earliest_show_failure(suffix, group.failures[0]);
                    group.failures[0] = suffix;
                }
                Some(MainFlowAuthoredShows::Multiple { first_span: first_span.ok_or(ProjectCheck::Unsupported)?, groups })
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        let filter = plan.filter.as_deref().unwrap_or_default();
        let guard = checked_integer_guard_slots::<false>(plan.typed.body, filter).ok_or(ProjectCheck::Unsupported)?;
        let goal = child(&child(plan.proof, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
        Ok((Self {
            proof_source: plan.proof_source, proof: wosy_syntax::byte_span(plan.proof),
            goal: wosy_syntax::byte_span(&goal), guard, filter, relation: *relation, bound,
        }, step))
    }
}

#[cfg(feature = "internal-checker")]
type MainFlowProofPlan<'a> = (MainFlowProofCase<'a>, Option<MainFlowAuthoredShows<'a>>);

#[cfg(feature = "internal-checker")]
enum MainFlowProofCases<'a> {
    Bare { first: MainFlowProofCase<'a>, rest: Vec<MainFlowProofCase<'a>> },
    Authored(Vec<MainFlowProofPlan<'a>>),
}

#[cfg(feature = "internal-checker")]
impl<'a> MainFlowProofCases<'a> {
    fn push(&mut self, (proof, step): MainFlowProofPlan<'a>) {
        match self {
            Self::Bare { rest, .. } if step.is_none() => rest.push(proof),
            Self::Authored(proofs) => proofs.push((proof, step)),
            Self::Bare { rest, .. } => {
                let authored = Self::Authored(Vec::with_capacity(rest.len() + 2));
                let Self::Bare { first, rest } = std::mem::replace(self, authored) else { unreachable!(); };
                let Self::Authored(proofs) = self else { unreachable!(); };
                proofs.push((first, None));
                proofs.extend(rest.into_iter().map(|prior| (prior, None)));
                proofs.push((proof, step));
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
struct SymbolicEqualityIssuer<'a> {
    canonical_target: &'a ResolvedCallTarget,
    layout: ScalarTargetLayout,
    body: &'a FunctionCallSites,
}

#[cfg(feature = "internal-checker")]
impl<'a> SymbolicEqualityIssuer<'a> {
    fn for_symbolic_case(
        scope: ScalarCheckerScope, plan: &PlannedModuleProof<'a>,
        (proof, authored): &MainFlowProofPlan<'a>,
    ) -> Option<Self> {
        if scope == ScalarCheckerScope::MainFlowOnly || authored.is_some() || proof.guard.always_rejects
            || proof.guard.domain.is_some()
            || !matches!(&plan.typed.goal, TypedProofGoal::Literal { operator: BinaryOperator::Equal, .. })
        {
            return None;
        }
        if !(proof.guard.comparisons.is_empty()
            || proof.guard.comparisons.len() == 1 && matches!(proof.filter, [CheckerPredicate::Integer(_)]))
            || (!valid_arbitrary_proof_body(plan.typed.body)
                && !supported_calling_proof_body(plan.typed.body, plan.typed.result_type))
        {
            return None;
        }
        Some(Self { canonical_target: plan.canonical_target, layout: plan.typed.target.target_layout, body: plan.typed.body })
    }
}

#[cfg(feature = "internal-checker")]
struct MainFlowProofTarget<'a> {
    cases: MainFlowProofCases<'a>,
    issuer: Option<SymbolicEqualityIssuer<'a>>,
    seen: bool,
}

#[cfg(feature = "internal-checker")]
impl<'a> MainFlowProofTarget<'a> {
    fn new((first, step): MainFlowProofPlan<'a>) -> Self {
        let cases = if let Some(step) = step {
            MainFlowProofCases::Authored(vec![(first, Some(step))])
        } else {
            MainFlowProofCases::Bare { first, rest: Vec::new() }
        };
        Self { cases, issuer: None, seen: false }
    }
}

#[cfg(feature = "internal-checker")]
struct MainFlowProofObserver<'a> {
    first: ((usize, usize), MainFlowProofTarget<'a>),
    other: HashMap<(usize, usize), MainFlowProofTarget<'a>>,
    active: bool,
    incomplete: bool,
    diagnostics: Vec<MainFlowProofDiagnostic>,
}

#[cfg(feature = "internal-checker")]
impl<'a> MainFlowProofObserver<'a> {
    fn new(target: (usize, usize), first: MainFlowProofPlan<'a>) -> Self {
        Self {
            first: (target, MainFlowProofTarget::new(first)), other: HashMap::new(),
            active: false, incomplete: false, diagnostics: Vec::new(),
        }
    }

    fn from_first_plan(
        scope: ScalarCheckerScope, root_source: &SourceIdentity, plan: &'a PlannedModuleProof<'a>,
    ) -> Result<Self, ProjectCheck> {
        let proof = MainFlowProofCase::prepare(plan, plan.proof_source)?;
        let issuer = if plan.proof_source == root_source {
            SymbolicEqualityIssuer::for_symbolic_case(scope, plan, &proof)
        } else {
            None
        };
        let mut observer = Self::new(plan.target, proof);
        observer.first.1.issuer = issuer;
        Ok(observer)
    }

    fn insert(&mut self, scope: ScalarCheckerScope, allow_issuer: bool, plan: &PlannedModuleProof<'a>, proof: MainFlowProofPlan<'a>) {
        let target = plan.target;
        if self.first.0 == target {
            self.first.1.issuer = None;
            self.first.1.cases.push(proof);
            return;
        }
        match self.other.entry(target) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let group = entry.get_mut();
                group.issuer = None;
                group.cases.push(proof);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let issuer = allow_issuer.then(|| SymbolicEqualityIssuer::for_symbolic_case(scope, plan, &proof)).flatten();
                entry.insert(MainFlowProofTarget { issuer, ..MainFlowProofTarget::new(proof) });
            }
        }
    }

    fn complete(&self, reached: &HashSet<(usize, usize)>) -> bool {
        !self.incomplete
            && (self.first.1.seen || !reached.contains(&self.first.0))
            && self.other.iter().all(|(target, proofs)| proofs.seen || !reached.contains(target))
    }
}

#[cfg(feature = "internal-checker")]
impl ActualIntegerObserver for MainFlowProofObserver<'_> {
    const ENABLED: bool = false;
    const CONTEXTUAL: bool = true;

    fn observe(&mut self, _: (usize, usize), _: &[&BigInt], _: &BigInt) {}

    fn observe_initializer(
        &mut self, target: (usize, usize), site: Option<InitializerProofSite<'_>>,
        parameters: &[ScalarType], arguments: &[&BigInt], value: &BigInt,
    ) {
        let proofs = if self.first.0 == target {
            &mut self.first.1
        } else if let Some(proofs) = self.other.get_mut(&target) {
            proofs
        } else {
            return;
        };
        let Some(site) = site else {
            self.incomplete = true;
            return;
        };
        if arguments.len() != parameters.len() {
            self.incomplete = true;
            return;
        }
        let mut invocation: Option<Rc<MainFlowProofInvocation>> = None;
        let mut emit = |proof: &MainFlowProofCase<'_>, failed_step: Option<ByteSpan>| {
            let invoked = if let Some(invoked) = &invocation {
                Rc::clone(invoked)
            } else {
                let (outer, inner, directive_position) = match &site.call {
                    InitializerProofOrigin::Direct { topology, occurrence, directive_position } => {
                        let Some(node) = topology.occurrences.get(occurrence.0) else { return false; };
                        (None, node.span, *directive_position)
                    }
                    InitializerProofOrigin::Nested { outer, caller, inner, directive_position } => {
                        let Some(outer) = outer.topology.occurrences.get(outer.occurrence.0) else { return false; };
                        let Some(inner) = caller.occurrences.get(inner.0) else { return false; };
                        (Some(outer.span), inner.span, *directive_position)
                    }
                };
                let invoked = Rc::new(MainFlowProofInvocation {
                    outer_call: outer.map(|range| wosy_syntax::SourceSpan { source: site.source.clone(), range }),
                    call: wosy_syntax::SourceSpan { source: site.source.clone(), range: inner },
                    directive_position,
                    arguments: parameters.iter().cloned().zip(arguments.iter().map(|value| (**value).clone())).collect(),
                    returned: value.clone(),
                });
                invocation = Some(Rc::clone(&invoked));
                invoked
            };
            self.diagnostics.push(MainFlowProofDiagnostic {
                proof: wosy_syntax::SourceSpan { source: proof.proof_source.clone(), range: proof.proof },
                goal: wosy_syntax::SourceSpan { source: proof.proof_source.clone(), range: proof.goal },
                failed_step: failed_step.map(|range| wosy_syntax::SourceSpan { source: proof.proof_source.clone(), range }),
                invocation: invoked,
            });
            true
        };
        match &proofs.cases {
            MainFlowProofCases::Bare { first, rest } => {
                if arguments.len() != first.guard.arity {
                    self.incomplete = true;
                    return;
                }
                proofs.seen = true;
                for proof in std::iter::once(first).chain(rest) {
                    if !proof.guard.accepts(arguments) {
                        continue;
                    }
                    self.active = true;
                    let Some(bound) = proof.bound.for_actual(arguments) else {
                        self.incomplete = true;
                        return;
                    };
                    if !proof.relation.accepts(value.cmp(bound)) && !emit(proof, None) {
                        self.incomplete = true;
                        return;
                    }
                }
            }
            MainFlowProofCases::Authored(cases) => {
                if arguments.len() != cases[0].0.guard.arity {
                    self.incomplete = true;
                    return;
                }
                proofs.seen = true;
                for (proof, step) in cases {
                    let Some(bound) = proof.bound.for_actual(arguments) else {
                        self.incomplete = true;
                        return;
                    };
                    let mut applies = || {
                        if !proof.guard.accepts(arguments) {
                            return false;
                        }
                        self.active = true;
                        true
                    };
                    let failed_step = match step {
                        Some(MainFlowAuthoredShows::Single(step)) => {
                            if !applies() {
                                continue;
                            }
                            let comparison = value.cmp(step.bound.as_ref());
                            if !step.relation.accepts(comparison) {
                                Some(step.span)
                            } else {
                                let ordering = if step.same_bound { comparison } else { value.cmp(bound) };
                                if proof.relation.accepts(ordering) {
                                    continue;
                                }
                                None
                            }
                        }
                        Some(MainFlowAuthoredShows::Multiple { groups, .. }) => {
                            if !applies() {
                                continue;
                            }
                            let failed = first_failed_show(groups, value);
                            if failed.is_none() && proof.relation.accepts(value.cmp(bound)) {
                                continue;
                            }
                            failed
                        }
                        Some(MainFlowAuthoredShows::Entry { span, slot, relation }) => {
                            let Some(argument) = arguments.get(*slot) else {
                                self.incomplete = true;
                                return;
                            };
                            if !applies() {
                                continue;
                            }
                            if !relation.accepts(value.cmp(*argument)) {
                                Some(*span)
                            } else if proof.relation.accepts(value.cmp(bound)) {
                                continue;
                            } else {
                                None
                            }
                        }
                        None => {
                            if !applies() || proof.relation.accepts(value.cmp(bound)) {
                                continue;
                            }
                            None
                        }
                    };
                    if !emit(proof, failed_step) {
                        self.incomplete = true;
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
struct NamedOrderedEvidence<'a> {
    observed: MainFlowProofObserver<'a>,
}

#[cfg(feature = "internal-checker")]
impl ActualIntegerObserver for NamedOrderedEvidence<'_> {
    const ENABLED: bool = false;
    const CONTEXTUAL: bool = true;

    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}

    fn observe_initializer(
        &mut self, target: PreparedCallable, site: Option<InitializerProofSite<'_>>,
        parameters: &[ScalarType], arguments: &[&BigInt], value: &BigInt,
    ) {
        self.observed.observe_initializer(target, site, parameters, arguments, value);
    }
}

#[cfg(feature = "internal-checker")]
impl<'a> OrderedRunEvidence for NamedOrderedEvidence<'a> {
    type Symbolic<'run> = SelectedSymbolicProof<'a, 'run> where Self: 'run;
    type SymbolicReport = (Option<SymbolicProofDiagnostic>, Vec<SymbolicProofDiagnostic>);

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if !self.observed.complete(reached) {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(self.observed.active)
    }

    fn symbolic(&self) -> Self::Symbolic<'_> {
        SelectedSymbolicProof {
            watched: &self.observed,
            equalities: SymbolicEqualityIndex::default(),
            failure: None, additional_failures: Vec::new(), snapshot_work: 0,
        }
    }

    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run {
        (proof.failure, proof.additional_failures)
    }
}

#[cfg(feature = "internal-checker")]
struct RankedOrderedEvidence<'a, 'proof> {
    plan: &'proof PlannedModuleProof<'a>,
    rank: TypedInductionRank<'a>,
    allow_reached: bool,
    allow_symbolic_summary: bool,
    reached_return: Option<Option<VerifiedInductionExactReturn<'proof>>>,
}

#[cfg(feature = "internal-checker")]
impl ActualIntegerObserver for RankedOrderedEvidence<'_, '_> {
    const ENABLED: bool = true;
    const CONTEXTUAL: bool = true;

    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}

    fn proved_recursive_return(
        &mut self, target: PreparedCallable, arguments: &[&BigInt], expected: &ScalarType,
    ) -> Option<BigInt> {
        if !self.allow_reached || target != self.plan.target
            || !matches!(&self.plan.typed.goal, TypedProofGoal::Literal { operator: BinaryOperator::Equal, .. })
        {
            return None;
        }
        let (plan, rank) = (self.plan, self.rank);
        self.reached_return.get_or_insert_with(|| VerifiedInductionExactReturn::verify(plan, rank).1)
            .as_mut()?.consume(target, arguments, expected)
    }
}

#[cfg(feature = "internal-checker")]
struct RankedOrderedSymbolic<'a, 'proof> {
    plan: &'proof PlannedModuleProof<'a>,
    rank: TypedInductionRank<'a>,
    failure: Option<SymbolicProofDiagnostic>,
    allow_summary: bool,
    verified: Option<VerifiedSymbolicEquality<'proof>>,
}

#[cfg(feature = "internal-checker")]
impl<'a, 'proof> OrderedRunEvidence for RankedOrderedEvidence<'a, 'proof> {
    type Symbolic<'run> = RankedOrderedSymbolic<'a, 'proof> where Self: 'run;
    type SymbolicReport = Option<SymbolicProofDiagnostic>;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if !reached.contains(&self.plan.target) {
            return Ok(false);
        }
        if self.reached_return.as_ref().and_then(Option::as_ref).is_some_and(|token| token.used) {
            return Ok(true);
        }
        Err(ProjectCheck::Unsupported)
    }

    fn symbolic(&self) -> Self::Symbolic<'_> {
        RankedOrderedSymbolic {
            plan: self.plan, rank: self.rank, failure: None,
            allow_summary: self.allow_symbolic_summary, verified: None,
        }
    }

    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run {
        proof.failure
    }
}

#[cfg(feature = "internal-checker")]
impl SymbolicRootProof for RankedOrderedSymbolic<'_, '_> {
    const ENABLED: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        if !reached.contains(&self.plan.target) {
            let (module, item) = self.plan.target;
            roots.entry(module).or_default().insert(item);
        }
    }

    fn try_collect(
        &mut self, target: PreparedCallable, calls: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, effects: &mut OrderedFailures,
        _runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        if target != self.plan.target {
            let Some(first) = &self.verified else { return Ok(None); };
            return first.consume(|_| None, target, calls, source, effects);
        }
        match self.plan.verify_induction_return_witness(self.rank)? {
            RankedInductionOutcome::NoApplicable => Ok(Some(false)),
            RankedInductionOutcome::Premise(InductionPremise::Established | InductionPremise::Unreachable) => {
                if self.allow_summary {
                    self.verified = ranked_symbolic_equality(self.plan, source);
                }
                Ok(Some(true))
            }
            RankedInductionOutcome::Refuted(base) => {
                let proof_span = |range| wosy_syntax::SourceSpan { source: base.proof_source.clone(), range };
                let callable_span = |range| wosy_syntax::SourceSpan { source: source.clone(), range };
                let (goal, failed_step) = match base.rejected {
                    RankedBaseRejection::Goal(range) => (range, None),
                    RankedBaseRejection::AuthoredShow(range) => {
                        let goal = child(&child(self.plan.proof, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
                        (wosy_syntax::byte_span(&goal), Some(proof_span(range)))
                    }
                };
                let earlier = self.rank.earlier.zip(base.earlier.as_ref());
                let axes = earlier.into_iter().map(|((slot, binding), _)| (binding.clone(), slot.ty.clone()))
                    .chain([(self.rank.descending.1.clone(), base.entry_type)]).collect();
                self.failure = Some(SymbolicProofDiagnostic {
                    proof: proof_span(wosy_syntax::byte_span(self.plan.proof)),
                    goal: proof_span(goal), failed_step,
                    callable: callable_span(function.name_span),
                    output: wosy_syntax::SourceSpan { source: base.return_source.clone(), range: base.return_span },
                    directive_position: effects.records.len(),
                    witness: ScalarCheckerIntegerWitness {
                        equal_axes: Vec::new(),
                        axes,
                        state: IntegerInputs::entry_point(earlier.map(|(_, value)| value), &base.entry),
                    },
                    returned: base.returned.clone(),
                });
                Ok(Some(true))
            }
            _ => Err(ProjectCheck::Unsupported),
        }
    }
}

#[cfg(feature = "internal-checker")]
fn ranked_symbolic_equality<'proof>(
    plan: &'proof PlannedModuleProof<'_>, source: &SourceIdentity,
) -> Option<VerifiedSymbolicEquality<'proof>> {
    if plan.proof_source != source {
        return None;
    }
    let TypedProofGoal::Literal { operator: BinaryOperator::Equal, bound, .. } = &plan.typed.goal else {
        return None;
    };
    let guards = checked_integer_guard_slots::<true>(plan.typed.body, plan.filter.as_deref().unwrap_or_default())?;
    if guards.always_rejects {
        return None;
    }
    Some(VerifiedSymbolicEquality {
        target: plan.target, canonical_target: plan.canonical_target, layout: plan.typed.target.target_layout,
        body: plan.typed.body, bound, shared: std::cell::OnceCell::new(),
        guards: VerifiedSymbolicGuards::Ranked(guards),
    })
}

#[cfg(feature = "internal-checker")]
struct MixedOrderedEvidence<'a, 'proof> {
    ranked: HashMap<PreparedCallable, RankedOrderedEvidence<'a, 'proof>>,
    bare: Option<NamedOrderedEvidence<'proof>>,
}

#[cfg(feature = "internal-checker")]
impl ActualIntegerObserver for MixedOrderedEvidence<'_, '_> {
    const ENABLED: bool = true;
    const CONTEXTUAL: bool = true;

    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}

    fn observe_initializer(
        &mut self, target: PreparedCallable, site: Option<InitializerProofSite<'_>>,
        parameters: &[ScalarType], arguments: &[&BigInt], value: &BigInt,
    ) {
        if let Some(bare) = &mut self.bare {
            bare.observe_initializer(target, site, parameters, arguments, value);
        }
    }

    fn proved_recursive_return(
        &mut self, target: PreparedCallable, arguments: &[&BigInt], expected: &ScalarType,
    ) -> Option<BigInt> {
        self.ranked.get_mut(&target)?.proved_recursive_return(target, arguments, expected)
    }
}

#[cfg(feature = "internal-checker")]
struct MixedOrderedSymbolic<'a, 'proof, 'run> {
    ranked: HashMap<PreparedCallable, RankedOrderedSymbolic<'a, 'proof>>,
    bare: Option<SelectedSymbolicProof<'proof, 'run>>,
    ranked_only_equalities: SymbolicEqualityIndex<'proof>,
    first: Option<SymbolicProofDiagnostic>,
    rest: Vec<SymbolicProofDiagnostic>,
}

#[cfg(feature = "internal-checker")]
impl<'a, 'proof> OrderedRunEvidence for MixedOrderedEvidence<'a, 'proof> {
    type Symbolic<'run> = MixedOrderedSymbolic<'a, 'proof, 'run> where Self: 'run;
    type SymbolicReport = (Option<SymbolicProofDiagnostic>, Vec<SymbolicProofDiagnostic>);

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        let mut active = false;
        for rank in self.ranked.values() {
            active |= rank.complete_startup(reached)?;
        }
        if let Some(bare) = &self.bare {
            active |= bare.complete_startup(reached)?;
        }
        Ok(active)
    }

    fn symbolic(&self) -> Self::Symbolic<'_> {
        MixedOrderedSymbolic {
            ranked: self.ranked.iter().map(|(target, evidence)| (*target, evidence.symbolic())).collect(),
            bare: self.bare.as_ref().map(|evidence| evidence.symbolic()),
            ranked_only_equalities: SymbolicEqualityIndex::default(),
            first: None, rest: Vec::new(),
        }
    }

    fn finish_symbolic<'run>(proof: Self::Symbolic<'run>) -> Self::SymbolicReport where Self: 'run {
        (proof.first, proof.rest)
    }
}

#[cfg(feature = "internal-checker")]
impl SymbolicRootProof for MixedOrderedSymbolic<'_, '_, '_> {
    const ENABLED: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        for rank in self.ranked.values() {
            rank.seed(roots, reached);
        }
        if let Some(bare) = &self.bare {
            bare.seed(roots, reached);
        }
    }

    fn try_collect(
        &mut self, target: PreparedCallable, calls: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, effects: &mut OrderedFailures,
        runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        let (active, failure, additional) = if let Some(rank) = self.ranked.get_mut(&target) {
            let active = rank.try_collect(target, calls, function, source, effects, runtime)?;
            if let Some(summary) = rank.verified.take() {
                let equalities = self.bare.as_mut().map_or(&mut self.ranked_only_equalities, |bare| &mut bare.equalities);
                equalities.insert(summary);
            }
            (active, rank.failure.take(), Vec::new())
        } else if let Some(bare) = &mut self.bare {
            let active = bare.try_collect(target, calls, function, source, effects, runtime)?;
            (active, bare.failure.take(), std::mem::take(&mut bare.additional_failures))
        } else {
            return self.ranked_only_equalities.consume(target, calls, source, effects);
        };
        if let Some(failure) = failure {
            if self.first.is_none() {
                self.first = Some(failure);
            } else {
                self.rest.push(failure);
            }
        }
        self.rest.extend(additional);
        Ok(active)
    }
}

#[cfg(feature = "internal-checker")]
fn closed_ranked_evidence<'a, 'proof, const LOCAL_CATALOGS: bool>(
    root_source: &SourceIdentity, startup_count: usize, plan: &'proof PlannedModuleProof<'a>, rank: TypedInductionRank<'a>,
) -> Result<RankedOrderedEvidence<'a, 'proof>, ProjectCheck> {
    if plan.proof_source != &plan.typed.target.source {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(RankedOrderedEvidence {
        plan, rank,
        allow_reached: plan.proof_source == root_source || (LOCAL_CATALOGS && plan.target.0 < startup_count),
        allow_symbolic_summary: plan.proof_source == root_source,
        reached_return: None,
    })
}

#[cfg(feature = "internal-checker")]
fn run_closed_ranked_checks<'a, 'proof, const LOCAL_CATALOGS: bool>(
    prepared: &PreparedProofModule, scope: ScalarCheckerScope, startup_count: usize,
    first: &'proof PlannedModuleProof<'a>, others: &'proof [PlannedModuleProof<'a>],
) -> Result<MainFlowProofReceipt, ProjectCheck> {
    if others.is_empty() {
        let rank = first.typed.measure.ok_or(ProjectCheck::Unsupported)?;
        let ranked = closed_ranked_evidence::<LOCAL_CATALOGS>(&prepared.source, startup_count, first, rank)?;
        let run = run_prepared_ordered_check(
            &prepared.prepared, scope, startup_count, OrderedReachPolicy::Closed, ranked,
        )?;
        return Ok(MainFlowProofReceipt {
            active: run.active, directives: run.directives, diagnostics: Vec::new(),
            symbolic_diagnostic: run.symbolic, additional_symbolic_diagnostics: Vec::new(),
            entry_evidence: run.entry_evidence,
        });
    }

    let plans = || std::iter::once(first).chain(others);
    let mut ranked = HashMap::new();
    for plan in plans() {
        if let Some(rank) = plan.typed.measure {
            if !matches!(&plan.typed.goal, TypedProofGoal::Literal { operator: BinaryOperator::Equal, .. }) {
                return Err(ProjectCheck::Unsupported);
            }
            let evidence = closed_ranked_evidence::<LOCAL_CATALOGS>(&prepared.source, startup_count, plan, rank)?;
            if ranked.insert(plan.target, evidence).is_some() {
                return Err(ProjectCheck::Unsupported);
            }
        }
    }
    let mut observed: Option<MainFlowProofObserver<'_>> = None;
    for plan in plans().filter(|plan| plan.typed.measure.is_none()) {
        if !plan.steps.items.is_empty() || ranked.contains_key(&plan.target) {
            return Err(ProjectCheck::Unsupported);
        }
        if let Some(observer) = &mut observed {
            observer.insert(scope, plan.proof_source == &prepared.source, plan, MainFlowProofCase::prepare(plan, plan.proof_source)?);
        } else {
            observed = Some(MainFlowProofObserver::from_first_plan(scope, &prepared.source, plan)?);
        }
    }
    let run = run_prepared_ordered_check(
        &prepared.prepared, scope, startup_count, OrderedReachPolicy::Closed,
        MixedOrderedEvidence { ranked, bare: observed.map(|observed| NamedOrderedEvidence { observed }) },
    )?;
    let (symbolic_diagnostic, additional_symbolic_diagnostics) = run.symbolic;
    Ok(MainFlowProofReceipt {
        active: run.active, directives: run.directives,
        diagnostics: run.actual.bare.map_or_else(Vec::new, |bare| bare.observed.diagnostics),
        symbolic_diagnostic, additional_symbolic_diagnostics, entry_evidence: run.entry_evidence,
    })
}

#[cfg(feature = "internal-checker")]
fn run_closed_without_named_proofs(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<MainFlowProofReceipt, ProjectCheck> {
    let run = run_prepared_ordered_check(prepared, scope, startup_count, OrderedReachPolicy::Closed, ProofFreeOrderedEvidence)?;
    Ok(MainFlowProofReceipt {
        active: run.active, directives: run.directives, diagnostics: Vec::new(),
        symbolic_diagnostic: None, additional_symbolic_diagnostics: Vec::new(),
        entry_evidence: run.entry_evidence,
    })
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn check_loaded_main_flow_proof_diagnostics(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proofs: Vec<(ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> Result<MainFlowProofReceipt, ProjectCheck> {
    check_loaded_ordered_proof_diagnostics(declarations, proofs, initialization_order, inputs, ScalarCheckerScope::MainFlowOnly)
}

#[cfg(feature = "internal-checker")]
fn check_loaded_ordered_proof_diagnostics(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proofs: Vec<(ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
) -> Result<MainFlowProofReceipt, ProjectCheck> {
    check_loaded_ordered_proof_diagnostics_in_scope::<false>(declarations, proofs, initialization_order, inputs, scope, Vec::new())
}

#[cfg(feature = "internal-checker")]
fn check_loaded_ordered_proof_diagnostics_in_scope<const LOCAL_CATALOGS: bool>(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proofs: Vec<(ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
    dormant: Vec<ScalarProofDeclarations>,
) -> Result<MainFlowProofReceipt, ProjectCheck> {
    let dormant = if scope == ScalarCheckerScope::CurrentPackage { dormant } else { Vec::new() };
    if proofs.is_empty() && dormant.is_empty() {
        let (prepared, startup_count) = prepare_checked_package_scope(
            declarations, initialization_order, Vec::new(), inputs, scope,
        )?;
        return run_closed_without_named_proofs(&prepared, scope, startup_count);
    }
    let startup_count = initialization_order.len();
    let prepared = prepare_loaded_proof_project(declarations, proofs, initialization_order, inputs, dormant)?;
    let project = prepared.prepared.parts().0;
    let sources = if LOCAL_CATALOGS {
        project.initialization_order.as_slice()
    } else {
        if prepared.proofs.iter().any(|(source, catalog)| source != &prepared.source && !catalog.definitions.is_empty()) {
            return Err(ProjectCheck::Unsupported);
        }
        std::slice::from_ref(&prepared.source)
    };
    let mut first = None;
    let mut others = Vec::new();
    for source in sources.iter().chain(project.modules[startup_count..].iter().map(|module| &module.source)) {
        let Some((proof_source, catalog)) = prepared.proofs.get_key_value(source) else { continue; };
        if catalog.definitions.is_empty() {
            continue;
        }
        if LOCAL_CATALOGS && (proof_source.project != prepared.source.project || proof_source.package != prepared.source.package) {
            return Err(ProjectCheck::Unsupported);
        }
        let mut select = |index, definition, name| {
            let selection = (proof_source, name, definition, catalog.first_query + index);
            if first.is_some() {
                others.push(selection);
            } else {
                first = Some(selection);
            }
        };
        if let [definition] = catalog.definitions.as_slice() {
            select(0, definition, module_proof_name(definition).ok_or(ProjectCheck::InvalidProof)?);
        } else {
            let names = catalog.definitions.iter().map(|definition|
                module_proof_name(definition).ok_or(ProjectCheck::InvalidProof)
            ).collect::<Result<Vec<_>, _>>()?;
            let mut unique = HashSet::with_capacity(names.len());
            if !names.iter().all(|name| unique.insert(name.text())) {
                return Err(ProjectCheck::AmbiguousRoot);
            }
            drop(unique);
            for (index, (definition, name)) in catalog.definitions.iter().zip(names).enumerate() {
                select(index, definition, name);
            }
        }
    }
    let Some((source, name, definition, query)) = first.as_ref() else {
        return run_closed_without_named_proofs(&prepared.prepared, scope, startup_count);
    };
    let first_plan = prepared.plan_resolved_module_proof::<true, false, false>(
        source, name.text(), prepared.resolve_selected_proof(definition, *query)?,
    )?;
    let plans = others.iter().map(|(source, name, definition, query)| {
        let selected = prepared.resolve_selected_proof(definition, *query)?;
        prepared.plan_resolved_module_proof::<true, false, false>(source, name.text(), selected)
    }).collect::<Result<Vec<_>, _>>()?;
    if first_plan.typed.measure.is_some() || plans.iter().any(|plan| plan.typed.measure.is_some()) {
        return run_closed_ranked_checks::<LOCAL_CATALOGS>(&prepared, scope, startup_count, &first_plan, &plans);
    }
    let mut observer = MainFlowProofObserver::from_first_plan(scope, &prepared.source, &first_plan)?;
    for plan in &plans {
        observer.insert(scope, plan.proof_source == &prepared.source, plan, MainFlowProofCase::prepare(plan, plan.proof_source)?);
    }
    let run = run_prepared_ordered_check(
        &prepared.prepared, scope, startup_count, OrderedReachPolicy::Closed,
        NamedOrderedEvidence { observed: observer },
    )?;
    let (symbolic_diagnostic, additional_symbolic_diagnostics) = run.symbolic;
    Ok(MainFlowProofReceipt {
        active: run.active, directives: run.directives, diagnostics: run.actual.observed.diagnostics,
        symbolic_diagnostic, additional_symbolic_diagnostics, entry_evidence: run.entry_evidence,
    })
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_loaded_scalar_ordered_proof_contexts(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    selected_root: (ScalarProofDeclarations, Vec<ScalarNamespaceBinding>),
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> Result<MainFlowProofReceipt, ScalarOrderedCheckerOutcome> {
    check_loaded_ordered_proof_diagnostics(
        declarations, vec![selected_root], initialization_order, inputs, ScalarCheckerScope::CurrentPackage,
    ).map_err(|error| match error {
        ProjectCheck::Invalid(diagnostics) => ScalarOrderedCheckerOutcome::Invalid(diagnostics),
        _ => ScalarOrderedCheckerOutcome::Unsupported,
    })
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_loaded_scalar_ordered_local_proof_contexts(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    local_catalogs: impl IntoIterator<Item = (ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    scope: ScalarCheckerScope,
    dormant: Vec<ScalarProofDeclarations>,
) -> Result<MainFlowProofReceipt, ScalarOrderedCheckerOutcome> {
    check_loaded_ordered_proof_diagnostics_in_scope::<true>(
        declarations, local_catalogs.into_iter().collect(), initialization_order, inputs, scope, dormant,
    ).map_err(|error| match error {
        ProjectCheck::Invalid(diagnostics) => ScalarOrderedCheckerOutcome::Invalid(diagnostics),
        _ => ScalarOrderedCheckerOutcome::Unsupported,
    })
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarProofCallOutcome {
    Established,
    NotEstablished,
    Unavailable,
}

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
pub fn check_loaded_scalar_module_proof_call(
    proof: (&SourceIdentity, &str),
    caller: (&SourceIdentity, &str),
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    catalog: (ScalarProofDeclarations, Vec<ScalarNamespaceBinding>),
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> ScalarProofCallOutcome {
    let result = prepare_loaded_proof_project(declarations, vec![catalog], initialization_order, inputs, Vec::new())
        .ok()
        .and_then(|prepared| production_bound_reuse::apply(&prepared, proof, caller));
    result.unwrap_or(ScalarProofCallOutcome::Unavailable)
}

#[cfg(feature = "internal-checker")]
mod production_bound_reuse {
    use super::*;
    use super::super::reference::ParameterSlot;

    struct VerifiedModuleBound<'a> {
        issuer: &'a PreparedProofModule,
        proof_identity: (&'a SourceIdentity, ByteSpan, &'a str),
        guards: Vec<ImportedEntryGuard<'a>>,
        values: IntegerSet,
    }

    pub(super) fn apply(
        project: &PreparedProofModule,
        (source, name): (&SourceIdentity, &str),
        (caller_source, caller_name): (&SourceIdentity, &str),
    ) -> Option<ScalarProofCallOutcome> {
        let plan = project.plan_module_proof_in_scope::<true, true, true>(source, name).ok()?;
        let goal = &plan.typed.goal;
        if !matches!(goal, TypedProofGoal::Literal { .. } | TypedProofGoal::DeferredDisjunction(_)) {
            return None;
        }
        let (scalar, calls) = project.prepared.parts();
        let caller = select_checked_root(scalar, caller_source, caller_name).ok()?;
        let root = scalar.initialization_order.first()?;
        if plan.typed.measure.is_some()
            && (!matches!(goal, TypedProofGoal::Literal { operator: BinaryOperator::Equal, .. }) || plan.proof_source != root
                || &plan.typed.target.source != root || caller_source != root)
        {
            return None;
        }
        let initialize = || {
            let (root, ordinals) = package_initialization_order(scalar).ok()?;
            let initialization = check_package_initialization_with_order(&project.prepared, root, ordinals).ok()?;
            let &venue = initialization.ordinals.get(plan.proof_source)?;
            if initialization.visits[venue] != InitializationVisit::Completed
                || initialization.visits[plan.target.0] != InitializationVisit::Completed
                || initialization.visits[caller.0] != InitializationVisit::Completed
                || initialization.reached.contains(&plan.target)
                || initialization.reached.contains(&(caller.0, caller.1))
            {
                return None;
            }
            Some(())
        };
        let body = calls.function_body(caller.0, caller.1, None)?;
        let (position, origin) = root_local_import_requirement(body)?;
        if (caller.0, caller.1) == plan.target
            || !caller.2.generic_parameters.is_empty() || !caller.2.overload_arms.is_empty()
            || !body.allocation_length_calls.is_empty()
            || !matches!(body.call_occurrences.as_slice(), [call] if call.ordinal == origin.call)
            || body.structure.checker_directives.len() != 1
        {
            return None;
        }
        let site = body.resolved_call_site(origin.call)?;
        let signature = body.checked_call_signatures.get(&origin.call)?;
        let layout = scalar.modules[caller.0].target_layout;
        let producer = plan.typed.body;
        let [output] = producer.boundary.results.as_slice() else { return None; };
        if output.ordinal != 0 || !matching_call(
            plan.canonical_target, plan.typed.target.target_layout, &producer.boundary.parameters, &output.ty,
            layout, body, origin, &site.target, signature,
        ) {
            return None;
        }
        let guard = checked_integer_guard_slots::<true>(producer, plan.filter.as_deref().unwrap_or_default())?;
        if guard.always_rejects {
            return None;
        }
        if let Some((slot, domain)) = guard.domain {
            if plan.proof_source != root || &plan.typed.target.source != root || caller_source != root
                || plan.typed.measure.is_some() || !plan.steps.items.is_empty() || !valid_arbitrary_proof_body(producer)
            {
                return None;
            }
            let mut issue = || {
                initialize()?;
                proved_arbitrary_values(&plan, &output.ty)
            };
            return imported_outcome(body, ImportedLocal {
                position,
                guards: guard.comparisons,
                values: ImportedIntegerValues::Deferred { slot, domain, issue: &mut issue },
            });
        }
        let TypedProofGoal::Literal { bound, .. } = goal else { return None; };
        initialize()?;
        let identity = (plan.proof_source, wosy_syntax::byte_span(plan.proof), name);
        let verified = VerifiedModuleBound::establish(project, &plan, identity, bound, &output.ty, guard.comparisons)?;
        verified.consume(project, identity, body, position)
    }

    impl<'a> VerifiedModuleBound<'a> {
        fn establish(
            issuer: &'a PreparedProofModule,
            plan: &'a PlannedModuleProof<'_>,
            proof_identity: (&'a SourceIdentity, ByteSpan, &'a str),
            bound: &BigInt,
            output: &ScalarType,
            guards: Vec<ImportedEntryGuard<'a>>,
        ) -> Option<Self> {
            let body = plan.typed.body;
            let values = if let Some(rank) = plan.typed.measure {
                if !matches!(plan.verify_unused_induction(rank), Ok(ProjectCheck::Proved)) {
                    return None;
                }
                IntegerSet::singleton(bound)
            } else {
                if !plan.steps.items.is_empty() || !valid_arbitrary_proof_body(body) {
                    return None;
                }
                proved_arbitrary_values(plan, output)?
            };
            Some(Self {
                issuer, proof_identity, guards, values,
            })
        }

        fn consume(
            self,
            issuer: &PreparedProofModule,
            identity: (&SourceIdentity, ByteSpan, &str),
            body: &FunctionCallSites,
            position: usize,
        ) -> Option<ScalarProofCallOutcome> {
            if !std::ptr::eq(self.issuer, issuer) || self.proof_identity != identity {
                return None;
            }
            imported_outcome(body, ImportedLocal {
                position, values: ImportedIntegerValues::ready(self.values), guards: self.guards,
            })
        }
    }

    pub(super) fn matching_call(
        producer_target: &ResolvedCallTarget, producer_layout: ScalarTargetLayout,
        parameters: &[ParameterSlot], output_type: &ScalarType, caller_layout: ScalarTargetLayout,
        body: &FunctionCallSites, origin: &CheckerIntegerCallOrigin, target: &ResolvedCallTarget,
        signature: &CheckedCallSignature,
    ) -> bool {
        let [output] = signature.outputs.outputs.as_slice() else { return false; };
        let arguments = origin.arguments.as_slice();
        producer_target == target && producer_layout == caller_layout && output_type == &output.ty
            && parameters.len() == signature.parameters.len() && parameters.len() == arguments.len()
            && parameters.iter().zip(&signature.parameters).zip(arguments).all(|((producer, caller), (ordinal, _))| {
                &producer.ty == caller && body.structure.checked_types.expressions.get(ordinal) == Some(caller)
            })
    }

    fn proved_arbitrary_values(
        plan: &PlannedModuleProof<'_>, output: &ScalarType,
    ) -> Option<IntegerSet> {
        let goal = match &plan.typed.goal {
            TypedProofGoal::DeferredDisjunction(atoms) => Some(TypedProofGoal::Disjunction(normalize_output_disjunction(output, atoms)?)),
            _ => None,
        };
        let checked_goal = goal.as_ref().unwrap_or(&plan.typed.goal);
        let ProofCoverage::Applicable(paths) = observe_arbitrary_integer_returns(plan.typed.body, plan.filter.as_deref())?.ok()? else {
            return None;
        };
        if !paths.iter().all(|path| checked_goal.holds_on_returned_path(path) == Some(true)) {
            return None;
        }
        match (goal, &plan.typed.goal) {
            (Some(TypedProofGoal::Disjunction(allowed)), _) => Some(allowed),
            (None, TypedProofGoal::Literal { operator, bound, .. }) => IntegerSet::from_type(output)?.keep(operator, bound),
            _ => None,
        }
    }

    fn imported_outcome<'a>(body: &'a FunctionCallSites, fact: ImportedLocal<'a>) -> Option<ScalarProofCallOutcome> {
        Some(if proves_requirement_with_imported_root_local(body, fact)? {
            ScalarProofCallOutcome::Established
        } else {
            ScalarProofCallOutcome::NotEstablished
        })
    }
}

#[cfg(feature = "internal-checker")]
fn module_proof_outcome(result: Result<ProjectCheck, ProjectCheck>) -> ScalarModuleProofOutcome {
    match result {
        Ok(ProjectCheck::Proved) => ScalarModuleProofOutcome::Proved,
        Ok(ProjectCheck::Refuted) => ScalarModuleProofOutcome::Refuted,
        Ok(ProjectCheck::NoApplicableInputs) => ScalarModuleProofOutcome::NoApplicableInputs,
        Err(ProjectCheck::Invalid(diagnostics)) => ScalarModuleProofOutcome::Invalid(diagnostics),
        Err(ProjectCheck::InvalidProof) => ScalarModuleProofOutcome::Invalid(Vec::new()),
        _ => ScalarModuleProofOutcome::Unsupported,
    }
}

#[cfg(feature = "internal-checker")]
#[derive(PartialEq, Eq)]
struct ExactGuardGroup<'a>(Option<&'a [CheckerPredicate]>);

#[cfg(feature = "internal-checker")]
impl Hash for ExactGuardGroup<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.map(<[_]>::len).hash(state);
        for predicate in self.0.unwrap_or_default() {
            std::mem::discriminant(predicate).hash(state);
            match predicate {
                CheckerPredicate::Boolean(value) => value.hash(state),
                CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => {}
                CheckerPredicate::EntryDomain { binding, values } => {
                    binding.hash(state);
                    values.hash(state);
                }
                CheckerPredicate::Integer(predicate) => {
                    predicate.binding.hash(state);
                    std::mem::discriminant(&predicate.operator).hash(state);
                    predicate.bound.hash(state);
                }
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
struct PlannedModuleProof<'a> {
    proof_source: &'a SourceIdentity,
    proof: &'a CstNode,
    canonical_target: &'a ResolvedCallTarget,
    target: (usize, usize),
    function: &'a ScalarFunction,
    name: &'a str,
    typed: TypedProofPostcondition<'a>,
    steps: ScalarBlock,
    filter: Option<Vec<CheckerPredicate>>,
}

#[cfg(feature = "internal-checker")]
struct LocalPeerCatalog<'a> {
    source: &'a SourceIdentity,
    proofs: &'a ModuleProofs,
    names: HashMap<String, Option<usize>>,
}

#[cfg(feature = "internal-checker")]
impl<'a> LocalPeerCatalog<'a> {
    fn new(source: &'a SourceIdentity, proofs: &'a ModuleProofs) -> Self {
        #[cfg(test)]
        PROOF_CATALOG_INDEX_BUILDS.with(|count| count.set(count.get() + 1));
        let mut names = HashMap::new();
        for (index, definition) in proofs.definitions.iter().enumerate() {
            if let Some(name) = module_proof_name(definition) {
                names.entry(name.text().to_owned()).and_modify(|entry| *entry = None).or_insert(Some(index));
            }
        }
        Self { source, proofs, names }
    }

    fn declaration(&self, name: &str) -> Result<(&str, &'a CstNode, usize), ProjectCheck> {
        let (name, slot) = self.names.get_key_value(name).ok_or(ProjectCheck::MissingRoot)?;
        let index = (*slot).ok_or(ProjectCheck::AmbiguousRoot)?;
        Ok((name.as_str(), &self.proofs.definitions[index], self.proofs.first_query + index))
    }
}

#[cfg(feature = "internal-checker")]
struct RequestedProofSource<'a> {
    first: (&'a str, usize),
    additional: HashMap<&'a str, usize>,
    catalog: Option<Box<LocalPeerCatalog<'a>>>,
}

#[cfg(feature = "internal-checker")]
impl RequestedProofSource<'_> {
    fn request(&self, name: &str) -> Option<&usize> {
        if self.first.0 == name { Some(&self.first.1) } else { self.additional.get(name) }
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct RequestedProofs<'a> {
    sources: HashMap<&'a SourceIdentity, RequestedProofSource<'a>>,
    distinct: usize,
}

#[cfg(feature = "internal-checker")]
impl<'a> RequestedProofs<'a> {
    fn request(&self, source: &SourceIdentity, name: &str) -> Option<&usize> {
        self.sources.get(source)?.request(name)
    }

    fn catalog(&self, source: &SourceIdentity) -> Option<&LocalPeerCatalog<'a>> {
        self.sources.get(source)?.catalog.as_deref()
    }
}

#[cfg(feature = "internal-checker")]
enum LocalPeerPlan<'a> {
    Selected(&'a PlannedModuleProof<'a>),
    Cited(Box<PlannedModuleProof<'a>>),
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy, Eq, PartialEq)]
enum LocalPeerState {
    Dormant,
    Pending,
    Visiting,
    Rejected,
    Terminal(usize),
}

#[cfg(feature = "internal-checker")]
struct LocalPeerNode<'a> {
    plan: Option<LocalPeerPlan<'a>>,
    next: Option<usize>,
    state: LocalPeerState,
}

#[cfg(feature = "internal-checker")]
impl<'a> LocalPeerNode<'a> {
    fn plan(&self) -> Option<&PlannedModuleProof<'a>> {
        match self.plan.as_ref()? {
            LocalPeerPlan::Selected(plan) => Some(plan),
            LocalPeerPlan::Cited(plan) => Some(plan),
        }
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct LocalPeerGraph<'a> {
    by_declaration: HashMap<usize, usize>,
    nodes: Vec<LocalPeerNode<'a>>,
    active: Vec<usize>,
}

#[cfg(feature = "internal-checker")]
impl<'a> LocalPeerGraph<'a> {
    fn enroll(
        &mut self, project: &'a PreparedProofModule,
        selected: &RequestedProofs<'_>,
        plans: &'a [(usize, PlannedModuleProof<'a>)],
        source: &'a SourceIdentity, (name, node, declaration): (&'a str, &'a CstNode, usize),
    ) -> usize {
        *self.by_declaration.entry(declaration).or_insert_with(|| {
            let plan = if let Some(request) = selected.request(source, name) {
                plans.binary_search_by_key(request, |(index, _)| *index).ok()
                    .map(|at| LocalPeerPlan::Selected(&plans[at].1))
            } else {
                #[cfg(test)]
                UNSELECTED_PEER_PLANS.with(|count| count.set(count.get() + 1));
                project.resolve_selected_proof(node, declaration)
                    .and_then(|proof| project.plan_resolved_module_proof::<false, true, false>(source, name, proof))
                    .ok().map(|plan| LocalPeerPlan::Cited(Box::new(plan)))
            };
            let index = self.nodes.len();
            self.nodes.push(LocalPeerNode { plan, next: None, state: LocalPeerState::Dormant });
            index
        })
    }

    fn activate(&mut self, index: usize) {
        if self.nodes[index].state == LocalPeerState::Dormant {
            self.nodes[index].state = LocalPeerState::Pending;
            self.active.push(index);
        }
    }

    fn resolve_edges(
        &mut self, project: &'a PreparedProofModule,
        selected: &'a RequestedProofs<'a>,
        plans: &'a [(usize, PlannedModuleProof<'a>)],
    ) {
        let mut ordinal = 0;
        while let Some(&index) = self.active.get(ordinal) {
            let request = (|| {
                let plan = self.nodes[index].plan()?;
                plan.peer_literal_goal()?;
                if plan.steps.items.is_empty() {
                    return None;
                }
                let catalog = selected.catalog(plan.proof_source)?;
                Some((catalog.source, catalog.declaration(plan.direct_peer_use()?).ok()?))
            })();
            if let Some((source, declaration)) = request {
                let next = self.enroll(project, selected, plans, source, declaration);
                let from = self.nodes[index].plan().expect("planned peer request");
                if self.nodes[next].plan().is_some_and(|to| from.same_peer_context(to) && to.peer_literal_goal().is_some()) {
                    self.nodes[index].next = Some(next);
                    self.activate(next);
                }
            } else if self.nodes[index].plan().is_some_and(|plan|
                plan.peer_literal_goal().is_some() && (plan.steps.items.is_empty() || plan.has_flat_show_steps()))
            {
                self.nodes[index].state = LocalPeerState::Terminal(index);
            } else {
                self.nodes[index].state = LocalPeerState::Rejected;
            }
            ordinal += 1;
        }
    }

    fn resolve_terminals(&mut self) {
        let mut path = Vec::new();
        for first in 0..self.nodes.len() {
            if self.nodes[first].state != LocalPeerState::Pending {
                continue;
            }
            let mut node = first;
            let mut resolved = loop {
                match self.nodes[node].state {
                    LocalPeerState::Pending => {
                        self.nodes[node].state = LocalPeerState::Visiting;
                        path.push(node);
                        let Some(next) = self.nodes[node].next else { break LocalPeerState::Rejected; };
                        node = next;
                    }
                    LocalPeerState::Visiting | LocalPeerState::Dormant => break LocalPeerState::Rejected,
                    state => break state,
                }
            };
            while let Some(node) = path.pop() {
                if let LocalPeerState::Terminal(terminal) = resolved {
                    let goal = |index: usize| self.nodes[index].plan().and_then(PlannedModuleProof::peer_literal_goal);
                    if goal(node) != goal(terminal) {
                        resolved = LocalPeerState::Rejected;
                    }
                }
                self.nodes[node].state = resolved;
            }
        }
    }

    fn terminal(&self, index: usize) -> Option<&PlannedModuleProof<'a>> {
        let LocalPeerState::Terminal(terminal) = self.nodes[index].state else { return None; };
        self.nodes[terminal].plan()
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct UnusedModuleProofGroup<'plan, 'project> {
    direct: Vec<(usize, &'plan PlannedModuleProof<'project>)>,
    peer_uses: Vec<UnusedPeerUse<'plan, 'project>>,
    paired_uses: Vec<PairedPeerUse<'plan, 'project>>,
}

#[cfg(feature = "internal-checker")]
struct PairedPeerUse<'plan, 'project> {
    index: usize,
    consumer: &'plan PlannedModuleProof<'project>,
    leaves: [usize; 2],
    entails: bool,
}

#[cfg(feature = "internal-checker")]
type AuthoredLeafKey<'a> = (&'a SourceIdentity, &'a str);

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct ImplicitAuthoredShows<'a> {
    first: Option<(AuthoredLeafKey<'a>, Option<usize>, bool)>,
    index: HashMap<AuthoredLeafKey<'a>, Option<usize>>,
    additional: Vec<bool>,
}

#[cfg(feature = "internal-checker")]
impl<'a> ImplicitAuthoredShows<'a> {
    fn cite<T>(
        &mut self, key: AuthoredLeafKey<'a>, prepare: impl FnOnce() -> Option<T>,
    ) -> (Option<AuthoredShowDestination>, Option<T>) {
        let (slot, check) = match self.first {
            Some((first, slot, _)) if first == key => (slot, None),
            None => {
                let check = prepare();
                let slot = check.as_ref().map(|_| 0);
                self.first = Some((key, slot, false));
                (slot, check)
            }
            Some(_) => match self.index.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => (*entry.get(), None),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let check = prepare();
                    let slot = check.as_ref().map(|_| {
                        self.additional.push(false);
                        self.additional.len()
                    });
                    entry.insert(slot);
                    (slot, check)
                }
            },
        };
        (slot.map(AuthoredShowDestination::Implicit), check)
    }

    fn outcome_mut(&mut self, slot: usize) -> &mut bool {
        if slot == 0 {
            &mut self.first.as_mut().expect("first authored leaf slot").2
        } else {
            &mut self.additional[slot - 1]
        }
    }

    fn proved(&self, slot: usize) -> bool {
        if slot == 0 {
            self.first.as_ref().expect("first authored leaf slot").2
        } else {
            self.additional[slot - 1]
        }
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy)]
enum AuthoredShowDestination {
    Selected(usize),
    Implicit(usize),
}

#[cfg(feature = "internal-checker")]
impl AuthoredShowDestination {
    fn record(
        self, outcome: ScalarModuleProofOutcome,
        selected: &mut [ScalarModuleProofOutcome], implicit: &mut ImplicitAuthoredShows<'_>,
    ) {
        match self {
            Self::Selected(index) => selected[index] = outcome,
            Self::Implicit(index) => *implicit.outcome_mut(index) = matches!(outcome, ScalarModuleProofOutcome::Proved),
        }
    }

    fn proved(self, selected: &[ScalarModuleProofOutcome], implicit: &ImplicitAuthoredShows<'_>) -> bool {
        match self {
            Self::Selected(index) => matches!(selected[index], ScalarModuleProofOutcome::Proved),
            Self::Implicit(index) => implicit.proved(index),
        }
    }
}

#[cfg(feature = "internal-checker")]
struct AuthoredShowCheck<'step, 'project> {
    destination: AuthoredShowDestination,
    plan: &'step PlannedModuleProof<'project>,
    relation: IntegerRelation,
    bound: &'step BigInt,
    proof: IntegerShowProof<'step>,
}

#[cfg(feature = "internal-checker")]
impl<'step, 'project: 'step> AuthoredShowCheck<'step, 'project> {
    fn group_key(&self) -> ((usize, usize), ExactGuardGroup<'step>) {
        (self.plan.target, ExactGuardGroup(self.plan.filter.as_deref()))
    }
}

#[cfg(feature = "internal-checker")]
struct AuthoredShowGroup<'step, 'project> {
    first: AuthoredShowCheck<'step, 'project>,
    additional: Vec<AuthoredShowCheck<'step, 'project>>,
}

#[cfg(feature = "internal-checker")]
impl<'step, 'project: 'step> AuthoredShowGroup<'step, 'project> {
    fn new(first: AuthoredShowCheck<'step, 'project>) -> Self {
        Self { first, additional: Vec::new() }
    }

    fn checks(self) -> impl Iterator<Item = AuthoredShowCheck<'step, 'project>> {
        std::iter::once(self.first).chain(self.additional)
    }
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct AuthoredShowSchedule<'step, 'project> {
    first: Option<AuthoredShowCheck<'step, 'project>>,
    groups: HashMap<((usize, usize), ExactGuardGroup<'step>), AuthoredShowGroup<'step, 'project>>,
}

#[cfg(feature = "internal-checker")]
impl<'step, 'project: 'step> AuthoredShowSchedule<'step, 'project> {
    fn push(&mut self, check: AuthoredShowCheck<'step, 'project>) {
        if self.first.is_none() && self.groups.is_empty() {
            self.first = Some(check);
            return;
        }
        if let Some(first) = self.first.take() {
            self.groups.insert(first.group_key(), AuthoredShowGroup::new(first));
        }
        match self.groups.entry(check.group_key()) {
            std::collections::hash_map::Entry::Occupied(mut group) => group.get_mut().additional.push(check),
            std::collections::hash_map::Entry::Vacant(group) => { group.insert(AuthoredShowGroup::new(check)); }
        }
    }

    fn verify(self, selected: &mut [ScalarModuleProofOutcome], implicit: &mut ImplicitAuthoredShows<'_>) {
        if let Some(first) = self.first {
            verify_authored_show_group(std::iter::once(first), selected, implicit);
        }
        for group in self.groups.into_values() {
            verify_authored_show_group(group.checks(), selected, implicit);
        }
    }
}

#[cfg(feature = "internal-checker")]
fn verify_authored_show_group<'step, 'project: 'step>(
    checks: impl IntoIterator<Item = AuthoredShowCheck<'step, 'project>>,
    selected: &mut [ScalarModuleProofOutcome], implicit: &mut ImplicitAuthoredShows<'_>,
) {
    let mut checks = checks.into_iter();
    let Some(first) = checks.next() else { return; };
    let plan = first.plan;
    let body = plan.typed.body;
    let authored_value = std::cell::OnceCell::new();
    let branch_values = std::array::from_fn(|_| std::cell::OnceCell::new());
    let rows = observe_authored_show_returns(body, plan.filter.as_deref(), &plan.function.body, &authored_value, &branch_values);
    let checks = std::iter::once(first).chain(checks);
    match rows {
        Some(Ok(ProofCoverage::Applicable(paths))) => {
            for check in checks {
                let outcome = match evaluate_borrowed_return_proof(
                    &paths, check.relation, check.bound, Some(&check.proof),
                ) {
                    Some(true) => ScalarModuleProofOutcome::Proved,
                    Some(false) => ScalarModuleProofOutcome::Refuted,
                    None => ScalarModuleProofOutcome::Unsupported,
                };
                check.destination.record(outcome, selected, implicit);
            }
        }
        Some(Ok(ProofCoverage::NoApplicableInputs)) => {
            for check in checks {
                check.destination.record(ScalarModuleProofOutcome::NoApplicableInputs, selected, implicit);
            }
        }
        Some(Err(failure)) => {
            let mut indices = checks.filter_map(|check| match check.destination {
                AuthoredShowDestination::Selected(index) => Some(index),
                AuthoredShowDestination::Implicit(_) => None,
            }).peekable();
            if indices.peek().is_some() {
                let shared = module_proof_outcome(Err(project_failure(body, &plan.typed.target.source, failure)));
                while let Some(index) = indices.next() {
                    if indices.peek().is_none() {
                        selected[index] = shared;
                        break;
                    }
                    selected[index] = shared.clone();
                }
            }
        }
        None => {}
    }
}

#[cfg(feature = "internal-checker")]
enum PeerShowClaims<'a> {
    One(IntegerRelation, Cow<'a, BigInt>),
    Many(Box<IntegerShowConstraints<'a>>),
}

#[cfg(feature = "internal-checker")]
struct UnusedPeerUse<'plan, 'project> {
    index: usize,
    consumer: &'plan PlannedModuleProof<'project>,
    name: &'plan str,
    relation: IntegerRelation,
    bound: &'plan BigInt,
    strict_entry: Option<StrictPeerDomain<'plan>>,
    implication_checked: bool,
    mixed_implies_claims: Option<bool>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct StrictPeerDomain<'a> {
    first: ImmutableEntryRange<'a>,
    second: Option<ImmutableEntryRange<'a>>,
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    static STRICT_PEER_ENTRY_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static UNSELECTED_PEER_PLANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PROOF_CATALOG_INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(feature = "internal-checker")]
fn strict_peer_entry_returns(
    paths: &[super::checker_flow::IntegerReturningPath<'_>], entry: StrictPeerDomain<'_>,
) -> bool {
    #[cfg(test)]
    STRICT_PEER_ENTRY_CHECKS.with(|count| count.set(count.get() + 1));
    paths.iter().any(|path| path.covers_immutable_entry_domain(entry.first, entry.second))
}

#[cfg(feature = "internal-checker")]
enum CheckedPeerLeaf {
    Proven,
    Refuted,
    Unavailable,
}

#[cfg(feature = "internal-checker")]
#[derive(Default)]
struct PeerPredicateBounds<'a> {
    first: Option<(&'a ScalarType, (BigInt, BigInt))>,
    other: Vec<(&'a ScalarType, (BigInt, BigInt))>,
}

#[cfg(feature = "internal-checker")]
impl<'a> PeerPredicateBounds<'a> {
    fn bounds_for(&mut self, ty: &'a ScalarType) -> Option<&(BigInt, BigInt)> {
        let first = &mut self.first;
        if first.is_none() {
            *first = Some((ty, integer_bounds(ty)?));
        }
        let (known, bounds) = first.as_ref()?;
        if *known == ty {
            return Some(bounds);
        }
        let slot = if let Some(slot) = self.other.iter().position(|(known, _)| *known == ty) {
            slot
        } else {
            self.other.push((ty, integer_bounds(ty)?));
            self.other.len() - 1
        };
        Some(&self.other[slot].1)
    }

    fn implies(
        &mut self, ty: &'a ScalarType,
        premise: (IntegerRelation, &BigInt), conclusion: (IntegerRelation, &BigInt),
    ) -> bool {
        premise == conclusion || self.bounds_for(ty).is_some_and(|(low, high)|
            integer_predicate_implies(premise, conclusion, (low, high)))
    }
}

#[cfg(feature = "internal-checker")]
impl PreparedProofModule {
    #[cfg(test)]
    fn plan_module_proof<'a>(&'a self, proof_source: &SourceIdentity, proof_name: &'a str) -> Result<PlannedModuleProof<'a>, ProjectCheck> {
        self.plan_module_proof_in_scope::<false, true, false>(proof_source, proof_name)
    }

    fn plan_module_proof_in_scope<'a, const ALLOW_INDUCTION: bool, const ALLOW_DOMAIN: bool, const DEFER_OUTPUT_UNION: bool>(
        &'a self, proof_source: &SourceIdentity, proof_name: &'a str,
    ) -> Result<PlannedModuleProof<'a>, ProjectCheck> {
        let (proof_source, _) = self.proofs.get_key_value(proof_source).ok_or(ProjectCheck::MissingRoot)?;
        let proof = self.select_resolved(proof_source, proof_name)?;
        self.plan_resolved_module_proof::<ALLOW_INDUCTION, ALLOW_DOMAIN, DEFER_OUTPUT_UNION>(proof_source, proof_name, proof)
    }

    fn plan_resolved_module_proof<'a, const ALLOW_INDUCTION: bool, const ALLOW_DOMAIN: bool, const DEFER_OUTPUT_UNION: bool>(
        &'a self, proof_source: &'a SourceIdentity, proof_name: &'a str, proof: SelectedModuleProof<'a>,
    ) -> Result<PlannedModuleProof<'a>, ProjectCheck> {
        let (module, item, function) = proof.function;
        let steps = derive_block(&child(proof.node, SyntaxKind::Block)?);
        let typed = typed_proof_postcondition::<ALLOW_INDUCTION, DEFER_OUTPUT_UNION>(&self.prepared, proof.function, proof.node, &steps, !DEFER_OUTPUT_UNION)?;
        let filter = prepare_proof_entry_filter_in_scope(
            proof.node, proof_source, typed.target.target_layout, &typed.input_names, typed.body,
            typed.measure.is_some() || (ALLOW_DOMAIN && steps.items.is_empty()),
        )?;
        Ok(PlannedModuleProof {
            proof_source, proof: proof.node, canonical_target: proof.target, target: (module, item),
            function, name: proof_name, typed, steps, filter,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn audit_induction_premise(
        &self, proof_source: &SourceIdentity, proof_name: &str,
    ) -> Result<InductionPremise, ProjectCheck> {
        let proof = self.select_resolved(proof_source, proof_name)?;
        let steps = derive_block(&child(proof.node, SyntaxKind::Block)?);
        let typed = typed_proof_postcondition::<true, false>(&self.prepared, proof.function, proof.node, &steps, false)?;
        let measure = typed.measure.ok_or(ProjectCheck::Unsupported)?;
        let filter = prepare_proof_entry_filter_in_scope(
            proof.node, proof_source, typed.target.target_layout, &typed.input_names, typed.body, true,
        )?;
        let proof_case = prepare_decrement_proof_steps(proof_name, &steps, &typed, measure)?;
        check_decrement_induction_premise(measure, &proof_case, filter.as_deref().unwrap_or_default())
    }

    #[cfg(test)]
    fn audit_induction_return_witness(
        &self, proof_source: &SourceIdentity, proof_name: &str,
    ) -> Result<InductionPremise, ProjectCheck> {
        let plan = self.plan_module_proof_in_scope::<true, false, false>(proof_source, proof_name)?;
        let measure = plan.typed.measure.ok_or(ProjectCheck::Unsupported)?;
        match plan.verify_induction_return_witness(measure)? {
            RankedInductionOutcome::NoApplicable => Ok(InductionPremise::Unreachable),
            RankedInductionOutcome::Premise(premise) => Ok(premise),
            RankedInductionOutcome::Refuted(_) => Err(ProjectCheck::Refuted),
        }
    }

    fn check_module_proofs(&self, proof_names: &[&str]) -> Vec<ScalarModuleProofOutcome> {
        let selections = proof_names.iter().map(|&name| (&self.source, name)).collect::<Vec<_>>();
        self.check_module_proof_selections(&selections)
    }

    fn check_module_proof_selections(&self, selections: &[(&SourceIdentity, &str)]) -> Vec<ScalarModuleProofOutcome> {
        let mut outcomes = vec![ScalarModuleProofOutcome::Unsupported; selections.len()];
        let mut selected = RequestedProofs::default();
        let mut duplicate = Vec::new();
        let mut plans = Vec::new();
        for (index, &(proof_source, name)) in selections.iter().enumerate() {
            let bucket = selected.sources.entry(proof_source).or_insert_with(|| RequestedProofSource {
                first: (name, index), additional: HashMap::new(), catalog: None,
            });
            if let Some(&prior) = bucket.request(name) {
                if prior != index {
                    duplicate.push((index, prior));
                    continue;
                }
            } else {
                bucket.additional.insert(name, index);
            }
            selected.distinct += 1;
            let planned = if bucket.additional.is_empty() {
                self.plan_module_proof_in_scope::<true, true, false>(proof_source, name)
            } else {
                (|| {
                    let (canonical, proofs) = self.proofs.get_key_value(proof_source).ok_or(ProjectCheck::MissingRoot)?;
                    let catalog = bucket.catalog.get_or_insert_with(|| Box::new(LocalPeerCatalog::new(canonical, proofs)));
                    let (_, node, declaration) = catalog.declaration(name)?;
                    let proof = self.resolve_selected_proof(node, declaration)?;
                    self.plan_resolved_module_proof::<true, true, false>(canonical, name, proof)
                })()
            };
            match planned {
                Ok(plan) => plans.push((index, plan)),
                Err(error) => outcomes[index] = module_proof_outcome(Err(error)),
            }
        }
        self.run_module_proof_plans(&plans, &mut selected, &mut outcomes);
        for (index, first) in duplicate {
            outcomes[index] = outcomes[first].clone();
        }
        outcomes
    }

    fn run_module_proof_plans<'a>(
        &'a self,
        plans: &[(usize, PlannedModuleProof<'a>)],
        selected: &mut RequestedProofs<'a>,
        outcomes: &mut [ScalarModuleProofOutcome],
    ) {
        if plans.is_empty() {
            return;
        }
        let Ok((root, ordinals)) = package_initialization_order(self.prepared.parts().0) else { return; };
        let mut independently_checked = None;
        let mut induction = None;
        if let (true, [(_, plan)]) = (selected.distinct == 1, plans) {
            if plan.proof_source == root && &plan.typed.target.source == root
                && matches!(&plan.typed.goal, TypedProofGoal::Literal { operator: BinaryOperator::Equal, .. })
            {
                if let Some(rank) = plan.typed.measure {
                    let (checked, token) = VerifiedInductionExactReturn::verify(plan, rank);
                    induction = token;
                    independently_checked = Some(checked);
                }
            }
        }
        let mut observers = ActualProofBatch {
            targets: HashMap::new(), proofs: Vec::with_capacity(plans.len()), induction,
        };
        for (plan_index, (_, plan)) in plans.iter().enumerate() {
            if plan.typed.measure.is_none() {
                observers.targets.entry(plan.target).or_default().push(plan_index);
                observers.proofs.push(Some(plan.observer()));
            } else {
                observers.proofs.push(None);
            }
        }
        let Ok((initialization, observers)) =
            check_package_initialization_observing(&self.prepared, root, ordinals, observers)
        else { return; };
        let mut graph = LocalPeerGraph::default();
        let mut pending_peers = Vec::new();
        let mut pending_pairs = Vec::new();
        let mut authored = AuthoredShowSchedule::default();
        let mut first_authored_citation = None;
        let mut additional_authored_citations = Vec::new();
        let mut implicit_authored = ImplicitAuthoredShows::default();
        let mut arbitrary_groups = HashMap::<_, UnusedModuleProofGroup<'_, '_>>::new();
        for ((index, plan), observer) in plans.iter().zip(observers.proofs) {
            let Some(&proof_module) = initialization.ordinals.get(plan.proof_source) else { continue; };
            if initialization.visits[proof_module] != InitializationVisit::Completed
                || initialization.visits[plan.target.0] != InitializationVisit::Completed
            {
                continue;
            }
            if let Some(measure) = plan.typed.measure {
                let used_verified_equality = observers.induction.as_ref()
                    .is_some_and(|token| token.target == plan.target && token.used);
                if !initialization.reached.contains(&plan.target) || used_verified_equality {
                    let checked = independently_checked.take()
                        .unwrap_or_else(|| plan.verify_unused_induction(measure));
                    outcomes[*index] = module_proof_outcome(checked);
                }
                continue;
            }
            if initialization.reached.contains(&plan.target) {
                if let Some(observer) = observer {
                    outcomes[*index] = module_proof_outcome(observer.outcome());
                }
            } else if !matches!(plan.typed.goal, TypedProofGoal::Literal { .. } | TypedProofGoal::Conjunction(_) | TypedProofGoal::Disjunction(_) | TypedProofGoal::Universal { .. } | TypedProofGoal::UniversalNotEqual(_) | TypedProofGoal::UniversalConjunction(_) | TypedProofGoal::Existential { .. } | TypedProofGoal::EntryWitness { .. } | TypedProofGoal::EntryComparison { .. } | TypedProofGoal::MixedEntryConjunction { .. } | TypedProofGoal::ConjunctiveExistential(_) | TypedProofGoal::DependentWitness(_)) {
                continue;
            } else if plan.uses_direct_goal() {
                let key = (plan.target, ExactGuardGroup(plan.filter.as_deref()));
                arbitrary_groups.entry(key).or_default().direct.push((*index, plan));
            } else if let Some(peer) = plan.direct_peer_use() {
                pending_peers.push((*index, plan, peer, None));
            } else {
                if let Some(peers) = plan.two_peer_uses() {
                    pending_pairs.push((*index, plan, peers));
                    continue;
                }
                match plan.citation_then_show() {
                    Ok(Some((peer, claim))) => {
                        pending_peers.push((*index, plan, peer, Some(claim)));
                        continue;
                    }
                    Err(error) => {
                        outcomes[*index] = module_proof_outcome(Err(error));
                        continue;
                    }
                    Ok(None) => {}
                }
                let (operator, bound, proof) = match plan.prepare_arbitrary_show() {
                    Ok(proof) => proof,
                    Err(error) => {
                        outcomes[*index] = module_proof_outcome(Err(error));
                        continue;
                    }
                };
                if matches!(proof, IntegerShowProof::Conditional { .. }) {
                    outcomes[*index] = module_proof_outcome(plan.verify_prepared_arbitrary_show(operator, bound, &proof));
                    continue;
                }
                let Some(relation) = IntegerRelation::from_operator(operator) else { continue; };
                authored.push(AuthoredShowCheck {
                    destination: AuthoredShowDestination::Selected(*index), plan, relation, bound, proof,
                });
            }
        }
        let mut cited_bounds = PeerPredicateBounds::default();
        if !pending_peers.is_empty() || !pending_pairs.is_empty() {
            for plan in pending_peers.iter().map(|(_, plan, _, _)| *plan)
                .chain(pending_pairs.iter().map(|(_, plan, _)| *plan))
            {
                selected.sources.get_mut(plan.proof_source).expect("requested peer consumer source")
                    .catalog.get_or_insert_with(|| {
                        Box::new(LocalPeerCatalog::new(plan.proof_source, &self.proofs[plan.proof_source]))
                    });
            }
            let mut admitted_roots = Vec::new();
            for (index, plan, peer, claim) in pending_peers {
                let Some(catalog) = selected.catalog(plan.proof_source) else { continue; };
                let Ok(declaration) = catalog.declaration(peer) else { continue; };
                let requested = graph.enroll(self, selected, plans, catalog.source, declaration);
                let Some(immediate) = graph.nodes[requested].plan() else { continue; };
                if !plan.same_peer_target(immediate) || immediate.peer_literal_goal().is_none() {
                    continue;
                }
                let strict_entry = if ExactGuardGroup(plan.filter.as_deref()) == ExactGuardGroup(immediate.filter.as_deref()) {
                    None
                } else if immediate.steps.items.is_empty() {
                    let Some(entry) = plan.closed_entry_admitted_by_peer(immediate) else { continue; };
                    Some(entry)
                } else {
                    continue;
                };
                graph.activate(requested);
                admitted_roots.push((index, plan, requested, strict_entry, claim));
            }
            for (index, plan, [first, second]) in pending_pairs {
                let Some(catalog) = selected.catalog(plan.proof_source) else { continue; };
                let (Ok(first), Ok(second)) = (catalog.declaration(first), catalog.declaration(second))
                    else { continue; };
                let first = graph.enroll(self, selected, plans, catalog.source, first);
                let second = graph.enroll(self, selected, plans, catalog.source, second);
                if first == second {
                    continue;
                }
                let direct = |index: usize| {
                    let peer = graph.nodes[index].plan()?;
                    if !peer.steps.items.is_empty() || !plan.same_peer_context(peer) {
                        return None;
                    }
                    peer.peer_literal_goal()
                };
                let (Some(first_goal), Some(second_goal)) = (direct(first), direct(second)) else { continue; };
                let Some(goal) = plan.peer_literal_goal() else { continue; };
                let entails = cited_bounds.bounds_for(plan.typed.result_type).is_some_and(|(min, max)| {
                    super::symbolic_integer::integer_predicate_pair_implies(first_goal, second_goal, goal, (min, max))
                });
                let key = (plan.target, ExactGuardGroup(plan.filter.as_deref()));
                arbitrary_groups.entry(key).or_default().paired_uses.push(PairedPeerUse {
                    index, consumer: plan, leaves: [first, second], entails,
                });
            }
            graph.resolve_edges(self, selected, plans);
            graph.resolve_terminals();
            let integer_equality = IntegerRelation::from_operator(&BinaryOperator::Equal).expect("checked integer equality");
            for (index, plan, requested, strict_entry, claim) in admitted_roots {
                let Some(leaf) = graph.terminal(requested) else { continue; };
                let Some((relation, bound)) = leaf.peer_literal_goal() else { continue; };
                if !leaf.steps.items.is_empty() {
                    if strict_entry.is_some() || claim.is_some() {
                        continue;
                    }
                    let TypedProofGoal::Literal { relation: requested_relation, bound: requested, .. } =
                        &plan.typed.goal else { continue; };
                    let equality = relation == integer_equality;
                    if !equality && !cited_bounds.implies(
                        plan.typed.result_type, (relation, bound), (*requested_relation, requested),
                    ) {
                        continue;
                    }
                    let destination = if let Some(selected_leaf) = selected.request(leaf.proof_source, leaf.name) {
                        Some(AuthoredShowDestination::Selected(*selected_leaf))
                    } else {
                        let (destination, check) = implicit_authored.cite((leaf.proof_source, leaf.name), || {
                            let (operator, bound, proof) = leaf.prepare_arbitrary_show().ok()?;
                            let relation = IntegerRelation::from_operator(operator)?;
                            if matches!(proof, IntegerShowProof::Conditional { .. }) {
                                return None;
                            }
                            Some((relation, bound, proof))
                        });
                        if let (Some(destination), Some((relation, bound, proof))) = (destination, check) {
                            authored.push(AuthoredShowCheck { destination, plan: leaf, relation, bound, proof });
                        }
                        destination
                    };
                    if let Some(destination) = destination {
                        let citation = (index, destination, equality.then_some((*requested_relation, bound, requested)));
                        if first_authored_citation.is_none() {
                            first_authored_citation = Some(citation);
                        } else {
                            additional_authored_citations.push(citation);
                        }
                    }
                    continue;
                }
                let implication_checked = strict_entry.is_some() && relation != integer_equality
                    && selected.request(leaf.proof_source, leaf.name).is_none();
                if implication_checked {
                    let TypedProofGoal::Literal { relation: requested_relation, bound: requested, .. } =
                        &plan.typed.goal else { continue; };
                    if !cited_bounds.implies(
                        plan.typed.result_type, (relation, bound), (*requested_relation, requested),
                    ) {
                        continue;
                    }
                }
                let mixed_implies_claims = claim.map(|claim| {
                    let premise = (relation, bound);
                    let mut implies = |relation, bound: &BigInt| {
                        cited_bounds.implies(plan.typed.result_type, premise, (relation, bound))
                    };
                    match claim {
                        PeerShowClaims::One(show_relation, show_bound) => {
                            let TypedProofGoal::Literal { relation: requested_relation, bound: requested, .. } =
                                &plan.typed.goal else { return false; };
                            let ensures = (*requested_relation, requested);
                            implies(show_relation, show_bound.as_ref())
                                && ((show_relation, show_bound.as_ref()) == ensures || implies(ensures.0, ensures.1))
                        }
                        PeerShowClaims::Many(output) => output.implied_by(premise, implies),
                    }
                });
                if strict_entry.is_some() && mixed_implies_claims == Some(false) {
                    continue;
                }
                let leaf_guard = ExactGuardGroup(leaf.filter.as_deref());
                arbitrary_groups.entry((plan.target, leaf_guard)).or_default().peer_uses.push(UnusedPeerUse {
                    index, consumer: plan, name: leaf.name, relation, bound, strict_entry, implication_checked, mixed_implies_claims,
                });
            }
        }
        authored.verify(outcomes, &mut implicit_authored);
        for (index, destination, equality) in first_authored_citation.into_iter().chain(additional_authored_citations) {
            if destination.proved(outcomes, &implicit_authored) {
                outcomes[index] = if equality.is_some_and(|(relation, bound, requested)| !relation.accepts(bound.cmp(requested))) {
                    ScalarModuleProofOutcome::Refuted
                } else {
                    ScalarModuleProofOutcome::Proved
                };
            }
        }
        for ((_, guard), group) in arbitrary_groups {
            let body = group.direct.first().map(|entry| entry.1)
                .or_else(|| group.peer_uses.first().map(|entry| entry.consumer))
                .or_else(|| group.paired_uses.first().map(|entry| entry.consumer))
                .expect("nonempty unused proof group").typed.body;
            if !valid_arbitrary_proof_body(body) {
                continue;
            }
            let mut peers = HashMap::new();
            for UnusedPeerUse { index, consumer, name, relation, bound, strict_entry, implication_checked, mixed_implies_claims } in group.peer_uses {
                let identity = (consumer.proof_source, name);
                let (_, _, uses): &mut (_, _, Vec<_>) = peers.entry(identity)
                    .or_insert_with(|| (relation, bound, Vec::new()));
                uses.push((index, consumer, strict_entry, implication_checked, mixed_implies_claims));
            }
            for pair in &group.paired_uses {
                if pair.entails {
                    for leaf in pair.leaves {
                        let peer = graph.nodes[leaf].plan().expect("admitted direct paired proof");
                        let TypedProofGoal::Literal { relation, bound, .. } = &peer.typed.goal else {
                            unreachable!("admitted direct paired literal goal");
                        };
                        peers.entry((peer.proof_source, peer.name)).or_insert_with(|| (*relation, bound, Vec::new()));
                    }
                }
            }
            match observe_unused_module_proof_returns(body, guard.0) {
                Some(Ok(ProofCoverage::Applicable(paths))) => {
                    let mut checked_peers = HashMap::new();
                    let mut first_strict_entry = None;
                    let mut other_strict_entries = HashMap::new();
                    let equality = IntegerRelation::from_operator(&BinaryOperator::Equal).expect("checked integer equality");
                    for (identity, (relation, bound, consumers)) in peers {
                        let proof = paths.iter().map(|path| path.output_satisfies(relation, bound))
                            .find(|holds| *holds != Some(true)).unwrap_or(Some(true));
                        let checked = if proof == Some(true) {
                            for (index, consumer, strict_entry, implication_checked, mixed_implies_claims) in consumers {
                                let legacy = match mixed_implies_claims {
                                    Some(true) => None,
                                    Some(false) => continue,
                                    None => {
                                        let TypedProofGoal::Literal { relation: requested_relation, bound: requested, .. } =
                                            &consumer.typed.goal else { continue; };
                                        if relation != equality && !implication_checked
                                            && !cited_bounds.implies(
                                                consumer.typed.result_type, (relation, bound), (*requested_relation, requested),
                                            )
                                        {
                                            continue;
                                        }
                                        Some((*requested_relation, requested))
                                    }
                                };
                                if let Some(key) = strict_entry {
                                    let returns = match first_strict_entry {
                                        Some((first, result)) if first == key => result,
                                        Some(_) => *other_strict_entries.entry(key).or_insert_with(|| {
                                            strict_peer_entry_returns(&paths, key)
                                        }),
                                        None => {
                                            let result = strict_peer_entry_returns(&paths, key);
                                            first_strict_entry = Some((key, result));
                                            result
                                        }
                                    };
                                    if !returns {
                                        continue;
                                    }
                                }
                                outcomes[index] = if legacy.is_some_and(|(requested_relation, requested)|
                                    relation == equality && !requested_relation.accepts(bound.cmp(requested)))
                                {
                                    ScalarModuleProofOutcome::Refuted
                                } else {
                                    ScalarModuleProofOutcome::Proved
                                };
                            }
                            CheckedPeerLeaf::Proven
                        } else if proof == Some(false) {
                            CheckedPeerLeaf::Refuted
                        } else {
                            CheckedPeerLeaf::Unavailable
                        };
                        checked_peers.insert(identity, checked);
                    }
                    for pair in group.paired_uses {
                        if pair.entails && pair.leaves.iter().all(|&leaf| {
                            let peer = graph.nodes[leaf].plan().expect("admitted direct paired proof");
                            matches!(checked_peers.get(&(peer.proof_source, peer.name)), Some(CheckedPeerLeaf::Proven))
                        })
                        {
                            outcomes[pair.index] = ScalarModuleProofOutcome::Proved;
                        }
                    }
                    for (index, plan) in group.direct {
                        if let Some(peer) = checked_peers.get(&(plan.proof_source, plan.name)) {
                            outcomes[index] = match peer {
                                CheckedPeerLeaf::Proven => ScalarModuleProofOutcome::Proved,
                                CheckedPeerLeaf::Refuted => ScalarModuleProofOutcome::Refuted,
                                CheckedPeerLeaf::Unavailable => ScalarModuleProofOutcome::Unsupported,
                            };
                            continue;
                        }
                        let goal = &plan.typed.goal;
                        let holds = if let TypedProofGoal::EntryComparison { slot, relation } = goal {
                            ImmutableProofOutputComparison::prepare(body, *slot, plan.typed.result_type, *relation)
                                .and_then(|entry| entry.holds(&paths, None))
                        } else if let TypedProofGoal::EntryWitness { slot, relation, domain } = goal {
                            ImmutableProofOutputComparison::prepare(body, *slot, plan.typed.result_type, *relation)
                                .and_then(|entry| entry.holds_existential_witness(&paths, domain, None))
                                .and_then(|verified| verified.then_some(true))
                        } else if let TypedProofGoal::ConjunctiveExistential(goal) = goal {
                            goal.holds_over(&paths, body, plan.typed.result_type)
                        } else if let TypedProofGoal::MixedEntryConjunction { output, entry } = goal {
                            body.boundary.checker_parameter_bindings.as_deref()
                                .and_then(|formals| evaluate_mixed_entry_output(&paths, formals, output, entry))
                        } else {
                            let mut holds = Some(true);
                            for path in &paths {
                                holds = goal.holds_on_returned_path(path);
                                if holds != Some(true) {
                                    break;
                                }
                            }
                            holds
                        };
                        outcomes[index] = match holds {
                            Some(true) => ScalarModuleProofOutcome::Proved,
                            Some(false) => ScalarModuleProofOutcome::Refuted,
                            None => ScalarModuleProofOutcome::Unsupported,
                        };
                    }
                }
                Some(Ok(ProofCoverage::NoApplicableInputs)) if guard.0.is_some() => {
                    for pair in group.paired_uses {
                        outcomes[pair.index] = ScalarModuleProofOutcome::NoApplicableInputs;
                    }
                    for (index, _) in group.direct {
                        outcomes[index] = ScalarModuleProofOutcome::NoApplicableInputs;
                    }
                    for (_, _, consumers) in peers.into_values() {
                        for (index, _, strict_entry, _, _) in consumers {
                            if strict_entry.is_none() {
                                outcomes[index] = ScalarModuleProofOutcome::NoApplicableInputs;
                            }
                        }
                    }
                }
                _ => (),
            }
        }
    }
}

#[cfg(feature = "internal-checker")]
impl PlannedModuleProof<'_> {
    fn verify_induction_return_witness(
        &self, measure: TypedInductionRank<'_>,
    ) -> Result<RankedInductionOutcome<'_>, ProjectCheck> {
        let proof_case = prepare_decrement_proof_steps(self.name, &self.steps, &self.typed, measure)?;
        let filter = self.filter.as_deref().unwrap_or_default();
        let domains = induction_entry_domain(measure, &proof_case, filter)?;
        let failure = check_canonical_induction_returns(
            self.function, self.canonical_target, &self.typed, measure, &proof_case, &domains.descending,
        )?;
        if domains.descending.is_empty() {
            return Ok(RankedInductionOutcome::NoApplicable);
        }
        if let Some(failure) = failure {
            if let Some(active) = domains.base_witness(&proof_case)? {
                let (earlier, entry) = if proof_case.reset.is_some() {
                    (Some(active), domains.descending.extremes().ok_or(ProjectCheck::Unsupported)?.0.clone())
                } else {
                    let earlier = domains.earlier.as_ref()
                        .and_then(IntegerSet::extremes).map(|(first, _)| first.clone());
                    (earlier, active)
                };
                let rejected = if failure.show_failed {
                    RankedBaseRejection::AuthoredShow(proof_case.base_show)
                } else {
                    let goal = child(&child(self.proof, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
                    RankedBaseRejection::Goal(wosy_syntax::byte_span(&goal))
                };
                return Ok(RankedInductionOutcome::Refuted(RankedBaseWitness {
                    proof_source: self.proof_source, return_source: &self.typed.target.source,
                    entry_type: measure.descending.0.ty.clone(), earlier, entry, returned: failure.returned,
                    return_span: failure.return_span, rejected,
                }));
            }
        }
        check_induction_entry_domain(&domains, &proof_case)
            .map(RankedInductionOutcome::Premise)
    }

    fn verify_unused_induction(&self, measure: TypedInductionRank<'_>) -> Result<ProjectCheck, ProjectCheck> {
        match self.verify_induction_return_witness(measure)? {
            RankedInductionOutcome::NoApplicable => Ok(ProjectCheck::NoApplicableInputs),
            RankedInductionOutcome::Premise(InductionPremise::Established | InductionPremise::Unreachable) =>
                Ok(ProjectCheck::Proved),
            RankedInductionOutcome::Premise(InductionPremise::NotStrict | InductionPremise::CalleeWhereUnproved) =>
                Err(ProjectCheck::Unsupported),
            RankedInductionOutcome::Refuted(_) => Ok(ProjectCheck::Refuted),
        }
    }

    fn uses_direct_goal(&self) -> bool {
        self.steps.items.is_empty() || matches!(self.typed.goal, TypedProofGoal::Existential { .. } | TypedProofGoal::EntryWitness { .. } | TypedProofGoal::ConjunctiveExistential(_) | TypedProofGoal::DependentWitness(_))
    }

    fn observer(&self) -> SelectedProofObserver<'_> {
        let mode = if self.uses_direct_goal() {
            ActualProofMode::Direct(checked_integer_guard_slots::<true>(self.typed.body, self.filter.as_deref().unwrap_or_default()))
        } else if matches!(self.typed.goal, TypedProofGoal::Literal { .. }) {
            ActualProofMode::Pending(self)
        } else {
            ActualProofMode::Direct(None)
        };
        SelectedProofObserver { mode, goal: &self.typed.goal, coverage: ActualProofCoverage::Unseen }
    }

    fn same_peer_target(&self, peer: &PlannedModuleProof<'_>) -> bool {
        self.proof_source == peer.proof_source && self.target == peer.target && self.canonical_target == peer.canonical_target
    }

    fn same_peer_context(&self, peer: &PlannedModuleProof<'_>) -> bool {
        self.same_peer_target(peer) && ExactGuardGroup(self.filter.as_deref()) == ExactGuardGroup(peer.filter.as_deref())
    }

    fn peer_literal_goal(&self) -> Option<(IntegerRelation, &BigInt)> {
        if self.typed.measure.is_some() || is_null_literal_name(&self.typed.output)
            || self.typed.input_names.iter().any(|name| is_null_literal_name(name))
        {
            return None;
        }
        let TypedProofGoal::Literal { relation, bound, .. } = &self.typed.goal
            else { return None; };
        Some((*relation, bound))
    }

    fn closed_entry_admitted_by_peer<'a>(&'a self, leaf: &PlannedModuleProof<'_>) -> Option<StrictPeerDomain<'a>> {
        let boundary = &self.typed.body.boundary;
        let bindings = boundary.checker_parameter_bindings.as_deref()?;
        let filter = self.filter.as_deref()?;
        let range = |binding: &'a ReferenceBindingId, ty: &ScalarType, lower: &'a super::checker_flow::IntegerPredicate,
                     upper: &'a super::checker_flow::IntegerPredicate| {
            if &lower.binding != binding || &upper.binding != binding
                || &lower.ty != ty || &upper.ty != ty || lower.bound > upper.bound
            {
                return None;
            }
            Some(ImmutableEntryRange { binding, lower: &lower.bound, upper: &upper.bound })
        };
        let (first, second) = match (bindings, boundary.parameters.as_slice(), filter) {
            ([binding], [parameter], [CheckerPredicate::Integer(entry)]) if entry.operator == BinaryOperator::Equal =>
                (range(binding, &parameter.ty, entry, entry)?, None),
            ([binding], [parameter], [CheckerPredicate::Integer(left), CheckerPredicate::Integer(right)]) => {
                let (lower, upper) = match (&left.operator, &right.operator) {
                    (BinaryOperator::GreaterEqual, BinaryOperator::LessEqual) => (left, right),
                    (BinaryOperator::LessEqual, BinaryOperator::GreaterEqual) => (right, left),
                    _ => return None,
                };
                (range(binding, &parameter.ty, lower, upper)?, None)
            }
            ([first, second], [first_param, second_param], [CheckerPredicate::Integer(left), CheckerPredicate::Integer(right)])
                if left.operator == BinaryOperator::Equal && right.operator == BinaryOperator::Equal =>
            {
                let (a, b) = if &left.binding == first { (left, right) } else { (right, left) };
                (range(first, &first_param.ty, a, a)?, Some(range(second, &second_param.ty, b, b)?))
            }
            _ => return None,
        };
        let by_binding = |binding: &ReferenceBindingId| {
            if binding == first.binding {
                Some((first, &boundary.parameters[0].ty))
            } else {
                second.filter(|entry| binding == entry.binding).map(|entry| (entry, &boundary.parameters[1].ty))
            }
        };
        for predicate in leaf.filter.as_deref().unwrap_or_default() {
            let admits = match predicate {
                CheckerPredicate::Boolean(value) => *value,
                CheckerPredicate::Integer(atom) => {
                    let (entry, ty) = by_binding(&atom.binding)?;
                    &atom.ty == ty && IntegerRelation::from_operator(&atom.operator)?
                        .accepts_closed(entry.lower, entry.upper, &atom.bound)
                }
                CheckerPredicate::EntryDomain { binding, values } => {
                    let (entry, _) = by_binding(binding)?;
                    values.covers_closed(entry.lower, entry.upper)
                }
                _ => return None,
            };
            if !admits {
                return None;
            }
        }
        Some(StrictPeerDomain { first, second })
    }

    fn direct_peer_use(&self) -> Option<&str> {
        let [item] = self.steps.items.as_slice() else { return None; };
        if self.steps.terminated_items.as_slice() != [true] {
            return None;
        }
        self.peer_use_step(item)
    }

    fn two_peer_uses(&self) -> Option<[&str; 2]> {
        let [first, second] = self.steps.items.as_slice() else { return None; };
        if self.steps.terminated_items.as_slice() != [true, true] {
            return None;
        }
        let second = self.peer_use_step(second)?;
        let first = self.peer_use_step(first)?;
        (first != second).then_some([first, second])
    }

    fn citation_then_show(&self) -> Result<Option<(&str, PeerShowClaims<'_>)>, ProjectCheck> {
        let [use_step, show_steps @ ..] = self.steps.items.as_slice() else { return Ok(None); };
        let [true, terminated @ ..] = self.steps.terminated_items.as_slice() else { return Ok(None); };
        if show_steps.is_empty() || terminated.len() != show_steps.len() {
            return Ok(None);
        }
        let Some(peer) = self.peer_use_step(use_step) else { return Ok(None); };
        if let [show] = show_steps {
            let (relation, bound) = self.checked_peer_show(show, terminated[0])?;
            return Ok(Some((peer, PeerShowClaims::One(relation, bound))));
        }
        let Some((relation, bound)) = self.peer_literal_goal() else { return Err(ProjectCheck::Unsupported); };
        let mut constraints = IntegerShowConstraints::default();
        constraints.add(relation, Cow::Borrowed(bound));
        for (step, terminated) in show_steps.iter().zip(terminated) {
            let (relation, bound) = self.checked_peer_show(step, *terminated)?;
            constraints.add(relation, bound);
        }
        Ok(Some((peer, PeerShowClaims::Many(Box::new(constraints)))))
    }

    fn checked_peer_show<'a>(
        &self, step: &'a ScalarBlockItem, terminated: bool,
    ) -> Result<(IntegerRelation, Cow<'a, BigInt>), ProjectCheck> {
        let claim = proof_show_expression(step, terminated)?;
        let comparison = IntegerComparison::from_expression(claim).ok_or(ProjectCheck::Unsupported)?;
        let (_, relation, bound) = checked_literal_output_goal_borrowed(
            comparison, &self.typed.output, &self.typed.input_names, self.typed.result_type,
        )?;
        Ok((relation, bound))
    }

    fn peer_use_step<'a>(&self, item: &'a ScalarBlockItem) -> Option<&'a str> {
        let ScalarBlockItem::Expression(ScalarExpression::Call {
            receiver: Some(core), name: use_step, type_arguments: use_types, arguments: use_arguments, ..
        }) = item else { return None; };
        if core != "core" || use_step != "use" || !use_types.is_empty() {
            return None;
        }
        let boundary = &self.typed.body.boundary;
        let bindings = boundary.checker_parameter_bindings.as_deref()?;
        if bindings.len() != self.typed.input_names.len() || !boundary.parameters.iter().all(|slot| is_integer_type(&slot.ty)) {
            return None;
        }
        if !matches!(self.typed.goal, TypedProofGoal::Literal { .. })
            || !matches!(self.typed.input_names.len(), 1 | 2)
            || is_null_literal_name(&self.typed.output)
            || self.typed.input_names.iter().any(|name| is_null_literal_name(name))
        {
            return None;
        }
        let [ScalarExpression::Call {
            receiver: None, name: peer, type_arguments, arguments, ..
        }] = use_arguments.as_slice() else { return None; };
        if peer == self.name || !type_arguments.is_empty() || arguments.len() != self.typed.input_names.len()
            || !arguments.iter().zip(&self.typed.input_names).all(|(argument, input)| {
                matches!(argument, ScalarExpression::Name { name, .. } if name == input)
            })
        {
            return None;
        }
        Some(peer)
    }

    fn has_flat_show_steps(&self) -> bool {
        self.steps.items.len() == self.steps.terminated_items.len()
            && self.steps.items.iter().zip(&self.steps.terminated_items).all(|(item, &terminated)|
                proof_show_expression(item, terminated).is_ok())
    }

    fn prepare_arbitrary_show(&self) -> Result<(&BinaryOperator, &BigInt, IntegerShowProof<'_>), ProjectCheck> {
        let TypedProofPostcondition { body, result_type, input_names, output, goal, .. } = &self.typed;
        let TypedProofGoal::Literal { operator, bound, .. } = goal else {
            return Err(ProjectCheck::Unsupported);
        };
        if !valid_arbitrary_proof_body(body) {
            return Err(ProjectCheck::Unsupported);
        }
        let proof = proof_show_plan::<false, true>(
            &self.steps, input_names, output, result_type, body,
            Some((operator, bound)),
        )?;
        Ok((operator, bound, proof))
    }

    fn verify_prepared_arbitrary_show(
        &self, operator: &BinaryOperator, bound: &BigInt, proof: &IntegerShowProof<'_>,
    ) -> Result<ProjectCheck, ProjectCheck> {
        let TypedProofPostcondition { target, body, .. } = &self.typed;
        let checked = check_integer_return_bound_with_shows(body, operator, bound, self.filter.as_deref(), proof);
        match checked {
            Some(Ok(ProofCoverage::Applicable(true))) => Ok(ProjectCheck::Proved),
            Some(Ok(ProofCoverage::Applicable(false))) => Ok(ProjectCheck::Refuted),
            Some(Ok(ProofCoverage::NoApplicableInputs)) => Ok(ProjectCheck::NoApplicableInputs),
            Some(Err(failure)) => Err(project_failure(body, &target.source, failure)),
            None => Err(ProjectCheck::Unsupported),
        }
    }
}

#[cfg(feature = "internal-checker")]
fn summary_elision_safe(body: &FunctionCallSites) -> bool {
    body.assignments.is_empty() && body.allocation_length_calls.is_empty()
        && body.structure.checker_contextual_occurrences.is_empty()
        && body.structure.checker_contextual_read_locals.is_empty()
        && body.structure.checker_directives.values().all(|directive| directive.operation == CheckerOperation::Require)
}

#[cfg(feature = "internal-checker")]
fn supported_calling_proof_body(body: &FunctionCallSites, output: &ScalarType) -> bool {
    body.boundary.parameters.len() == 1 && !body.structure.checker_integer_call_locals.is_empty()
        && supported_integer_continuation(body, output)
}

#[cfg(feature = "internal-checker")]
fn valid_arbitrary_proof_body(body: &FunctionCallSites) -> bool {
    body.structure.checker_directives.is_empty() && body.call_occurrences.is_empty()
        && body.allocation_length_calls.is_empty() && body.assignments.is_empty()
}

#[cfg(feature = "internal-checker")]
struct CheckedIntegerGuardSlots<'a> {
    arity: usize,
    always_rejects: bool,
    comparisons: Vec<ImportedEntryGuard<'a>>,
    domain: Option<(usize, &'a IntegerSet)>,
}

#[cfg(feature = "internal-checker")]
impl CheckedIntegerGuardSlots<'_> {
    fn accepts(&self, arguments: &[&BigInt]) -> bool {
        self.arity == arguments.len() && !self.always_rejects
            && self.domain.is_none_or(|(slot, values)| values.contains(arguments[slot]))
            && self.comparisons.iter().all(|comparison| {
                arguments.get(comparison.slot).is_some_and(|argument| comparison.relation.accepts((*argument).cmp(comparison.bound)))
            })
    }
}

#[cfg(feature = "internal-checker")]
fn checked_integer_guard_slots<'a, const ALLOW_DOMAIN: bool>(
    body: &FunctionCallSites,
    filter: &'a [CheckerPredicate],
) -> Option<CheckedIntegerGuardSlots<'a>> {
    let parameters = body.boundary.parameters.as_slice();
    let bindings = body.boundary.checker_parameter_bindings.as_deref()?;
    if !matches!(parameters.len(), 1 | 2) || parameters.len() != bindings.len()
        || !parameters.iter().all(|parameter| is_integer_type(&parameter.ty))
    {
        return None;
    }
    let mut guard = CheckedIntegerGuardSlots { arity: parameters.len(), always_rejects: false, comparisons: Vec::new(), domain: None };
    for predicate in filter {
        match predicate {
            CheckerPredicate::Boolean(value) => guard.always_rejects |= !value,
            CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return None,
            CheckerPredicate::EntryDomain { binding, values } => {
                if !ALLOW_DOMAIN || guard.domain.is_some() {
                    return None;
                }
                let slot = bindings.iter().position(|known| known == binding)?;
                guard.domain = Some((slot, values));
            }
            CheckerPredicate::Integer(predicate) => {
                let slot = bindings.iter().position(|binding| binding == &predicate.binding)?;
                guard.comparisons.push(ImportedEntryGuard {
                    slot,
                    relation: IntegerRelation::from_operator(&predicate.operator)?,
                    bound: &predicate.bound,
                });
            }
        }
    }
    Some(guard)
}

#[cfg(feature = "internal-checker")]
enum ActualProofMode<'a> {
    Direct(Option<CheckedIntegerGuardSlots<'a>>),
    Pending(&'a PlannedModuleProof<'a>),
    Authored(CheckedIntegerGuardSlots<'a>, Box<ActualFlatShow<'a>>),
    Invalid(Box<ProjectCheck>),
}

#[cfg(feature = "internal-checker")]
struct SelectedProofObserver<'a> {
    mode: ActualProofMode<'a>,
    goal: &'a TypedProofGoal,
    coverage: ActualProofCoverage,
}

#[cfg(feature = "internal-checker")]
enum ActualProofCoverage {
    Unseen,
    Inapplicable,
    Proved,
    Refuted,
    Unsupported,
}

#[cfg(feature = "internal-checker")]
impl SelectedProofObserver<'_> {
    fn outcome(self) -> Result<ProjectCheck, ProjectCheck> {
        if let ActualProofMode::Invalid(error) = self.mode {
            return Err(*error);
        }
        match self.coverage {
            ActualProofCoverage::Proved => Ok(ProjectCheck::Proved),
            ActualProofCoverage::Refuted => Ok(ProjectCheck::Refuted),
            ActualProofCoverage::Inapplicable => Ok(ProjectCheck::NoApplicableInputs),
            ActualProofCoverage::Unsupported | ActualProofCoverage::Unseen => Err(ProjectCheck::Unsupported),
        }
    }

    fn prepare_actual_shows<'a>(plan: &'a PlannedModuleProof<'a>, arity: usize) -> ActualProofMode<'a> {
        let starts_with_show = plan.steps.items.first().zip(plan.steps.terminated_items.first())
            .is_some_and(|(item, &terminated)| proof_show_expression(item, terminated).is_ok());
        let guarded = matches!(plan.steps.items.as_slice(), [
            ScalarBlockItem::Expression(ScalarExpression::If { .. })
        ]);
        if !starts_with_show && !guarded {
            return ActualProofMode::Direct(None);
        }
        let Some(guard) = checked_integer_guard_slots::<true>(plan.typed.body, plan.filter.as_deref().unwrap_or_default())
            .filter(|guard| guard.arity == arity)
        else {
            return ActualProofMode::Direct(None);
        };
        let prepared = plan.prepare_arbitrary_show().and_then(|(operator, bound, proof)| {
            let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
            let bindings = plan.typed.body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
            ActualFlatShow::admit(proof, relation, bound, bindings, arity).ok_or(ProjectCheck::Unsupported)
        });
        match prepared {
            Ok(show) => ActualProofMode::Authored(guard, Box::new(show)),
            Err(error @ (ProjectCheck::Invalid(_) | ProjectCheck::InvalidProof)) => ActualProofMode::Invalid(Box::new(error)),
            Err(_) => ActualProofMode::Direct(None),
        }
    }

    fn record(&mut self, arguments: &[&BigInt], value: &BigInt) {
        let (guard, shows) = match &self.mode {
            ActualProofMode::Direct(Some(guard)) => (guard, None),
            ActualProofMode::Authored(guard, show) => (guard, Some(show.as_ref())),
            ActualProofMode::Pending(plan) => {
                self.mode = Self::prepare_actual_shows(*plan, arguments.len());
                return self.record(arguments, value);
            }
            ActualProofMode::Direct(None) | ActualProofMode::Invalid(_) => {
                self.coverage = ActualProofCoverage::Unsupported;
                return;
            }
        };
        if guard.arity != arguments.len() {
            self.coverage = ActualProofCoverage::Unsupported;
            return;
        }
        if matches!(self.coverage, ActualProofCoverage::Unsupported | ActualProofCoverage::Refuted) {
            return;
        }
        if guard.accepts(arguments) {
            let holds = match shows {
                Some(show) => show.holds(value, arguments),
                None => match self.goal {
                    TypedProofGoal::Literal { relation, bound, .. } | TypedProofGoal::Universal { relation, bound } =>
                        Some(relation.accepts(value.cmp(bound))),
                    TypedProofGoal::Existential { relation, witness, witness_in_domain } =>
                        (*witness_in_domain && relation.accepts(value.cmp(witness))).then_some(true),
                    TypedProofGoal::EntryWitness { slot, relation, domain } => {
                        let Some(candidate) = arguments.get(*slot) else {
                            self.coverage = ActualProofCoverage::Unsupported;
                            return;
                        };
                        (relation.accepts(value.cmp(candidate)) && domain.contains(candidate)).then_some(true)
                    },
                    TypedProofGoal::UniversalNotEqual(domain) => Some(!domain.covers_closed(value, value)),
                    TypedProofGoal::UniversalConjunction(goal) => Some(match goal {
                        UniversalConjunction::Vacuous => true,
                        UniversalConjunction::Impossible => false,
                        UniversalConjunction::Output(output) => output.holds_value(value),
                        UniversalConjunction::OutputAvoiding(bounds) =>
                            bounds.0.holds_value(value) && !bounds.1.covers_closed(value, value),
                    }),
                    TypedProofGoal::DependentWitness(witness) => match witness {
                        DependentWitness::Unavailable => None,
                        DependentWitness::Comparison(relation, bound) => relation.accepts(value.cmp(bound)).then_some(true),
                        DependentWitness::Avoiding(domain) => (!domain.covers_closed(value, value)).then_some(true),
                        DependentWitness::Output(output) => output.holds_value(value).then_some(true),
                        DependentWitness::OutputAvoiding(bounds) =>
                            (bounds.0.holds_value(value) && !bounds.1.covers_closed(value, value)).then_some(true),
                    },
                    TypedProofGoal::ConjunctiveExistential(goal) => goal.holds_at(value, arguments),
                    TypedProofGoal::Disjunction(allowed) => Some(allowed.contains(value)),
                    TypedProofGoal::DeferredDisjunction(_) => None,
                    TypedProofGoal::MixedEntryConjunction { output, entry } =>
                        holds_actual_mixed_entry(output, entry, value, arguments),
                    TypedProofGoal::Conjunction(atoms) => Some(atoms.iter().all(|(relation, bound)| relation.accepts(value.cmp(bound)))),
                    TypedProofGoal::EntryComparison { slot, relation } =>
                        arguments.get(*slot).map(|entry| relation.accepts(value.cmp(entry))),
                },
            };
            self.coverage = match holds {
                Some(true) => ActualProofCoverage::Proved,
                Some(false) => ActualProofCoverage::Refuted,
                None => ActualProofCoverage::Unsupported,
            };
        } else if matches!(self.coverage, ActualProofCoverage::Unseen) {
            self.coverage = ActualProofCoverage::Inapplicable;
        }
    }
}

#[cfg(feature = "internal-checker")]
struct VerifiedInductionExactReturn<'a> {
    target: (usize, usize),
    result_type: &'a ScalarType,
    value: &'a BigInt,
    where_guard: CheckedIntegerGuardSlots<'a>,
    used: bool,
}

#[cfg(feature = "internal-checker")]
impl<'a> VerifiedInductionExactReturn<'a> {
    fn verify(plan: &'a PlannedModuleProof<'_>, rank: TypedInductionRank<'_>)
        -> (Result<ProjectCheck, ProjectCheck>, Option<Self>)
    {
        let checked = plan.verify_unused_induction(rank);
        let token = (|| {
            if !matches!(checked, Ok(ProjectCheck::Proved)) {
                return None;
            }
            let TypedProofGoal::Literal { operator: BinaryOperator::Equal, bound, .. } = &plan.typed.goal else {
                return None;
            };
            Some(Self {
                target: plan.target, result_type: plan.typed.result_type, value: bound,
                where_guard: checked_integer_guard_slots::<true>(plan.typed.body, plan.filter.as_deref().unwrap_or_default())?,
                used: false,
            })
        })();
        (checked, token)
    }

    fn consume(
        &mut self, target: PreparedCallable, arguments: &[&BigInt], expected: &ScalarType,
    ) -> Option<BigInt> {
        if self.target != target || self.result_type != expected || !self.where_guard.accepts(arguments) {
            return None;
        }
        self.used = true;
        Some(self.value.clone())
    }
}

#[cfg(feature = "internal-checker")]
struct ActualProofBatch<'a> {
    targets: HashMap<(usize, usize), Vec<usize>>,
    proofs: Vec<Option<SelectedProofObserver<'a>>>,
    induction: Option<VerifiedInductionExactReturn<'a>>,
}

#[cfg(feature = "internal-checker")]
impl ActualIntegerObserver for ActualProofBatch<'_> {
    const ENABLED: bool = true;

    fn proved_recursive_return(
        &mut self, target: (usize, usize), arguments: &[&BigInt], expected: &ScalarType,
    ) -> Option<BigInt> {
        self.induction.as_mut()?.consume(target, arguments, expected)
    }

    fn observe(&mut self, target: (usize, usize), arguments: &[&BigInt], value: &BigInt) {
        if let Some(proofs) = self.targets.get(&target) {
            for &proof in proofs {
                if let Some(observer) = self.proofs[proof].as_mut() {
                    observer.record(arguments, value);
                }
            }
        }
    }
}

fn proof_target_tokens(declaration: &CstNode) -> Result<(Option<CstToken>, CstToken), ProjectCheck> {
    let target = child(declaration, SyntaxKind::ProofTarget)?;
    let qualified = child(&target, SyntaxKind::QualifiedCall)?;
    let mut identifiers = qualified.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Identifier);
    let first = identifiers.next().ok_or(ProjectCheck::InvalidProof)?;
    match (identifiers.next(), identifiers.next()) {
        (None, None) => Ok((None, first)),
        (Some(member), None) => Ok((Some(first), member)),
        _ => Err(ProjectCheck::InvalidProof),
    }
}

#[cfg(test)]
fn check_proof_fragment(
    prepared: &PreparedCheckerProject,
    target_source: &SourceIdentity,
    fragment: &ProofFragmentParse,
) -> ProjectCheck {
    check_proof_fragment_impl(prepared, target_source, fragment, false)
        .map(|(outcome, _)| outcome)
        .unwrap_or_else(|error| error)
}

#[cfg(test)]
fn verify_return_bound(
    prepared: &PreparedCheckerProject,
    target_source: &SourceIdentity,
    fragment: &ProofFragmentParse,
) -> Option<VerifiedReturnBound> {
    check_proof_fragment_impl(prepared, target_source, fragment, true)
        .ok()
        .and_then(|(_, bound)| bound)
}

#[cfg(test)]
fn check_proof_fragment_impl(
    prepared: &PreparedCheckerProject,
    target_source: &SourceIdentity,
    fragment: &ProofFragmentParse,
    retain_bound: bool,
) -> Result<(ProjectCheck, Option<VerifiedReturnBound>), ProjectCheck> {
    let fragment = fragment.as_result();
    if !fragment.is_valid() {
        return Err(ProjectCheck::InvalidProof);
    }
    let root = fragment.canonical_cst();
    let declaration = child(&child(&root.root, SyntaxKind::Root)?, SyntaxKind::ProofDecl)?;
    let summary_source = retain_bound.then_some(&root.source)
        .filter(|source| !prepared.parts().0.modules.iter().any(|module| &module.source == *source))
        .map(ProofSummaryOrigin::DetachedFragment);
    let (receiver, name) = proof_target_tokens(&declaration)?;
    if receiver.is_some() {
        return Err(ProjectCheck::Unsupported);
    }
    let function = select_checked_root(prepared.parts().0, target_source, name.text())?;
    check_selected_proof(prepared, function, &declaration, &root.source, summary_source)
}

#[cfg(feature = "internal-checker")]
enum ConjunctiveWitness {
    Literal(BigInt, std::cell::OnceCell<bool>),
    Formal(usize),
}

#[cfg(feature = "internal-checker")]
struct ConjunctiveExistential {
    relation: IntegerRelation,
    output: IntegerShowConstraints<'static>,
    domain: IntegerSet,
    witness: ConjunctiveWitness,
}

#[cfg(feature = "internal-checker")]
impl ConjunctiveExistential {
    fn holds_at(&self, value: &BigInt, arguments: &[&BigInt]) -> Option<bool> {
        match &self.witness {
            ConjunctiveWitness::Literal(candidate, admitted) => (self.relation.accepts(value.cmp(candidate))
                && self.output.holds_value(value)
                && *admitted.get_or_init(|| self.domain.contains(candidate))).then_some(true),
            ConjunctiveWitness::Formal(slot) => {
                let candidate = arguments.get(*slot)?;
                (self.relation.accepts(value.cmp(candidate)) && self.output.holds_value(value)
                    && self.domain.contains(candidate)).then_some(true)
            }
        }
    }

    fn holds_over(
        &self, paths: &[super::checker_flow::IntegerReturningPath<'_>], body: &FunctionCallSites, ty: &ScalarType,
    ) -> Option<bool> {
        match &self.witness {
            ConjunctiveWitness::Literal(candidate, admitted) => {
                for path in paths {
                    if !path.output_satisfies_existential_literal(self.relation, candidate, &self.output)? {
                        return None;
                    }
                }
                admitted.get_or_init(|| self.domain.contains(candidate)).then_some(true)
            }
            ConjunctiveWitness::Formal(slot) => ImmutableProofOutputComparison::prepare(body, *slot, ty, self.relation)?
                .holds_existential_witness(paths, &self.domain, Some(&self.output))?.then_some(true),
        }
    }
}

#[cfg(feature = "internal-checker")]
enum UniversalConjunction {
    Vacuous,
    Impossible,
    Output(Box<IntegerShowConstraints<'static>>),
    OutputAvoiding(Box<(IntegerShowConstraints<'static>, IntegerSet)>),
}

#[cfg(feature = "internal-checker")]
enum DependentWitness {
    Unavailable,
    Comparison(IntegerRelation, BigInt),
    Avoiding(IntegerSet),
    Output(Box<IntegerShowConstraints<'static>>),
    OutputAvoiding(Box<(IntegerShowConstraints<'static>, IntegerSet)>),
}

enum TypedProofGoal {
    Literal {
        operator: BinaryOperator,
        relation: IntegerRelation,
        bound: BigInt,
    },
    Conjunction(Vec<(IntegerRelation, BigInt)>),
    Disjunction(IntegerSet),
    #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
    DeferredDisjunction([(IntegerRelation, BigInt); 2]),
    Universal { relation: IntegerRelation, bound: BigInt },
    #[cfg(feature = "internal-checker")]
    UniversalNotEqual(IntegerSet),
    #[cfg(feature = "internal-checker")]
    UniversalConjunction(UniversalConjunction),
    Existential { relation: IntegerRelation, witness: BigInt, witness_in_domain: bool },
    #[cfg(feature = "internal-checker")]
    EntryWitness { slot: usize, relation: IntegerRelation, domain: IntegerSet },
    #[cfg(feature = "internal-checker")]
    ConjunctiveExistential(Box<ConjunctiveExistential>),
    #[cfg(feature = "internal-checker")]
    DependentWitness(DependentWitness),
    EntryComparison { slot: usize, relation: IntegerRelation },
    #[cfg(feature = "internal-checker")]
    MixedEntryConjunction { output: IntegerShowConstraints<'static>, entry: [Option<IntegerRelation>; 2] },
}

#[cfg(feature = "internal-checker")]
impl TypedProofGoal {
    fn holds_on_returned_path(&self, path: &super::checker_flow::IntegerReturningPath<'_>) -> Option<bool> {
        match self {
            Self::Literal { relation, bound, .. } | Self::Universal { relation, bound } =>
                path.output_satisfies(*relation, bound),
            Self::Existential { relation, witness, witness_in_domain } => {
                if *witness_in_domain && path.output_satisfies(*relation, witness)? { Some(true) } else { None }
            }
            Self::Disjunction(allowed) => path.output_within(allowed),
            Self::UniversalNotEqual(domain) => path.output_disjoint_from(domain),
            Self::DependentWitness(witness) => match witness {
                DependentWitness::Unavailable => None,
                DependentWitness::Comparison(relation, bound) => path.output_satisfies(*relation, bound)?.then_some(true),
                DependentWitness::Avoiding(domain) => path.output_disjoint_from(domain)?.then_some(true),
                DependentWitness::Output(output) => path.output_satisfies_constraints(output)?.then_some(true),
                DependentWitness::OutputAvoiding(bounds) =>
                    path.output_satisfies_constraints_avoiding(&bounds.0, &bounds.1)?.then_some(true),
            },
            Self::UniversalConjunction(goal) => match goal {
                UniversalConjunction::Vacuous => Some(true),
                UniversalConjunction::Impossible => Some(false),
                UniversalConjunction::Output(output) => path.output_satisfies_constraints(output),
                UniversalConjunction::OutputAvoiding(bounds) => path.output_satisfies_constraints_avoiding(&bounds.0, &bounds.1),
            },
            Self::Conjunction(atoms) => {
                for (relation, bound) in atoms {
                    if !path.output_satisfies(*relation, bound)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Self::EntryComparison { .. } | Self::DeferredDisjunction(_) | Self::MixedEntryConjunction { .. } | Self::EntryWitness { .. } | Self::ConjunctiveExistential(_) => None,
        }
    }
}

fn checked_literal_output_goal(
    comparison: IntegerComparison<'_>, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<(BinaryOperator, IntegerRelation, BigInt), ProjectCheck> {
    let (operator, relation, bound) = checked_literal_output_goal_borrowed(comparison, output, inputs, ty)?;
    Ok((operator.clone(), relation, bound.into_owned()))
}

fn checked_literal_output_goal_borrowed<'a>(
    comparison: IntegerComparison<'a>, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<(&'a BinaryOperator, IntegerRelation, std::borrow::Cow<'a, BigInt>), ProjectCheck> {
    if comparison.name != output {
        return Err(if inputs.iter().any(|input| input == comparison.name) {
            ProjectCheck::Unsupported
        } else {
            ProjectCheck::InvalidProof
        });
    }
    if !integer_literal_fits_type(comparison.bound.as_ref(), ty) {
        return Err(ProjectCheck::InvalidProof);
    }
    let relation = IntegerRelation::from_operator(comparison.operator).ok_or(ProjectCheck::Unsupported)?;
    Ok((comparison.operator, relation, comparison.bound))
}

fn typed_universal_output_goal(
    forall: &CstNode,
    ensures: &ScalarExpression,
    output: &str,
    inputs: &[String],
    output_type: &ScalarType,
    #[cfg_attr(not(feature = "internal-checker"), allow(unused_variables))] allow_equality: bool,
) -> Result<TypedProofGoal, ProjectCheck> {
    let (ty, binder) = typed_quantifier_binder(forall, output, inputs, output_type)?;
    let ScalarExpression::Binary { operator, left, right, .. } = ensures else {
        return Err(ProjectCheck::Unsupported);
    };
    if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output)
        || !matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == &binder)
    {
        return Err(ProjectCheck::Unsupported);
    }
    #[cfg(feature = "internal-checker")]
    if allow_equality && matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual) {
        let domain = typed_quantifier_domain(forall, &binder, inputs, &ty)?;
        if domain.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        if *operator == BinaryOperator::NotEqual {
            return Ok(TypedProofGoal::UniversalNotEqual(domain));
        }
        return Ok(match domain.into_singleton() {
            Some(bound) => TypedProofGoal::Universal {
                relation: IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?, bound,
            },
            None => TypedProofGoal::UniversalConjunction(UniversalConjunction::Impossible),
        });
    }
    let lower = match operator {
        BinaryOperator::Less | BinaryOperator::LessEqual => true,
        BinaryOperator::Greater | BinaryOperator::GreaterEqual => false,
        _ => return Err(ProjectCheck::Unsupported),
    };
    let domain = typed_quantifier_domain(forall, &binder, inputs, &ty)?;
    let (minimum, maximum) = domain.extremes().ok_or(ProjectCheck::Unsupported)?;
    let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
    let bound = if lower { minimum } else { maximum }.clone();
    Ok(TypedProofGoal::Universal { relation, bound })
}

#[cfg(feature = "internal-checker")]
fn typed_named_universal_conjunction(
    forall: &CstNode, ensures: &ScalarExpression, output: &str, inputs: &[String], output_type: &ScalarType,
) -> Result<TypedProofGoal, ProjectCheck> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let (ty, binder) = typed_quantifier_binder(forall, output, inputs, output_type)?;
    let mut binder_relation: Option<IntegerRelation> = None;
    let mut output_bounds = IntegerShowConstraints::default();
    let mut saw_literal = false;
    visit_output_conjuncts(ensures, |atom| {
        if let ScalarExpression::Binary { operator, left, right, .. } = atom {
            if matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == &binder) {
                if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output) {
                    return Err(ProjectCheck::Unsupported);
                }
                let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
                binder_relation = Some(binder_relation.map_or(relation, |known| known.intersection(relation)));
                return Ok(());
            }
        }
        let (relation, bound) = typed_output_atom(atom, output, inputs, &ty)?;
        output_bounds.add(relation, Cow::Owned(bound));
        saw_literal = true;
        Ok(())
    })?;
    let relation = binder_relation.ok_or(ProjectCheck::Unsupported)?;
    let domain = typed_quantifier_domain(forall, &binder, inputs, &ty)?;
    let Some((minimum, maximum)) = domain.extremes() else {
        return Ok(TypedProofGoal::UniversalConjunction(UniversalConjunction::Vacuous));
    };
    let bound = match (relation.accepts(Less), relation.accepts(Greater), relation.accepts(Equal)) {
        (true, false, _) => minimum,
        (false, true, _) => maximum,
        (false, false, true) if minimum == maximum => minimum,
        (false, false, _) => return Ok(TypedProofGoal::UniversalConjunction(UniversalConjunction::Impossible)),
        (true, true, false) => {
            return if saw_literal {
                output_bounds.normalize();
                Ok(TypedProofGoal::UniversalConjunction(UniversalConjunction::OutputAvoiding(Box::new((output_bounds, domain)))))
            } else {
                Ok(TypedProofGoal::UniversalNotEqual(domain))
            };
        }
        _ => return Err(ProjectCheck::Unsupported),
    };
    output_bounds.add(relation, Cow::Owned(bound.clone()));
    output_bounds.normalize();
    Ok(TypedProofGoal::UniversalConjunction(UniversalConjunction::Output(Box::new(output_bounds))))
}

#[cfg(feature = "internal-checker")]
fn typed_named_repeated_universal<const DEPENDENT: bool>(
    foralls: impl Iterator<Item = CstNode>, ensures: &ScalarExpression,
    output: &str, inputs: &[String], ty: &ScalarType, dependent: Option<(&CstNode, &ScalarBlock)>,
) -> Result<TypedProofGoal, ProjectCheck> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    if !matches!(ensures, ScalarExpression::Binary { operator: BinaryOperator::And, .. }) {
        return Err(ProjectCheck::Unsupported);
    }
    let declarations = foralls.map(|forall| {
        let (_, binder) = typed_quantifier_binder(&forall, output, inputs, ty)?;
        Ok((forall, binder))
    }).collect::<Result<Vec<_>, ProjectCheck>>()?;
    let mut names = HashMap::with_capacity(declarations.len());
    for (index, (_, name)) in declarations.iter().enumerate() {
        if names.insert(name.as_str(), index).is_some() {
            return Err(ProjectCheck::InvalidProof);
        }
    }
    let witness = if DEPENDENT {
        let (exists, steps) = dependent.ok_or(ProjectCheck::Unsupported)?;
        let (_, existential) = typed_quantifier_binder(exists, output, inputs, ty)?;
        if names.contains_key(existential.as_str()) {
            return Err(ProjectCheck::InvalidProof);
        }
        let ScalarExpression::Name { name, .. } = authored_witness_argument(steps)? else {
            return Err(ProjectCheck::Unsupported);
        };
        let chosen = *names.get(name.as_str()).ok_or(ProjectCheck::Unsupported)?;
        Some((exists, existential, chosen))
    } else {
        None
    };
    let mut saw_existential = false;
    let mut relations = vec![None::<IntegerRelation>; declarations.len()];
    let mut output_bounds = IntegerShowConstraints::default();
    let mut has_output_bounds = false;
    visit_output_conjuncts(ensures, |atom| {
        if let ScalarExpression::Binary { operator, left, right, .. } = atom {
            if let ScalarExpression::Name { name, .. } = right.as_ref() {
                let slot = if let Some((_, binder, chosen)) = &witness {
                    if name == binder {
                        saw_existential = true;
                        *chosen
                    } else {
                        *names.get(name.as_str()).ok_or(ProjectCheck::Unsupported)?
                    }
                } else {
                    *names.get(name.as_str()).ok_or(ProjectCheck::Unsupported)?
                };
                if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output) {
                    return Err(ProjectCheck::Unsupported);
                }
                let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
                relations[slot] = Some(relations[slot].map_or(relation, |known| known.intersection(relation)));
                return Ok(());
            }
        }
        let (relation, bound) = typed_output_atom(atom, output, inputs, ty)?;
        output_bounds.add(relation, Cow::Owned(bound));
        has_output_bounds = true;
        Ok(())
    })?;
    if DEPENDENT && !saw_existential {
        return Err(ProjectCheck::Unsupported);
    }
    let admission = witness.map(|(exists, binder, chosen)| {
        let domain = typed_quantifier_domain(exists, &binder, inputs, ty)?;
        Ok::<_, ProjectCheck>((chosen, domain))
    }).transpose()?;
    let mut unavailable = false;
    let mut impossible = false;
    let mut forbidden = OwnedIntegerUnion::default();
    for (slot, ((forall, binder), relation)) in declarations.into_iter().zip(relations).enumerate() {
        let domain = typed_quantifier_domain(&forall, &binder, inputs, ty)?;
        let (Some((minimum, maximum)), Some(relation)) = (domain.extremes(), relation) else {
            unavailable = true;
            continue;
        };
        if unavailable || impossible {
            continue;
        }
        let lower = match (relation.accepts(Less), relation.accepts(Greater), relation.accepts(Equal)) {
            (true, false, _) => Some(true),
            (false, true, _) => Some(false),
            (false, false, true) if minimum == maximum => Some(true),
            (false, false, _) => { impossible = true; continue; }
            (true, true, false) => None,
            _ => return Err(ProjectCheck::Unsupported),
        };
        if let Some((chosen, existential)) = &admission {
            if slot == *chosen && !domain.is_subset_of(existential) {
                impossible = true;
                continue;
            }
        }
        let Some(lower) = lower else {
            forbidden.push(domain);
            continue;
        };
        let bound = domain.into_extremum(lower).ok_or(ProjectCheck::Unsupported)?;
        output_bounds.add(relation, Cow::Owned(bound));
        has_output_bounds = true;
    }
    if unavailable {
        return Err(ProjectCheck::Unsupported);
    }
    if impossible {
        return Ok(if DEPENDENT {
            TypedProofGoal::DependentWitness(DependentWitness::Unavailable)
        } else {
            TypedProofGoal::UniversalConjunction(UniversalConjunction::Impossible)
        });
    }
    if let Some(forbidden) = forbidden.finish() {
        if !has_output_bounds {
            return Ok(if DEPENDENT {
                TypedProofGoal::DependentWitness(DependentWitness::Avoiding(forbidden))
            } else {
                TypedProofGoal::UniversalNotEqual(forbidden)
            });
        }
        output_bounds.normalize();
        let bounds = Box::new((output_bounds, forbidden));
        return Ok(if DEPENDENT {
            TypedProofGoal::DependentWitness(DependentWitness::OutputAvoiding(bounds))
        } else {
            TypedProofGoal::UniversalConjunction(UniversalConjunction::OutputAvoiding(bounds))
        });
    }
    output_bounds.normalize();
    let output = Box::new(output_bounds);
    Ok(if DEPENDENT {
        TypedProofGoal::DependentWitness(DependentWitness::Output(output))
    } else {
        TypedProofGoal::UniversalConjunction(UniversalConjunction::Output(output))
    })
}

fn typed_quantifier_binder(
    quantifier: &CstNode, output: &str, inputs: &[String], output_type: &ScalarType,
) -> Result<(ScalarType, String), ProjectCheck> {
    let ty = super::derive_type(&child(quantifier, SyntaxKind::TypeSpec)?);
    if &ty != output_type || !is_integer_type(&ty) {
        return Err(ProjectCheck::Unsupported);
    }
    let mut names = identifiers(quantifier);
    let binder = names.next().ok_or(ProjectCheck::InvalidProof)?;
    if names.next().is_some() || binder == output || inputs.contains(&binder)
        || is_null_literal_name(&binder) || is_null_literal_name(output)
    {
        return Err(ProjectCheck::InvalidProof);
    }
    Ok((ty, binder))
}

fn typed_quantifier_domain(
    quantifier: &CstNode, binder: &str, inputs: &[String], ty: &ScalarType,
) -> Result<IntegerSet, ProjectCheck> {
    let mut domain = IntegerSet::from_type(ty).ok_or(ProjectCheck::Unsupported)?;
    if let Some(guard) = quantifier.children().find(|node| node.kind() == SyntaxKind::ProofQuantifierGuard) {
        let condition = derive_expression(&child(&guard, SyntaxKind::Expression)?);
        let mut pending = vec![&condition];
        while let Some(expression) = pending.pop() {
            match expression {
                ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } => {
                    pending.push(right);
                    pending.push(left);
                }
                ScalarExpression::Boolean { value: false, .. } => domain = IntegerSet::default(),
                ScalarExpression::Boolean { value: true, .. } => {}
                expression => {
                    let comparison = IntegerComparison::from_expression(expression).ok_or(ProjectCheck::Unsupported)?;
                    let (operator, _, bound) = checked_literal_output_goal(comparison, binder, inputs, ty)?;
                    domain = domain.keep(&operator, &bound).ok_or(ProjectCheck::Unsupported)?;
                }
            }
        }
    }
    Ok(domain)
}

fn authored_witness_argument(steps: &ScalarBlock) -> Result<&ScalarExpression, ProjectCheck> {
    let [ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver, name, arguments, type_arguments, ..
    })] = steps.items.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    if receiver.as_deref() != Some("core") || name != "witness"
        || !type_arguments.is_empty() || steps.terminated_items.as_slice() != [true]
    {
        return Err(ProjectCheck::Unsupported);
    }
    let [argument] = arguments.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    Ok(argument)
}

#[cfg(feature = "internal-checker")]
fn typed_existential_conjuncts(
    predicate: &ScalarExpression, binder: &str, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<(IntegerRelation, IntegerShowConstraints<'static>), ProjectCheck> {
    let mut relation: Option<IntegerRelation> = None;
    let mut output_bounds = IntegerShowConstraints::default();
    visit_output_conjuncts(predicate, |atom| {
        if let ScalarExpression::Binary { operator, left, right, .. } = atom {
            if matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == binder) {
                if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output) {
                    return Err(ProjectCheck::Unsupported);
                }
                let atom = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
                relation = Some(relation.map_or(atom, |known| known.intersection(atom)));
                return Ok(());
            }
        }
        let (atom, bound) = typed_output_atom(atom, output, inputs, ty)?;
        output_bounds.add(atom, Cow::Owned(bound));
        Ok(())
    })?;
    let relation = relation.ok_or(ProjectCheck::Unsupported)?;
    Ok((relation, output_bounds))
}

#[cfg(feature = "internal-checker")]
fn dependent_witness_conjunction(
    relation: IntegerRelation, mut output: IntegerShowConstraints<'static>,
    universal: IntegerSet, existential: &IntegerSet,
) -> Result<DependentWitness, ProjectCheck> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let lower = match (relation.accepts(Less), relation.accepts(Greater), relation.accepts(Equal)) {
        (true, false, _) => true,
        (false, true, _) => false,
        (false, false, true) if universal.singleton_value().is_some() => true,
        (false, false, _) => return Ok(DependentWitness::Unavailable),
        (true, true, false) => {
            if !universal.is_subset_of(existential) {
                return Ok(DependentWitness::Unavailable);
            }
            output.normalize();
            return Ok(DependentWitness::OutputAvoiding(Box::new((output, universal))));
        }
        _ => return Err(ProjectCheck::Unsupported),
    };
    if !universal.is_subset_of(existential) {
        return Ok(DependentWitness::Unavailable);
    }
    let bound = universal.into_extremum(lower).ok_or(ProjectCheck::Unsupported)?;
    output.add(relation, Cow::Owned(bound));
    output.normalize();
    Ok(DependentWitness::Output(Box::new(output)))
}

#[cfg(feature = "internal-checker")]
fn typed_named_dependent_witness(
    forall: &CstNode, exists: &CstNode, steps: &ScalarBlock, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<TypedProofGoal, ProjectCheck> {
    let (_, universal) = typed_quantifier_binder(forall, output, inputs, ty)?;
    let (_, existential) = typed_quantifier_binder(exists, output, inputs, ty)?;
    if universal == existential {
        return Err(ProjectCheck::InvalidProof);
    }
    let predicate = derive_expression(&child(exists, SyntaxKind::Expression)?);
    if matches!(&predicate, ScalarExpression::Binary { operator: BinaryOperator::And, .. }) {
        let (relation, output_bounds) = typed_existential_conjuncts(&predicate, &existential, output, inputs, ty)?;
        if !matches!(authored_witness_argument(steps)?, ScalarExpression::Name { name, .. } if name == &universal) {
            return Err(ProjectCheck::Unsupported);
        }
        let universal_domain = typed_quantifier_domain(forall, &universal, inputs, ty)?;
        let existential_domain = typed_quantifier_domain(exists, &existential, inputs, ty)?;
        if universal_domain.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        return Ok(TypedProofGoal::DependentWitness(dependent_witness_conjunction(
            relation, output_bounds, universal_domain, &existential_domain,
        )?));
    }
    let ScalarExpression::Binary { operator, left, right, .. } = predicate else {
        return Err(ProjectCheck::Unsupported);
    };
    if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output)
        || !matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == &existential)
        || !matches!(authored_witness_argument(steps)?, ScalarExpression::Name { name, .. } if name == &universal)
    {
        return Err(ProjectCheck::Unsupported);
    }
    let relation = IntegerRelation::from_operator(&operator).ok_or(ProjectCheck::Unsupported)?;
    let universal_domain = typed_quantifier_domain(forall, &universal, inputs, ty)?;
    let existential_domain = typed_quantifier_domain(exists, &existential, inputs, ty)?;
    if universal_domain.is_empty() {
        return Err(ProjectCheck::Unsupported);
    }
    let witness = if (operator == BinaryOperator::Equal && universal_domain.singleton_value().is_none())
        || !universal_domain.is_subset_of(&existential_domain)
    {
        DependentWitness::Unavailable
    } else {
        match operator {
            BinaryOperator::Equal => universal_domain.into_singleton()
                .map_or(DependentWitness::Unavailable, |bound| DependentWitness::Comparison(relation, bound)),
            BinaryOperator::NotEqual => DependentWitness::Avoiding(universal_domain),
            BinaryOperator::Less | BinaryOperator::LessEqual | BinaryOperator::Greater | BinaryOperator::GreaterEqual => {
                let lower = matches!(operator, BinaryOperator::Less | BinaryOperator::LessEqual);
                let bound = universal_domain.into_extremum(lower).ok_or(ProjectCheck::Unsupported)?;
                DependentWitness::Comparison(relation, bound)
            }
            _ => return Err(ProjectCheck::Unsupported),
        }
    };
    Ok(TypedProofGoal::DependentWitness(witness))
}

fn typed_existential_output_goal(
    exists: &CstNode, steps: &ScalarBlock, output: &str, inputs: &[String], ty: &ScalarType,
    #[cfg_attr(not(feature = "internal-checker"), allow(unused_variables))] body: &FunctionCallSites,
    #[cfg_attr(not(feature = "internal-checker"), allow(unused_variables))] allow_named_entry_facts: bool,
) -> Result<TypedProofGoal, ProjectCheck> {
    let (ty, binder) = typed_quantifier_binder(exists, output, inputs, ty)?;
    let predicate = derive_expression(&child(exists, SyntaxKind::Expression)?);
    let simple_relation = || {
        let ScalarExpression::Binary { operator, left, right, .. } = &predicate else {
            return Err(ProjectCheck::Unsupported);
        };
        if *operator != BinaryOperator::Equal && (!cfg!(feature = "internal-checker") || !allow_named_entry_facts) {
            return Err(ProjectCheck::Unsupported);
        }
        let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
        if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output)
            || !matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == &binder)
        {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(relation)
    };
    let argument = authored_witness_argument(steps)?;
    #[cfg(feature = "internal-checker")]
    if allow_named_entry_facts && matches!(&predicate, ScalarExpression::Binary { operator: BinaryOperator::And, .. }) {
        let (relation, mut output_bounds) =
            typed_existential_conjuncts(&predicate, &binder, output, inputs, &ty)?;
        let witness = if let ScalarExpression::Name { name, .. } = argument {
            ConjunctiveWitness::Formal(checked_existential_formal(name, inputs, body, &ty)?)
        } else {
            let value = concrete_integer_literal(argument).ok_or(ProjectCheck::Unsupported)?;
            if !integer_literal_fits_type(value.as_ref(), &ty) {
                return Err(ProjectCheck::InvalidProof);
            }
            ConjunctiveWitness::Literal(value.into_owned(), std::cell::OnceCell::new())
        };
        output_bounds.normalize();
        let domain = typed_quantifier_domain(exists, &binder, inputs, &ty)?;
        return Ok(TypedProofGoal::ConjunctiveExistential(Box::new(ConjunctiveExistential {
            relation, output: output_bounds, domain, witness,
        })));
    }
    let relation = simple_relation()?;
    #[cfg(feature = "internal-checker")]
    if allow_named_entry_facts {
        if let ScalarExpression::Name { name, .. } = argument {
            let slot = checked_existential_formal(name, inputs, body, &ty)?;
            let domain = typed_quantifier_domain(exists, &binder, inputs, &ty)?;
            return Ok(TypedProofGoal::EntryWitness { slot, relation, domain });
        }
    }
    let witness = concrete_integer_literal(argument).ok_or(ProjectCheck::Unsupported)?;
    if !integer_literal_fits_type(witness.as_ref(), &ty) {
        return Err(ProjectCheck::InvalidProof);
    }
    let domain = typed_quantifier_domain(exists, &binder, inputs, &ty)?;
    let witness_in_domain = domain.contains(witness.as_ref());
    Ok(TypedProofGoal::Existential { relation, witness: witness.into_owned(), witness_in_domain })
}

#[cfg(feature = "internal-checker")]
fn checked_existential_formal(
    name: &str, inputs: &[String], body: &FunctionCallSites, ty: &ScalarType,
) -> Result<usize, ProjectCheck> {
    if is_null_literal_name(name) {
        return Err(ProjectCheck::InvalidProof);
    }
    let slot = inputs.iter().position(|input| input == name).ok_or(ProjectCheck::InvalidProof)?;
    let parameters = &body.boundary.parameters;
    let bindings = body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
    if !matches!(parameters.len(), 1 | 2) || bindings.len() != parameters.len()
        || parameters.get(slot).is_none_or(|parameter| &parameter.ty != ty)
    {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(slot)
}

struct TypedProofPostcondition<'a> {
    target: &'a ScalarModule,
    body: &'a FunctionCallSites,
    result_type: &'a ScalarType,
    input_names: Vec<String>,
    output: String,
    goal: TypedProofGoal,
    #[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
    measure: Option<TypedInductionRank<'a>>,
}

type TypedEntryMeasure<'a> = (&'a super::reference::ParameterSlot, &'a ReferenceBindingId);

#[derive(Clone, Copy)]
struct TypedInductionRank<'a> {
    earlier: Option<TypedEntryMeasure<'a>>,
    descending: TypedEntryMeasure<'a>,
}

fn typed_proof_decreases<'a>(
    declaration: &CstNode,
    input_names: &[String],
    body: &'a FunctionCallSites,
) -> Result<Option<TypedInductionRank<'a>>, ProjectCheck> {
    let Some(clause) = declaration.children().find(|node| node.kind() == SyntaxKind::ProofDecreases) else {
        return Ok(None);
    };
    let mut expressions = clause.children().filter(|node| node.kind() == SyntaxKind::Expression);
    let first = expressions.next().ok_or(ProjectCheck::InvalidProof)?;
    let second = expressions.next();
    if second.is_some() && (input_names.len() != 2 || expressions.next().is_some()) {
        return Err(ProjectCheck::Unsupported);
    }
    let scalar = |expression: &CstNode| -> Result<TypedEntryMeasure<'a>, ProjectCheck> {
        let ScalarExpression::Name { name, .. } = derive_expression(expression) else {
            return Err(ProjectCheck::Unsupported);
        };
        let slot = input_names.iter().position(|input| *input == name).ok_or(ProjectCheck::InvalidProof)?;
        let parameter = body.boundary.parameters.get(slot).ok_or(ProjectCheck::InvalidProof)?;
        if !is_integer_type(&parameter.ty) {
            return Err(if matches!(parameter.ty, ScalarType::F32 | ScalarType::F64) {
                ProjectCheck::Unsupported
            } else {
                ProjectCheck::InvalidProof
            });
        }
        let binding = body.boundary.checker_parameter_bindings.as_deref()
            .and_then(|bindings| bindings.get(slot)).ok_or(ProjectCheck::Unsupported)?;
        Ok((parameter, binding))
    };
    let first = scalar(&first)?;
    let rank = if let Some(second) = second {
        let second = scalar(&second)?;
        if first.0.ordinal != 0 || second.0.ordinal != 1 {
            return Err(ProjectCheck::Unsupported);
        }
        TypedInductionRank { earlier: Some(first), descending: second }
    } else {
        TypedInductionRank { earlier: None, descending: first }
    };
    Ok(Some(rank))
}

#[cfg(feature = "internal-checker")]
#[derive(Debug, Eq, PartialEq)]
enum InductionPremise {
    Established,
    Unreachable,
    NotStrict,
    CalleeWhereUnproved,
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
enum RankedInductionOutcome<'a> {
    NoApplicable,
    Premise(InductionPremise),
    Refuted(RankedBaseWitness<'a>),
}

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
struct RankedBaseWitness<'a> {
    proof_source: &'a SourceIdentity,
    return_source: &'a SourceIdentity,
    entry_type: ScalarType,
    earlier: Option<BigInt>,
    entry: BigInt,
    returned: &'a BigInt,
    return_span: ByteSpan,
    rejected: RankedBaseRejection,
}

#[cfg(feature = "internal-checker")]
#[derive(Debug, Eq, PartialEq)]
enum RankedBaseRejection {
    AuthoredShow(ByteSpan),
    Goal(ByteSpan),
}

#[cfg(feature = "internal-checker")]
struct CheckedRankedBaseFailure<'a> {
    returned: &'a BigInt,
    return_span: ByteSpan,
    show_failed: bool,
}

#[cfg(feature = "internal-checker")]
struct TypedInductionProofSteps<'a> {
    base_guard: CheckerPredicate,
    base_claim: Cow<'a, BigInt>,
    base_show: ByteSpan,
    decrement: &'a BigInt,
    reset: Option<Cow<'a, BigInt>>,
}

#[cfg(feature = "internal-checker")]
fn prepare_decrement_proof_steps<'a>(
    proof_name: &str,
    steps: &'a ScalarBlock,
    typed: &TypedProofPostcondition<'a>,
    rank: TypedInductionRank<'a>,
) -> Result<TypedInductionProofSteps<'a>, ProjectCheck> {
    if typed.input_names.len() != 1 + usize::from(rank.earlier.is_some()) {
        return Err(ProjectCheck::Unsupported);
    }
    let [ScalarBlockItem::Expression(ScalarExpression::If {
        condition, then_branch, else_branch, ..
    })] = steps.items.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    let [ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver: Some(core), name: use_step, type_arguments: use_types, arguments: use_arguments, ..
    })] = else_branch.items.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    if core != "core" || use_step != "use" || !use_types.is_empty()
        || else_branch.terminated_items.as_slice() != [true]
    {
        return Err(ProjectCheck::Unsupported);
    }
    let [ScalarExpression::Call {
        receiver: None, name: use_name, type_arguments, arguments, ..
    }] = use_arguments.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    if use_name != proof_name || !type_arguments.is_empty() {
        return Err(ProjectCheck::Unsupported);
    }
    let ((parameter, binding), argument, reset) = match (rank.earlier, arguments.as_slice()) {
        (None, [decrement]) => (rank.descending, decrement, None),
        (Some((earlier, _)), [ScalarExpression::Name { name, .. }, decrement])
            if name == &typed.input_names[earlier.ordinal] => (rank.descending, decrement, None),
        (Some(earlier), [decrement, suffix]) => {
            let reset = concrete_integer_literal(suffix)
                .filter(|value| integer_literal_fits_type(value.as_ref(), &rank.descending.0.ty))
                .ok_or(ProjectCheck::Unsupported)?;
            (earlier, decrement, Some(reset))
        }
        _ => return Err(ProjectCheck::Unsupported),
    };
    let entry = &typed.input_names[parameter.ordinal];
    let ScalarExpression::Binary { operator: BinaryOperator::Subtract, left, right, .. } = argument else {
        return Err(ProjectCheck::Unsupported);
    };
    let ScalarExpression::Integer { value: decrement, .. } = right.as_ref() else {
        return Err(ProjectCheck::Unsupported);
    };
    if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == entry)
        || decrement.sign() != num_bigint::Sign::Plus
        || !integer_literal_fits_type(decrement, &parameter.ty)
    {
        return Err(ProjectCheck::Unsupported);
    }
    let ([show_item @ ScalarBlockItem::Expression(show)], [terminated]) =
        (then_branch.items.as_slice(), then_branch.terminated_items.as_slice())
    else {
        return Err(ProjectCheck::Unsupported);
    };
    let claim = IntegerComparison::from_expression(proof_show_expression(show_item, *terminated)?)
        .ok_or(ProjectCheck::Unsupported)?;
    if claim.operator != &BinaryOperator::Equal || claim.name != typed.output {
        return Err(ProjectCheck::Unsupported);
    }
    if !integer_literal_fits_type(claim.bound.as_ref(), typed.result_type) {
        return Err(ProjectCheck::InvalidProof);
    }
    let base_claim = claim.bound;
    let base_show = super::typecheck::expression_span(show);
    let base_guard = proof_input_condition::<true>(condition, &typed.input_names, typed.body, false)?;
    if matches!(&base_guard, CheckerPredicate::Integer(predicate) if &predicate.binding != binding) {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(TypedInductionProofSteps { base_guard, base_claim, base_show, decrement, reset })
}

#[cfg(feature = "internal-checker")]
fn check_canonical_induction_returns<'a>(
    function: &ScalarFunction,
    target: &ResolvedCallTarget,
    typed: &TypedProofPostcondition<'a>,
    rank: TypedInductionRank<'_>,
    proof: &TypedInductionProofSteps<'_>,
    suffix_domain: &IntegerSet,
) -> Result<Option<CheckedRankedBaseFailure<'a>>, ProjectCheck> {
    use super::reference::{OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};
    use super::typecheck::expression_span;

    let (parameter, binding) = if proof.reset.is_some() {
        rank.earlier.ok_or(ProjectCheck::Unsupported)?
    } else {
        rank.descending
    };
    let body = typed.body;
    let structure = &body.structure;
    let (relation, bound) = match &typed.goal {
        TypedProofGoal::Literal { relation, bound, .. } | TypedProofGoal::Universal { relation, bound } => (relation, bound),
        _ => return Err(ProjectCheck::Unsupported),
    };
    if !body.allocation_length_calls.is_empty() {
        return Err(ProjectCheck::Unsupported);
    }
    let output_at = |block: super::reference::BlockOrdinal| {
        let [item] = structure.blocks.get(block.0)?.items.as_slice() else {
            return None;
        };
        match item.kind {
            BlockItemKind::FinalOutput(slot) if slot.output == 0
                && structure.checked_types.expressions.get(&slot.expression) == Some(typed.result_type) => Some(slot.expression),
            _ => None,
        }
    };
    let tail = output_at(structure.root).ok_or(ProjectCheck::Unsupported)?;
    let Some(ConditionalRegion::Value { condition, then_block, else_block, .. }) = body.conditionals.get(&tail) else {
        return Err(ProjectCheck::Unsupported);
    };
    if structure.checker_branches.get(condition) != Some(&proof.base_guard) {
        return Err(ProjectCheck::Unsupported);
    }

    let base_output = output_at(*then_block).ok_or(ProjectCheck::Unsupported)?;
    let base_occurrence = if proof.reset.is_some() {
        if let Some(ConditionalRegion::Value { condition, then_block, else_block, .. }) = body.conditionals.get(&base_output) {
            let Some(CheckerPredicate::Integer(predicate)) = structure.checker_branches.get(condition) else {
                return Err(ProjectCheck::Unsupported);
            };
            if &predicate.binding != rank.descending.1 || predicate.ty != rank.descending.0.ty {
                return Err(ProjectCheck::Unsupported);
            }
            let then_output = output_at(*then_block).filter(|slot| structure.checker_integer_literals.contains_key(slot))
                .ok_or(ProjectCheck::Unsupported)?;
            let else_output = output_at(*else_block).filter(|slot| structure.checker_integer_literals.contains_key(slot))
                .ok_or(ProjectCheck::Unsupported)?;
            let relation = IntegerRelation::from_operator(&predicate.operator).ok_or(ProjectCheck::Unsupported)?;
            if suffix_domain.every(relation, &predicate.bound) {
                then_output
            } else if suffix_domain.every(relation.complement(), &predicate.bound) {
                else_output
            } else {
                return Err(ProjectCheck::Unsupported);
            }
        } else {
            base_output
        }
    } else {
        base_output
    };
    let base = structure.checker_integer_literals.get(&base_occurrence).ok_or(ProjectCheck::Unsupported)?;

    let recursive = output_at(*else_block).ok_or(ProjectCheck::Unsupported)?;
    let [site] = body.call_occurrences.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    if site.ordinal != recursive || body.resolved_call_site(recursive).map(|site| &site.target) != Some(target) {
        return Err(ProjectCheck::Unsupported);
    }
    let signature = body.checked_call_signatures.get(&recursive).ok_or(ProjectCheck::Unsupported)?;
    let parameters_match = match (rank.earlier, signature.parameters.as_slice()) {
        (None, [input]) => input == &parameter.ty,
        (Some((earlier, _)), [first, second]) => first == &earlier.ty && second == &rank.descending.0.ty,
        _ => false,
    };
    if !parameters_match || !matches!(signature.outputs.outputs.as_slice(), [output] if &output.ty == typed.result_type)
    {
        return Err(ProjectCheck::Unsupported);
    }

    let source_tail = function.body.tail_conditional().ok_or(ProjectCheck::Unsupported)?;
    let mut source_outputs = source_tail.else_branch.final_expressions();
    let Some((source_call @ ScalarExpression::Call { arguments, .. }, _)) = source_outputs.next() else {
        return Err(ProjectCheck::Unsupported);
    };
    let (argument, unchanged, reset) = match (rank.earlier, proof.reset.as_ref(), arguments.as_slice()) {
        (None, None, [decrement]) => (decrement, None, None),
        (Some(earlier), None, [first, decrement]) => (decrement, Some((first, earlier)), None),
        (Some(_), Some(value), [decrement, suffix]) => (decrement, None, Some((suffix, value))),
        _ => return Err(ProjectCheck::Unsupported),
    };
    let ScalarExpression::Binary { operator: BinaryOperator::Subtract, left, right, .. }
        = argument else { return Err(ProjectCheck::Unsupported); };
    if source_outputs.next().is_some()
        || body.occurrences.get(tail.0).map(|node| node.span) != Some(source_tail.span)
        || body.occurrences.get(recursive.0).map(|node| node.span) != Some(expression_span(source_call))
        || !matches!(right.as_ref(), ScalarExpression::Integer { value, .. } if value == proof.decrement)
    {
        return Err(ProjectCheck::Unsupported);
    }
    let checked_operand = |parent, edge, expression, ty| {
        let OccurrenceNode::Expression(occurrence) = structure.edges.get(&(OccurrenceParent::Expression(parent), edge))? else {
            return None;
        };
        (body.occurrences.get(occurrence.0)?.span == expression_span(expression)
            && structure.checked_types.expressions.get(occurrence) == Some(ty)).then_some(*occurrence)
    };
    if let Some((expression, (earlier, expected))) = unchanged {
        let first = checked_operand(recursive, OccurrenceEdge::CallArgument(earlier.ordinal), expression, &earlier.ty)
            .ok_or(ProjectCheck::Unsupported)?;
        if !matches!(body.prepared_plain_name(first), Some(PlainNameIdentity::Lexical(entry)) if entry == expected) {
            return Err(ProjectCheck::Unsupported);
        }
    }
    if let Some((suffix, value)) = reset {
        let slot = rank.descending.0;
        checked_operand(recursive, OccurrenceEdge::CallArgument(slot.ordinal), suffix, &slot.ty)
            .ok_or(ProjectCheck::Unsupported)?;
        if concrete_integer_literal(suffix).as_deref() != Some(value.as_ref()) {
            return Err(ProjectCheck::Unsupported);
        }
    }
    let argument = checked_operand(recursive, OccurrenceEdge::CallArgument(parameter.ordinal), argument, &parameter.ty)
        .ok_or(ProjectCheck::Unsupported)?;
    let left = checked_operand(argument, OccurrenceEdge::BinaryLeft, left, &parameter.ty).ok_or(ProjectCheck::Unsupported)?;
    checked_operand(argument, OccurrenceEdge::BinaryRight, right, &parameter.ty).ok_or(ProjectCheck::Unsupported)?;
    if !matches!(body.prepared_plain_name(left), Some(PlainNameIdentity::Lexical(entry)) if entry == binding) {
        return Err(ProjectCheck::Unsupported);
    }
    let show_failed = base != proof.base_claim.as_ref();
    if !show_failed && relation.accepts(base.cmp(bound)) {
        return Ok(None);
    }
    let return_span = body.occurrences.get(base_occurrence.0).ok_or(ProjectCheck::Unsupported)?.span;
    Ok(Some(CheckedRankedBaseFailure { returned: base, return_span, show_failed }))
}

#[cfg(feature = "internal-checker")]
fn check_decrement_induction_premise(
    rank: TypedInductionRank<'_>,
    proof: &TypedInductionProofSteps<'_>,
    entry_filter: &[CheckerPredicate],
) -> Result<InductionPremise, ProjectCheck> {
    let domains = induction_entry_domain(rank, proof, entry_filter)?;
    check_induction_entry_domain(&domains, proof)
}

#[cfg(feature = "internal-checker")]
struct InductionEntryDomains {
    descending: IntegerSet,
    earlier: Option<IntegerSet>,
    minimum: BigInt,
}

#[cfg(feature = "internal-checker")]
impl InductionEntryDomains {
    fn decreasing(&self, proof: &TypedInductionProofSteps<'_>) -> Result<&IntegerSet, ProjectCheck> {
        if proof.reset.is_some() {
            self.earlier.as_ref().ok_or(ProjectCheck::Unsupported)
        } else {
            Ok(&self.descending)
        }
    }

    fn base_witness(&self, proof: &TypedInductionProofSteps<'_>) -> Result<Option<BigInt>, ProjectCheck> {
        let domain = self.decreasing(proof)?;
        Ok(match &proof.base_guard {
            CheckerPredicate::Boolean(true) => domain.extremes().map(|(first, _)| first.clone()),
            CheckerPredicate::Boolean(false) => None,
            CheckerPredicate::Integer(predicate) => domain.keep(&predicate.operator, &predicate.bound)
                .ok_or(ProjectCheck::Unsupported)?.extremes().map(|(first, _)| first.clone()),
            _ => return Err(ProjectCheck::Unsupported),
        })
    }
}

#[cfg(feature = "internal-checker")]
fn induction_entry_domain(
    rank: TypedInductionRank<'_>,
    proof: &TypedInductionProofSteps<'_>,
    entry_filter: &[CheckerPredicate],
) -> Result<InductionEntryDomains, ProjectCheck> {
    let (parameter, binding) = rank.descending;
    let mut entries = IntegerSet::from_type(&parameter.ty).ok_or(ProjectCheck::Unsupported)?;
    let mut earlier = match rank.earlier {
        Some((slot, binding)) => Some((IntegerSet::from_type(&slot.ty).ok_or(ProjectCheck::Unsupported)?, binding)),
        None => None,
    };
    let active = if proof.reset.is_some() {
        &earlier.as_ref().ok_or(ProjectCheck::Unsupported)?.0
    } else {
        &entries
    };
    let minimum = active.extremes().ok_or(ProjectCheck::Unsupported)?.0.clone();
    let empty = |minimum| InductionEntryDomains { descending: IntegerSet::default(), earlier: None, minimum };
    for guard in entry_filter {
        let (guard_binding, domain_guard) = match guard {
            CheckerPredicate::Boolean(true) => continue,
            CheckerPredicate::Boolean(false) => return Ok(empty(minimum)),
            CheckerPredicate::Integer(predicate) => (&predicate.binding, None),
            CheckerPredicate::EntryDomain { binding, values } => (binding, Some(values)),
            _ => return Err(ProjectCheck::Unsupported),
        };
        let domain = if guard_binding == binding {
            &mut entries
        } else {
            let (domain, earlier_binding) = earlier.as_mut().ok_or(ProjectCheck::Unsupported)?;
            if guard_binding != *earlier_binding {
                return Err(ProjectCheck::Unsupported);
            }
            domain
        };
        if let Some(accepted) = domain_guard {
            match domain.intersection(accepted) {
                IntegerIntersection::Unchanged => {}
                IntegerIntersection::Changed(retained) => *domain = retained,
                IntegerIntersection::Empty => return Ok(empty(minimum)),
            }
        } else if let CheckerPredicate::Integer(predicate) = guard {
            *domain = domain.keep(&predicate.operator, &predicate.bound).ok_or(ProjectCheck::Unsupported)?;
            if domain.is_empty() {
                return Ok(empty(minimum));
            }
        }
    }
    Ok(InductionEntryDomains { descending: entries, minimum, earlier: earlier.map(|(domain, _)| domain) })
}

#[cfg(feature = "internal-checker")]
fn check_induction_entry_domain(
    domains: &InductionEntryDomains,
    proof: &TypedInductionProofSteps<'_>,
) -> Result<InductionPremise, ProjectCheck> {
    if domains.descending.is_empty() {
        return Ok(InductionPremise::Unreachable);
    }
    let entry_domain = domains.decreasing(proof)?;
    let narrowed;
    let recursive = match &proof.base_guard {
        CheckerPredicate::Boolean(true) => return Ok(InductionPremise::Unreachable),
        CheckerPredicate::Boolean(false) => entry_domain,
        CheckerPredicate::Integer(predicate) => {
            let operator = complement_integer_comparison(&predicate.operator).ok_or(ProjectCheck::Unsupported)?;
            narrowed = entry_domain.keep(operator, &predicate.bound).ok_or(ProjectCheck::Unsupported)?;
            &narrowed
        }
        _ => return Err(ProjectCheck::Unsupported),
    };
    let Some((first, _)) = recursive.extremes() else {
        return Ok(InductionPremise::Unreachable);
    };
    if first < &(&domains.minimum + proof.decrement) {
        return Ok(InductionPremise::NotStrict);
    }
    if proof.reset.as_ref().is_some_and(|reset| !domains.descending.contains(reset.as_ref()))
        || !recursive.intervals().all(|(lo, hi)| {
            entry_domain.covers_closed(&(lo - proof.decrement), &(hi - proof.decrement))
        })
    {
        return Ok(InductionPremise::CalleeWhereUnproved);
    }
    Ok(InductionPremise::Established)
}

#[cfg(all(test, feature = "internal-checker"))]
thread_local! {
    static OUTPUT_UNION_NORMALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn typed_output_atom(
    expression: &ScalarExpression, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<(IntegerRelation, BigInt), ProjectCheck> {
    let comparison = IntegerComparison::from_expression(expression).ok_or(ProjectCheck::Unsupported)?;
    let (_, relation, bound) = checked_literal_output_goal(comparison, output, inputs, ty)?;
    Ok((relation, bound))
}

fn typed_output_conjunction(
    expression: &ScalarExpression, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<Vec<(IntegerRelation, BigInt)>, ProjectCheck> {
    let mut atoms = Vec::new();
    visit_output_conjuncts(expression, |atom| {
        atoms.push(typed_output_atom(atom, output, inputs, ty)?);
        Ok(())
    })?;
    Ok(atoms)
}

fn visit_output_conjuncts(
    expression: &ScalarExpression,
    mut visit: impl FnMut(&ScalarExpression) -> Result<(), ProjectCheck>,
) -> Result<(), ProjectCheck> {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        if let ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } = expression {
            pending.push(right);
            pending.push(left);
        } else {
            visit(expression)?;
        }
    }
    Ok(())
}

#[cfg(feature = "internal-checker")]
fn typed_unused_named_conjunction(
    expression: &ScalarExpression, output: &str, inputs: &[String], body: &FunctionCallSites, ty: &ScalarType,
) -> Result<TypedProofGoal, ProjectCheck> {
    let mut literals = Vec::new();
    let mut normalized = None;
    let mut entry: [Option<IntegerRelation>; 2] = [None; 2];
    visit_output_conjuncts(expression, |atom| {
        if let Some(old) = checked_old_output_relation(atom, output, inputs, body, ty) {
            let (slot, relation) = old?;
            let condition = entry.get_mut(slot).ok_or(ProjectCheck::Unsupported)?;
            *condition = Some(condition.map_or(relation, |prior| prior.intersection(relation)));
            if normalized.is_none() {
                let mut constraints = IntegerShowConstraints::default();
                for (relation, bound) in std::mem::take(&mut literals) {
                    constraints.add(relation, Cow::Owned(bound));
                }
                normalized = Some(constraints);
            }
        } else {
            let (relation, bound) = typed_output_atom(atom, output, inputs, ty)?;
            if let Some(constraints) = normalized.as_mut() {
                constraints.add(relation, Cow::Owned(bound));
            } else {
                literals.push((relation, bound));
            }
        }
        Ok(())
    })?;
    match normalized {
        Some(mut output) => {
            output.normalize();
            Ok(TypedProofGoal::MixedEntryConjunction { output, entry })
        }
        None => Ok(TypedProofGoal::Conjunction(literals)),
    }
}

#[cfg(feature = "internal-checker")]
fn normalize_show_conjunction(atoms: &mut Vec<(IntegerRelation, BigInt)>, start: usize) {
    atoms[start..].sort_unstable_by(|(_, left), (_, right)| left.cmp(right));
    let mut kept = start;
    for index in start..atoms.len() {
        if kept > start && atoms[kept - 1].1 == atoms[index].1 {
            atoms[kept - 1].0 = atoms[kept - 1].0.intersection(atoms[index].0);
        } else {
            if kept != index {
                atoms.swap(kept, index);
            }
            kept += 1;
        }
    }
    atoms.truncate(kept);
}

#[cfg(feature = "internal-checker")]
fn typed_show_alternatives(
    expression: &ScalarExpression, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<IntegerShowOutput, ProjectCheck> {
    enum Work<'a> {
        Alternative(&'a ScalarExpression),
        Conjunct(&'a ScalarExpression),
        EndClause(usize),
    }
    let mut pending = vec![Work::Alternative(expression)];
    let mut atoms = Vec::new();
    let mut clause_ends = Vec::new();
    while let Some(item) = pending.pop() {
        match item {
            Work::Alternative(ScalarExpression::Binary { operator: BinaryOperator::Or, left, right, .. }) => {
                pending.push(Work::Alternative(right));
                pending.push(Work::Alternative(left));
            }
            Work::Alternative(term @ ScalarExpression::Binary { operator: BinaryOperator::And, .. }) => {
                pending.push(Work::EndClause(atoms.len()));
                pending.push(Work::Conjunct(term));
            }
            Work::Alternative(atom) => {
                atoms.push(typed_output_atom(atom, output, inputs, ty)?);
                clause_ends.push(atoms.len());
            }
            Work::Conjunct(ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. }) => {
                pending.push(Work::Conjunct(right));
                pending.push(Work::Conjunct(left));
            }
            Work::Conjunct(atom) => atoms.push(typed_output_atom(atom, output, inputs, ty)?),
            Work::EndClause(start) => {
                normalize_show_conjunction(&mut atoms, start);
                clause_ends.push(atoms.len());
            }
        }
    }
    Ok(IntegerShowOutput::Alternatives { atoms, clause_ends })
}

fn typed_output_disjunction(
    left: &ScalarExpression, right: &ScalarExpression, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<[(IntegerRelation, BigInt); 2], ProjectCheck> {
    if is_null_literal_name(output) {
        return Err(ProjectCheck::InvalidProof);
    }
    Ok([typed_output_atom(left, output, inputs, ty)?, typed_output_atom(right, output, inputs, ty)?])
}

fn normalize_output_disjunction(
    output: &ScalarType, [(left_relation, left_bound), (right_relation, right_bound)]: &[(IntegerRelation, BigInt); 2],
) -> Option<IntegerSet> {
    #[cfg(all(test, feature = "internal-checker"))]
    OUTPUT_UNION_NORMALIZATIONS.with(|count| count.set(count.get() + 1));
    IntegerSet::comparison_union(output, (*left_relation, left_bound), (*right_relation, right_bound))
}

fn checked_old_output_relation(
    expression: &ScalarExpression, output: &str, inputs: &[String], body: &FunctionCallSites, result: &ScalarType,
) -> Option<Result<(usize, IntegerRelation), ProjectCheck>> {
    let ScalarExpression::Binary { operator, left, right, .. } = expression else { return None; };
    let ScalarExpression::Call { receiver: Some(core), name: old, type_arguments, arguments, .. } = right.as_ref()
        else { return None; };
    if core != "core" || old != "old" {
        return None;
    }
    Some((|| {
        let relation = IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?;
        let ScalarExpression::Name { name: out, .. } = left.as_ref() else { return Err(ProjectCheck::Unsupported); };
        let [ScalarExpression::Name { name: input, .. }] = arguments.as_slice() else { return Err(ProjectCheck::Unsupported); };
        if !type_arguments.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        if out != output || is_null_literal_name(out) || is_null_literal_name(input) {
            return Err(ProjectCheck::InvalidProof);
        }
        let slot = inputs.iter().position(|name| name == input).ok_or(ProjectCheck::InvalidProof)?;
        let parameters = &body.boundary.parameters;
        if !matches!(parameters.len(), 1 | 2) || parameters.get(slot).is_none_or(|parameter| &parameter.ty != result) {
            return Err(ProjectCheck::Unsupported);
        }
        Ok((slot, relation))
    })())
}

fn typed_proof_postcondition<'a, const AUDIT_INDUCTION: bool, const DEFER_OUTPUT_UNION: bool>(
    prepared: &'a PreparedCheckerProject,
    (module, item, function): (usize, usize, &'a ScalarFunction),
    declaration: &CstNode,
    steps: &ScalarBlock,
    #[cfg_attr(not(feature = "internal-checker"), allow(unused_variables))] allow_named_entry_facts: bool,
) -> Result<TypedProofPostcondition<'a>, ProjectCheck> {
    let target = child(declaration, SyntaxKind::ProofTarget)?;
    let input_names = identifiers(&child(&target, SyntaxKind::Parameters)?).collect::<Vec<_>>();
    let outputs = child(declaration, SyntaxKind::ProofOutputs)?;
    let output_names = identifiers(&child(&outputs, SyntaxKind::Parameters)?).collect::<Vec<_>>();
    let mut unique = HashSet::new();
    if !input_names.iter().chain(&output_names).all(|name| unique.insert(name.as_str())) {
        return Err(ProjectCheck::InvalidProof);
    }
    let ensures_node = child(declaration, SyntaxKind::ProofEnsures)?;
    let exists = ensures_node.children().find(|node| node.kind() == SyntaxKind::ProofExists);

    let (project, calls) = prepared.parts();
    let target = &project.modules[module];
    if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
        return Err(ProjectCheck::Unsupported);
    }
    let body = calls.function_body(module, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let boundary = &body.boundary;
    if input_names.len() != boundary.parameters.len() || output_names.len() != boundary.results.len() {
        return Err(ProjectCheck::InvalidProof);
    }
    let measure = typed_proof_decreases(declaration, &input_names, body)?;
    if !AUDIT_INDUCTION && measure.is_some() {
        return Err(ProjectCheck::Unsupported);
    }
    let ([result], [output]) = (boundary.results.as_slice(), output_names.as_slice()) else {
        return Err(ProjectCheck::Unsupported);
    };
    if !is_integer_type(&result.ty) {
        return Err(ProjectCheck::Unsupported);
    }
    let mut foralls = declaration.children().filter(|node| node.kind() == SyntaxKind::ProofForall);
    let goal = if let Some(exists) = exists {
        if measure.is_some() {
            return Err(ProjectCheck::Unsupported);
        }
        if let Some(forall) = foralls.next() {
            #[cfg(feature = "internal-checker")]
            {
                if !allow_named_entry_facts {
                    return Err(ProjectCheck::Unsupported);
                }
                if let Some(second) = foralls.next() {
                    let predicate = derive_expression(&child(&exists, SyntaxKind::Expression)?);
                    typed_named_repeated_universal::<true>(
                        [forall, second].into_iter().chain(foralls), &predicate, output, &input_names, &result.ty,
                        Some((&exists, steps)),
                    )?
                } else {
                    typed_named_dependent_witness(&forall, &exists, steps, output, &input_names, &result.ty)?
                }
            }
            #[cfg(not(feature = "internal-checker"))]
            {
                let _ = forall;
                return Err(ProjectCheck::Unsupported);
            }
        } else {
            typed_existential_output_goal(&exists, steps, output, &input_names, &result.ty, body, allow_named_entry_facts)?
        }
    } else {
        let ensures = derive_expression(&child(&ensures_node, SyntaxKind::Expression)?);
        if let Some(forall) = foralls.next() {
            if measure.is_some() {
                if foralls.next().is_some() {
                    return Err(ProjectCheck::Unsupported);
                }
                typed_universal_output_goal(&forall, &ensures, output, &input_names, &result.ty, false)?
            } else if let Some(second) = foralls.next() {
                #[cfg(feature = "internal-checker")]
                {
                    if !allow_named_entry_facts || !steps.items.is_empty() {
                        return Err(ProjectCheck::Unsupported);
                    }
                    typed_named_repeated_universal::<false>(
                        [forall, second].into_iter().chain(foralls), &ensures, output, &input_names, &result.ty, None,
                    )?
                }
                #[cfg(not(feature = "internal-checker"))]
                {
                    let _ = second;
                    return Err(ProjectCheck::Unsupported);
                }
            } else {
                #[cfg(feature = "internal-checker")]
                let conjunction = if allow_named_entry_facts && steps.items.is_empty()
                    && matches!(&ensures, ScalarExpression::Binary { operator: BinaryOperator::And, .. })
                {
                    Some(typed_named_universal_conjunction(&forall, &ensures, output, &input_names, &result.ty)?)
                } else {
                    None
                };
                #[cfg(not(feature = "internal-checker"))]
                let conjunction = None;
                match conjunction {
                    Some(goal) => goal,
                    None => typed_universal_output_goal(
                        &forall, &ensures, output, &input_names, &result.ty, allow_named_entry_facts && steps.items.is_empty(),
                    )?,
                }
            }
        } else if let ScalarExpression::Binary { operator: BinaryOperator::Or, left, right, .. } = &ensures {
            let atoms = typed_output_disjunction(left, right, output, &input_names, &result.ty)?;
            if DEFER_OUTPUT_UNION {
                TypedProofGoal::DeferredDisjunction(atoms)
            } else {
                TypedProofGoal::Disjunction(normalize_output_disjunction(&result.ty, &atoms).ok_or(ProjectCheck::Unsupported)?)
            }
        } else if matches!(&ensures, ScalarExpression::Binary { operator: BinaryOperator::And, .. }) {
            #[cfg(feature = "internal-checker")]
            let mixed = if allow_named_entry_facts && steps.items.is_empty() && measure.is_none() {
                Some(typed_unused_named_conjunction(&ensures, output, &input_names, body, &result.ty)?)
            } else {
                None
            };
            #[cfg(not(feature = "internal-checker"))]
            let mixed = None;
            match mixed {
                Some(goal) => goal,
                None => TypedProofGoal::Conjunction(typed_output_conjunction(&ensures, output, &input_names, &result.ty)?),
            }
        } else if let Some(ensures) = IntegerComparison::from_expression(&ensures) {
            let (operator, relation, bound) = checked_literal_output_goal(ensures, output, &input_names, &result.ty)?;
            TypedProofGoal::Literal { operator, relation, bound }
        } else {
            let (slot, relation) = checked_old_output_relation(&ensures, output, &input_names, body, &result.ty)
                .ok_or(ProjectCheck::Unsupported)??;
            TypedProofGoal::EntryComparison { slot, relation }
        }
    };

    Ok(TypedProofPostcondition {
        target,
        body,
        result_type: &result.ty,
        input_names,
        output: output.clone(),
        goal,
        measure,
    })
}

#[cfg(test)]
fn check_selected_proof(
    prepared: &PreparedCheckerProject,
    (module, item, function): (usize, usize, &ScalarFunction),
    declaration: &CstNode,
    proof_source: &SourceIdentity,
    summary_source: Option<ProofSummaryOrigin<'_>>,
) -> Result<(ProjectCheck, Option<VerifiedReturnBound>), ProjectCheck> {
    let steps = derive_block(&child(declaration, SyntaxKind::Block)?);
    let TypedProofPostcondition { target, body, result_type, input_names, output, goal, .. } =
        typed_proof_postcondition::<false, false>(prepared, (module, item, function), declaration, &steps, false)?;
    let TypedProofGoal::Literal { operator, bound, .. } = goal else {
        return Err(ProjectCheck::Unsupported);
    };

    let filter = prepare_proof_entry_filter(declaration, proof_source, target.target_layout, &input_names, body)
        .map_err(|error| match error {
            ProjectCheck::Invalid(_) => ProjectCheck::InvalidProof,
            error => error,
        })?;
    let checked = if !steps.items.is_empty() {
        let proof = proof_show_plan::<true, false>(&steps, &input_names, &output, result_type, body, None)?;
        check_integer_return_bound_with_shows(body, &operator, &bound, filter.as_deref(), &proof)
    } else if let Some(filter) = filter.as_deref() {
        check_integer_return_bound_where(body, &operator, &bound, filter)
    } else {
        check_integer_return_bound(body, &operator, &bound)
            .map(|result| result.map(ProofCoverage::Applicable))
    };
    let outcome = match checked {
        None => ProjectCheck::Unsupported,
        Some(Ok(ProofCoverage::Applicable(true))) => ProjectCheck::Proved,
        Some(Ok(ProofCoverage::Applicable(false))) => ProjectCheck::Refuted,
        Some(Ok(ProofCoverage::NoApplicableInputs)) => ProjectCheck::NoApplicableInputs,
        Some(Err(failure)) => project_failure(body, &target.source, failure),
    };
    let verified = summary_source.filter(|_| outcome == ProjectCheck::Proved && steps.items.is_empty()).and_then(|origin| {
        issue_return_bound(
            origin,
            declaration,
            &target.source,
            function,
            body,
            target.target_layout,
            (&operator, &bound),
            filter.as_deref(),
        )
    });
    Ok((outcome, verified))
}

#[cfg(test)]
fn issue_return_bound(
    origin: ProofSummaryOrigin<'_>,
    declaration: &CstNode,
    target_source: &SourceIdentity,
    function: &ScalarFunction,
    body: &FunctionCallSites,
    target_layout: ScalarTargetLayout,
    (operator, bound): (&BinaryOperator, &BigInt),
    conditions: Option<&[CheckerPredicate]>,
) -> Option<VerifiedReturnBound> {
    if !body.structure.checker_directives.is_empty()
        || !body.call_occurrences.is_empty()
        || !body.allocation_length_calls.is_empty()
        || !body.assignments.is_empty()
    {
        return None;
    }
    let [output] = body.boundary.results.as_slice() else {
        return None;
    };
    let guards = conditions.unwrap_or_default().iter().map(|condition| {
        match condition {
            CheckerPredicate::Boolean(value) => Some(VerifiedEntryGuard::Boolean(*value)),
            CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => None,
            CheckerPredicate::Integer(predicate) => {
                let slot = body.boundary.checker_parameter_bindings.as_deref()?
                    .iter().position(|binding| *binding == predicate.binding)?;
                Some(VerifiedEntryGuard::Integer {
                    slot,
                    ty: body.boundary.parameters[slot].ty.clone(),
                    operator: predicate.operator.clone(),
                    bound: predicate.bound.clone(),
                })
            }
        }
    }).collect::<Option<Vec<_>>>()?;
    let (proof_venue, proof_source, target) = match origin {
        ProofSummaryOrigin::DetachedFragment(source) => (
            ProofVenue::DetachedFragment,
            source,
            ResolvedCallTarget::ModuleCallable {
                source: target_source.clone(),
                declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Declaration,
            },
        ),
        ProofSummaryOrigin::ModuleDeclaration { source, target } => {
            let ResolvedCallTarget::ModuleCallable {
                source: callable_source,
                declaration_span,
                concrete: ConcreteCallSelection::Declaration,
            } = target else {
                return None;
            };
            if callable_source != target_source || *declaration_span != function.name_span {
                return None;
            }
            (ProofVenue::ModuleDeclaration, source, target.clone())
        }
    };
    Some(VerifiedReturnBound {
        proof_venue,
        proof_source: proof_source.clone(),
        proof_span: byte_span(declaration),
        proof_name: identifiers(declaration).next()?,
        target,
        target_layout,
        parameters: body.boundary.parameters.iter().map(|parameter| parameter.ty.clone()).collect(),
        output_slot: output.ordinal,
        output_type: output.ty.clone(),
        operator: operator.clone(),
        bound: bound.clone(),
        guards,
    })
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
#[cfg(test)]
enum ImportedRequirement {
    Proved,
    NotEstablished,
}

#[cfg(test)]
fn use_verified_return_bound(
    prepared: &PreparedCheckerProject,
    caller_source: &SourceIdentity,
    caller_name: &str,
    proof_identity: (&SourceIdentity, ByteSpan, &str),
    verified: &VerifiedReturnBound,
) -> Option<ImportedRequirement> {
    if verified.proof_venue != ProofVenue::DetachedFragment
        || proof_identity != (&verified.proof_source, verified.proof_span, verified.proof_name.as_str())
    {
        return None;
    }
    apply_verified_return_bound(prepared, caller_source, caller_name, verified)
}

#[cfg(test)]
fn apply_verified_return_bound(
    prepared: &PreparedCheckerProject,
    caller_source: &SourceIdentity,
    caller_name: &str,
    verified: &VerifiedReturnBound,
) -> Option<ImportedRequirement> {
    let (project, calls) = prepared.parts();
    let (module, item, function) = select_checked_root(project, caller_source, caller_name).ok()?;
    if project.modules[module].target_layout != verified.target_layout
        || !function.generic_parameters.is_empty()
        || !function.overload_arms.is_empty()
        || verified.target == (ResolvedCallTarget::ModuleCallable {
            source: caller_source.clone(),
            declaration_span: function.name_span,
            concrete: ConcreteCallSelection::Declaration,
        })
    {
        return None;
    }
    let body = calls.function_body(module, item, None)?;
    let (position, origin) = root_local_import_requirement(body)?;
    let structure = &body.structure;
    let [(argument_ordinal, _)] = origin.arguments.as_slice() else { return None; };
    let site = body.resolved_call_site(origin.call)?;
    let signature = body.checked_call_signatures.get(&origin.call)?;
    let [parameter] = signature.parameters.as_slice() else {
        return None;
    };
    let [output] = signature.outputs.outputs.as_slice() else {
        return None;
    };
    if site.target != verified.target
        || signature.parameters != verified.parameters
        || verified.output_slot != 0
        || output.ty != verified.output_type
        || structure.checked_types.expressions.get(argument_ordinal)? != parameter
    {
        return None;
    }
    let mut guards = Vec::new();
    for guard in &verified.guards {
        match guard {
            VerifiedEntryGuard::Boolean(true) => {}
            VerifiedEntryGuard::Integer { slot: 0, ty, operator, bound } if ty == parameter =>
                guards.push(ImportedEntryGuard { slot: 0, relation: IntegerRelation::from_operator(operator)?, bound }),
            _ => return None,
        }
    }
    let values = IntegerSet::from_type(&verified.output_type)?.keep(&verified.operator, &verified.bound)?;
    let imported = ImportedLocal { position, values: ImportedIntegerValues::ready(values), guards };
    Some(if proves_requirement_with_imported_root_local(body, imported)? {
        ImportedRequirement::Proved
    } else {
        ImportedRequirement::NotEstablished
    })
}

fn root_local_import_requirement(body: &FunctionCallSites) -> Option<(usize, &CheckerIntegerCallOrigin)> {
    let structure = &body.structure;
    let root = structure.root;
    let items = &structure.blocks.get(root.0)?.items;
    let first_nonliteral = items.iter().position(|item| {
        !matches!(item.kind, BlockItemKind::Binding(_))
            || !structure.checker_integer_locals.contains_key(&(root, item.position))
    })?;
    root_local_import_requirement_at(body, &items[first_nonliteral..])
}

fn root_local_import_requirement_at<'a>(
    body: &'a FunctionCallSites, items: &[super::control::BlockItem],
) -> Option<(usize, &'a CheckerIntegerCallOrigin)> {
    let structure = &body.structure;
    let root = structure.root;
    let [binding_item, requirement_item, tail @ ..] = items else {
        return None;
    };
    if !matches!(binding_item.kind, BlockItemKind::Binding(_))
        || !(tail.is_empty() || matches!(tail, [item] if matches!(item.kind, BlockItemKind::FinalOutput(_))))
    {
        return None;
    }
    let BlockItemKind::Evaluation { expression } = requirement_item.kind else {
        return None;
    };
    let directive = structure.checker_directives.get(&expression)?;
    let CheckerPredicate::Integer(predicate) = directive.predicate.as_ref()? else {
        return None;
    };
    let origin = structure.checker_integer_call_locals.get(&(root, binding_item.position))?;
    if directive.operation != CheckerOperation::Require || predicate.binding != origin.binding {
        return None;
    }
    Some((binding_item.position, origin))
}

fn pure_proof_input_expression(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Name { name, .. } => name != "null",
        ScalarExpression::Boolean { .. } | ScalarExpression::Integer { .. }
        | ScalarExpression::Float { .. } | ScalarExpression::Char { .. }
        | ScalarExpression::InvalidInteger { .. } | ScalarExpression::InvalidFloat { .. } => true,
        ScalarExpression::Unary { operand, .. } => pure_proof_input_expression(operand),
        ScalarExpression::Binary { left, right, .. } =>
            pure_proof_input_expression(left) && pure_proof_input_expression(right),
        _ => false,
    }
}

fn typed_proof_where(
    declaration: &CstNode,
    source: &SourceIdentity,
    target_layout: ScalarTargetLayout,
    input_names: &[String],
    body: &FunctionCallSites,
) -> Result<Option<ScalarExpression>, ProjectCheck> {
    let Some(clause) = declaration.children().find(|node| node.kind() == SyntaxKind::ProofWhere) else {
        return Ok(None);
    };
    let expression = derive_expression(&child(&clause, SyntaxKind::Expression)?);
    if !pure_proof_input_expression(&expression) {
        return Err(ProjectCheck::Unsupported);
    }
    let inputs = input_names.iter().zip(&body.boundary.parameters).map(|(name, parameter)| (name.clone(), parameter.ty.clone()));
    typecheck_proof_where(source, target_layout, &expression, inputs).map_err(ProjectCheck::Invalid)?;
    Ok(Some(expression))
}

#[cfg(test)]
fn prepare_proof_entry_filter(
    declaration: &CstNode,
    source: &SourceIdentity,
    layout: ScalarTargetLayout,
    inputs: &[String],
    body: &FunctionCallSites,
) -> Result<Option<Vec<CheckerPredicate>>, ProjectCheck> {
    prepare_proof_entry_filter_in_scope(declaration, source, layout, inputs, body, false)
}

fn prepare_proof_entry_filter_in_scope(
    declaration: &CstNode, source: &SourceIdentity, layout: ScalarTargetLayout,
    inputs: &[String], body: &FunctionCallSites, allow_domain: bool,
) -> Result<Option<Vec<CheckerPredicate>>, ProjectCheck> {
    typed_proof_where(declaration, source, layout, inputs, body)?
        .as_ref().map(|expression| proof_entry_filters(expression, inputs, body, allow_domain)).transpose()
}

fn proof_entry_filters(
    expression: &ScalarExpression, inputs: &[String], body: &FunctionCallSites, allow_domain: bool,
) -> Result<Vec<CheckerPredicate>, ProjectCheck> {
    let mut filters = Vec::new();
    let mut pending = vec![(expression, false)];
    let mut has_domain = false;
    while let Some((expression, negated)) = pending.pop() {
        match expression {
            ScalarExpression::Unary { operator: UnaryOperator::LogicalNot, operand, .. } =>
                pending.push((operand, !negated)),
            ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } if !negated => {
                pending.push((right, false));
                pending.push((left, false));
            }
            ScalarExpression::Binary { operator: BinaryOperator::Or, left, right, .. }
                if !negated && allow_domain && !has_domain =>
            {
                let (CheckerPredicate::Integer(left), CheckerPredicate::Integer(right)) = (
                    proof_input_condition::<false>(left, inputs, body, false)?,
                    proof_input_condition::<false>(right, inputs, body, false)?,
                ) else { return Err(ProjectCheck::Unsupported); };
                if left.binding != right.binding || left.ty != right.ty {
                    return Err(ProjectCheck::Unsupported);
                }
                let left_relation = IntegerRelation::from_operator(&left.operator).ok_or(ProjectCheck::Unsupported)?;
                let right_relation = IntegerRelation::from_operator(&right.operator).ok_or(ProjectCheck::Unsupported)?;
                let values = IntegerSet::comparison_union(&left.ty, (left_relation, &left.bound), (right_relation, &right.bound))
                    .ok_or(ProjectCheck::Unsupported)?;
                filters.push(CheckerPredicate::EntryDomain { binding: left.binding, values });
                has_domain = true;
            }
            _ => filters.push(proof_input_condition::<false>(expression, inputs, body, negated)?),
        }
    }
    Ok(filters)
}

fn complement_integer_comparison(operator: &BinaryOperator) -> Option<&'static BinaryOperator> {
    Some(match operator {
        BinaryOperator::Equal => &BinaryOperator::NotEqual,
        BinaryOperator::NotEqual => &BinaryOperator::Equal,
        BinaryOperator::Less => &BinaryOperator::GreaterEqual,
        BinaryOperator::LessEqual => &BinaryOperator::Greater,
        BinaryOperator::Greater => &BinaryOperator::LessEqual,
        BinaryOperator::GreaterEqual => &BinaryOperator::Less,
        _ => return None,
    })
}

fn proof_input_condition<const VERIFY_LITERAL_FIT: bool>(
    expression: &ScalarExpression,
    inputs: &[String],
    body: &FunctionCallSites,
    negated: bool,
) -> Result<CheckerPredicate, ProjectCheck> {
    if let ScalarExpression::Boolean { value, .. } = expression {
        return Ok(CheckerPredicate::Boolean(*value != negated));
    }
    let mut condition = IntegerComparison::from_expression(expression).ok_or(ProjectCheck::Unsupported)?;
    if negated {
        condition.operator = complement_integer_comparison(condition.operator).ok_or(ProjectCheck::Unsupported)?;
    }
    let index = inputs.iter().position(|input| input == condition.name)
        .ok_or(ProjectCheck::InvalidProof)?;
    let ty = &body.boundary.parameters[index].ty;
    if VERIFY_LITERAL_FIT {
        if !is_integer_type(ty) {
            return Err(ProjectCheck::Unsupported);
        }
        if !integer_literal_fits_type(condition.bound.as_ref(), ty) {
            return Err(ProjectCheck::InvalidProof);
        }
    }
    let bindings = body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
    let predicate = condition.typed(ty, ty, bindings[index].clone())
        .ok_or(ProjectCheck::Unsupported)?;
    Ok(CheckerPredicate::Integer(predicate))
}

fn proof_show_plan<'a, const VERIFY_ENTRY_WRITES: bool, const ALLOW_NAMED_STEPS: bool>(
    steps: &'a ScalarBlock,
    inputs: &[String],
    output: &str,
    ty: &ScalarType,
    body: &'a FunctionCallSites,
    ensures: Option<(&BinaryOperator, &'a BigInt)>,
) -> Result<IntegerShowProof<'a>, ProjectCheck> {
    let bindings = body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
    if VERIFY_ENTRY_WRITES && body.assignments.values().flat_map(|group| group.destinations()).any(|destination| {
        match body.prepared_direct_destination(destination.destination) {
            Some(DirectDestinationIdentity::Existing(binding)) => bindings.contains(binding),
            Some(DirectDestinationIdentity::Module(_) | DirectDestinationIdentity::Introduce { .. }) => false,
            None => true,
        }
    }) {
        return Err(ProjectCheck::Unsupported);
    }
    match steps.items.as_slice() {
        [item @ ScalarBlockItem::Expression(ScalarExpression::Call { .. })] => {
            let [terminated] = steps.terminated_items.as_slice() else { return Err(ProjectCheck::Unsupported); };
            let claim = proof_show_expression(item, *terminated)?;
            #[cfg(feature = "internal-checker")]
            if ALLOW_NAMED_STEPS {
                if let Some(entry) = checked_old_output_relation(claim, output, inputs, body, ty) {
                    let (slot, relation) = entry?;
                    let entry = ImmutableProofOutputComparison::prepare(body, slot, ty, relation)
                        .ok_or(ProjectCheck::Unsupported)?;
                    return Ok(IntegerShowProof::EntryComparison(entry));
                }
                if let ScalarExpression::Binary { operator: BinaryOperator::Or, left, right, .. } = claim {
                    if inputs.iter().any(|name| is_null_literal_name(name)) {
                        return Err(ProjectCheck::InvalidProof);
                    }
                    let composite = |term: &ScalarExpression| matches!(term,
                        ScalarExpression::Binary { operator: BinaryOperator::And | BinaryOperator::Or, .. });
                    let show = if composite(left) || composite(right) {
                        if is_null_literal_name(output) {
                            return Err(ProjectCheck::InvalidProof);
                        }
                        typed_show_alternatives(claim, output, inputs, ty)?
                    } else {
                        IntegerShowOutput::Disjunction(typed_output_disjunction(left, right, output, inputs, ty)?)
                    };
                    return Ok(IntegerShowProof::Output(show));
                }
                if matches!(claim, ScalarExpression::Binary { operator: BinaryOperator::And, .. }) {
                    if is_null_literal_name(output) || inputs.iter().any(|name| is_null_literal_name(name)) {
                        return Err(ProjectCheck::InvalidProof);
                    }
                    let mut atoms = typed_output_conjunction(claim, output, inputs, ty)?;
                    normalize_show_conjunction(&mut atoms, 0);
                    return Ok(IntegerShowProof::Output(IntegerShowOutput::Conjunction(atoms)));
                }
                if matches!(claim, ScalarExpression::Binary { operator, .. } if *operator != BinaryOperator::Equal) {
                    if let Some(comparison) = IntegerComparison::from_expression(claim) {
                        if is_null_literal_name(output) || inputs.iter().any(|name| is_null_literal_name(name)) {
                            return Err(ProjectCheck::InvalidProof);
                        }
                        let (_, relation, bound) = checked_literal_output_goal(comparison, output, inputs, ty)?;
                        return Ok(IntegerShowProof::Output(IntegerShowOutput::Relation { relation, bound }));
                    }
                }
            }
            Ok(IntegerShowProof::Equality(proof_show_equality(claim, inputs, output, ty, body, bindings)?))
        }
        [ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. })] => {
            let guard = proof_input_condition::<true>(condition, inputs, body, false)?;
            #[cfg(feature = "internal-checker")]
            if ALLOW_NAMED_STEPS && matches!(&guard, CheckerPredicate::Integer(_)) {
                let comparison_claim = |arm: &'a ScalarBlock| {
                    let ([item], [terminated]) = (arm.items.as_slice(), arm.terminated_items.as_slice()) else {
                        return None;
                    };
                    let claim = proof_show_expression(item, *terminated).ok()?;
                    let ScalarExpression::Binary { operator, right, .. } = claim else { return None; };
                    IntegerRelation::from_operator(operator)?;
                    let old = matches!(right.as_ref(), ScalarExpression::Call {
                        receiver: Some(core), name, ..
                    } if core == "core" && name == "old");
                    Some((claim, operator, old))
                };
                if let (Some((yes, yes_operator, yes_old)), Some((no, no_operator, no_old))) =
                    (comparison_claim(then_branch), comparison_claim(else_branch))
                {
                    let entry_claims = yes_old && no_old;
                    let literal_claims = !yes_old && !no_old
                        && (*yes_operator != BinaryOperator::Equal || *no_operator != BinaryOperator::Equal);
                    if entry_claims || literal_claims {
                        if let CheckerPredicate::Integer(guard) = guard {
                            if is_null_literal_name(output) || inputs.iter().any(|name| is_null_literal_name(name)) {
                                return Err(ProjectCheck::InvalidProof);
                            }
                            if guard.ty != *ty {
                                return Err(ProjectCheck::Unsupported);
                            }
                            let claims = if entry_claims {
                                let validate = |claim| {
                                    let (slot, relation) = checked_old_output_relation(claim, output, inputs, body, ty)
                                        .ok_or(ProjectCheck::Unsupported)??;
                                    if bindings.get(slot) != Some(&guard.binding) {
                                        return Err(ProjectCheck::Unsupported);
                                    }
                                    Ok(relation)
                                };
                                GuardedOutputClaims::ImmutableEntry { yes: validate(yes)?, no: validate(no)? }
                            } else {
                                GuardedOutputClaims::Literal {
                                    yes: typed_output_atom(yes, output, inputs, ty)?,
                                    no: typed_output_atom(no, output, inputs, ty)?,
                                }
                            };
                            return Ok(IntegerShowProof::GuardedOutput { guard, claims });
                        }
                    }
                }
            }
            Ok(IntegerShowProof::Conditional {
                guard,
                then_claim: proof_show_claim(then_branch, inputs, output, ty, body, bindings)?,
                else_claim: proof_show_claim(else_branch, inputs, output, ty, body, bindings)?,
            })
        }
        [_, _, ..] if ALLOW_NAMED_STEPS && steps.terminated_items.len() == steps.items.len() => {
            if is_null_literal_name(output) || inputs.iter().any(|name| is_null_literal_name(name)) {
                return Err(ProjectCheck::InvalidProof);
            }
            let ensures_equality = ensures.and_then(|(operator, bound)| (*operator == BinaryOperator::Equal).then_some(bound));
            let mut seen = HashSet::new();
            let mut claims = Vec::new();
            #[cfg(feature = "internal-checker")]
            let mut mixed: Option<(IntegerShowConstraints<'a>, [Option<IntegerRelation>; 2])> = None;
            for (item, &terminated) in steps.items.iter().zip(&steps.terminated_items) {
                let expression = proof_show_expression(item, terminated)?;
                #[cfg(feature = "internal-checker")]
                {
                    let old = checked_old_output_relation(expression, output, inputs, body, ty);
                    let non_equality = matches!(expression, ScalarExpression::Binary { operator, .. }
                        if *operator != BinaryOperator::Equal && IntegerRelation::from_operator(operator).is_some());
                    if mixed.is_some() || old.is_some() || non_equality {
                        if mixed.is_none() {
                            let mut normalized = IntegerShowConstraints::default();
                            let (operator, bound) = ensures.ok_or(ProjectCheck::Unsupported)?;
                            normalized.add(IntegerRelation::from_operator(operator).ok_or(ProjectCheck::Unsupported)?,
                                std::borrow::Cow::Borrowed(bound));
                            let equality = IntegerRelation::from_operator(&BinaryOperator::Equal).ok_or(ProjectCheck::Unsupported)?;
                            for claim in claims.drain(..) {
                                let IntegerReturn::Const(value) = claim else { return Err(ProjectCheck::Unsupported); };
                                normalized.add(equality, std::borrow::Cow::Borrowed(value));
                            }
                            drop(std::mem::take(&mut seen));
                            mixed = Some((normalized, [None; 2]));
                        }
                        let (constraints, entry) = mixed.as_mut().ok_or(ProjectCheck::Unsupported)?;
                        if let Some(old) = old {
                            let (slot, relation) = old?;
                            let known = entry.get_mut(slot).ok_or(ProjectCheck::Unsupported)?;
                            *known = Some(known.map_or(relation, |current| current.intersection(relation)));
                        } else {
                            if matches!(expression, ScalarExpression::Binary {
                                operator: BinaryOperator::Equal, right, ..
                            } if !matches!(right.as_ref(), ScalarExpression::Integer { .. })) {
                                return Err(ProjectCheck::Unsupported);
                            }
                            let comparison = IntegerComparison::from_expression(expression).ok_or(ProjectCheck::Unsupported)?;
                            let (_, relation, value) = checked_literal_output_goal_borrowed(comparison, output, inputs, ty)?;
                            constraints.add(relation, value);
                        }
                        continue;
                    }
                }
                let claim = proof_show_equality(expression, inputs, output, ty, body, bindings)?;
                if matches!(claim, IntegerReturn::Const(value) if ensures_equality == Some(value)) {
                    continue;
                }
                if seen.insert(claim) {
                    claims.push(claim);
                }
            }
            #[cfg(feature = "internal-checker")]
            if let Some((mut output, entry)) = mixed {
                output.normalize();
                return Ok(IntegerShowProof::MixedSequence { formals: bindings, output, entry });
            }
            Ok(IntegerShowProof::Sequence(claims))
        }
        _ => Err(ProjectCheck::Unsupported),
    }
}

fn proof_show_claim<'a>(
    arm: &'a ScalarBlock,
    inputs: &[String],
    output: &str,
    ty: &ScalarType,
    body: &FunctionCallSites,
    bindings: &'a [ReferenceBindingId],
) -> Result<IntegerReturn<'a>, ProjectCheck> {
    let ([item], [terminated]) = (arm.items.as_slice(), arm.terminated_items.as_slice()) else {
        return Err(ProjectCheck::Unsupported);
    };
    proof_show_item_claim(item, *terminated, inputs, output, ty, body, bindings)
}

fn proof_show_item_claim<'a>(
    item: &'a ScalarBlockItem,
    terminated: bool,
    inputs: &[String],
    output: &str,
    ty: &ScalarType,
    body: &FunctionCallSites,
    bindings: &'a [ReferenceBindingId],
) -> Result<IntegerReturn<'a>, ProjectCheck> {
    proof_show_equality(proof_show_expression(item, terminated)?, inputs, output, ty, body, bindings)
}

fn proof_show_expression(item: &ScalarBlockItem, terminated: bool) -> Result<&ScalarExpression, ProjectCheck> {
    let ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver, name, arguments, type_arguments, ..
    }) = item else {
        return Err(ProjectCheck::Unsupported);
    };
    if receiver.as_deref() != Some("core") || name != "show" || !type_arguments.is_empty() || !terminated {
        return Err(ProjectCheck::Unsupported);
    }
    let [claim] = arguments.as_slice() else { return Err(ProjectCheck::Unsupported); };
    Ok(claim)
}

fn proof_show_equality<'a>(
    claim: &'a ScalarExpression,
    inputs: &[String],
    output: &str,
    ty: &ScalarType,
    body: &FunctionCallSites,
    bindings: &'a [ReferenceBindingId],
) -> Result<IntegerReturn<'a>, ProjectCheck> {
    let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } = claim else {
        return Err(ProjectCheck::Unsupported);
    };
    let ScalarExpression::Name { name, .. } = left.as_ref() else {
        return Err(ProjectCheck::Unsupported);
    };
    if name != output {
        return Err(ProjectCheck::Unsupported);
    }
    match right.as_ref() {
        ScalarExpression::Integer { value, .. } if integer_literal_fits_type(value, ty) => {
            Ok(IntegerReturn::Const(value))
        }
        ScalarExpression::Integer { .. } => Err(ProjectCheck::InvalidProof),
        ScalarExpression::Name { name, .. } => {
            let index = inputs.iter().position(|input| input == name).ok_or(ProjectCheck::Unsupported)?;
            if body.boundary.parameters[index].ty != *ty {
                return Err(ProjectCheck::InvalidProof);
            }
            Ok(IntegerReturn::Alias(&bindings[index]))
        }
        _ => Err(ProjectCheck::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;
    use crate::{derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project, ScalarTargetLayout};

    const OUTPUT: &str = r#"{ .stdout = ""; .stderr = "unexpected state"; .code = 1; .files = []; }"#;

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_failure_collection_joins_actual_and_unused_roots_once() {
        let app = source("src/main.w", "collect-ordered-package-failures");
        let headings = ["value < 2", "value < 4", "value < 6"];
        let prepare = |first: u8, second: u8, last: u8| {
            let output = |code: u8| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
            let text = format!(r#"%%start
                unit(u8) unopened = fn(value) {{ core.require(value < 4, {}); }};
                unit(u8) actual = fn(value) {{ core.require(value < 2, {}); }};
                unit(u8) later = fn(value) {{ core.require(value < 6, {}); }};
                actual(3);
            %%end"#, output(second), output(first), output(last));
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, prepared)
        };
        for codes in [[0, 23, 9], [0, 0, 0]] {
            let (text, prepared) = prepare(codes[0], codes[1], codes[2]);
            let (active, reports) = collect_ordered_package_failures(
                &prepared, ScalarCheckerScope::CurrentPackage, 1,
            ).unwrap();
            assert!(active);
            let data = reports.iter().map(|failure| {
                let ProjectCheck::Failed { source, condition, output, state, .. } = failure else {
                    panic!("expected authored failure");
                };
                assert_eq!(source, &app);
                let intervals = state.intervals_at(0, 0).unwrap().map(|(lo, hi)| (lo.clone(), hi.clone()))
                    .collect::<Vec<_>>();
                (&text[condition.start as usize..condition.end as usize], &output.code_value, intervals)
            }).collect::<Vec<_>>();
            assert_eq!(data.iter().map(|(condition, _, _)| *condition).collect::<Vec<_>>(), headings);
            assert_eq!(data.iter().map(|(_, code, _)| (*code).clone()).collect::<Vec<_>>(), codes.map(BigInt::from));
            assert_eq!(data.iter().map(|(_, _, rows)| rows.clone()).collect::<Vec<_>>(),
                [(3, 3), (4, 255), (6, 255)].map(|(lo, hi)| vec![(BigInt::from(lo), BigInt::from(hi))]));
            let (active, main_only) = collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 1).unwrap();
            assert!(active);
            drop(data);
            let first = reports.into_iter().next().unwrap();
            assert_eq!(main_only, [first]);
            assert_eq!(check_package_roots(&prepared), Err(main_only.into_iter().next().unwrap()));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_contextual_writes_keep_outer_binding_and_exact_witness() {
        let app = source("src/main.w", "ordered-base-contextual-storage");
        let program = format!(r#"%%start
            unit(i8) state = fn(selector) {{
                i8 core.context.count = 1;
                if (selector < 0) {{
                    i8 core.context.count = core.context.count;
                    core.require(core.context.count == 1, {OUTPUT});
                    core.context.count = 2;
                    core.require(core.context.count == 2, {OUTPUT});
                }} else {{
                    core.context.count = 3;
                }};
                if (selector < 0) {{
                    core.require(core.context.count == 1, {OUTPUT});
                }} else {{
                    core.require(core.context.count == 3, {OUTPUT});
                }};
            }};
        %%end"#);
        let run = |text: &str, scope| {
            check_loaded_scalar_ordered_local_proof_contexts(
                vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
                inputs(), scope, Vec::new(),
            ).unwrap_or_else(|error| panic!("proof-free contextual checking in {scope:?}: {error:?}"))
        };
        let valid = run(&program, ScalarCheckerScope::CurrentPackage);
        assert!(valid.active && valid.directives.is_empty());
        assert!(!run(&program, ScalarCheckerScope::MainFlowOnly).active);
        let rejected = program.replace("core.context.count == 2", "core.context.count == 9")
            .replace("core.context.count == 3", "core.context.count == 1");
        let rejected = run(&rejected, ScalarCheckerScope::CurrentPackage);
        let [inner, outer] = rejected.directives.as_slice() else { panic!("one inner and one outer failure"); };
        let inner_axes = inner.witness.axes().collect::<Vec<_>>();
        let outer_axes = outer.witness.axes().collect::<Vec<_>>();
        assert_eq!((inner_axes.len(), outer_axes.len()), (3, 2));
        assert_eq!(inner_axes[1], outer_axes[1]);
        assert_ne!(inner_axes[1], inner_axes[2]);
        let point = |v| BigInt::from(v);
        assert!(inner.witness.intervals_at(0, 0).unwrap().eq([(&point(-128), &point(-1))]));
        assert!(inner.witness.intervals_at(0, 1).unwrap().eq([(&point(1), &point(1))]));
        assert!(inner.witness.intervals_at(0, 2).unwrap().eq([(&point(2), &point(2))]));
        assert!(outer.witness.intervals_at(0, 0).unwrap().eq([(&point(0), &point(127))]));
        assert!(outer.witness.intervals_at(0, 1).unwrap().eq([(&point(3), &point(3))]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_checker_local_writes_keep_scopes_and_correlated_lanes() {
        let app = source("src/main.w", "ordered-base-local-scoped-writes");
        let program = format!(r#"%%start
            unit(i8) state = fn(selector) {{
                i8 outer = 0;
                if (selector < 0) {{
                    i8 scratch = 1;
                    scratch = 2;
                    core.require(scratch == 2, {OUTPUT});
                    outer = 3;
                }} else {{
                    i8 scratch = 4;
                    scratch = 5;
                    core.require(scratch == 5, {OUTPUT});
                    core.require(outer == 0, {OUTPUT});
                }};
                if (selector < 0) {{
                    core.require(outer == 3, {OUTPUT});
                }} else {{
                    core.require(outer == 0, {OUTPUT});
                }};
            }};
        %%end"#);
        let run = |text: &str, scope| {
            check_loaded_scalar_ordered_local_proof_contexts(
                vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
                inputs(), scope, Vec::new(),
            ).unwrap_or_else(|error| panic!("proof-free checked local writes in {scope:?}: {error:?}"))
        };
        let full = run(&program, ScalarCheckerScope::CurrentPackage);
        assert!(full.active && full.directives.is_empty());
        let main_only = run(&program, ScalarCheckerScope::MainFlowOnly);
        assert!(!main_only.active && main_only.directives.is_empty());

        let rejected = program.replace("require(scratch == 5", "require(scratch == 4")
            .replace("require(outer == 3", "require(outer == 0");
        let observed = run(&rejected, ScalarCheckerScope::CurrentPackage);
        let [inner, after] = observed.directives.as_slice() else { panic!("two exact scoped failures"); };
        let condition = |span: ByteSpan| &rejected[span.start as usize..span.end as usize];
        assert_eq!(condition(inner.condition), "scratch == 4");
        assert_eq!(condition(after.condition), "outer == 0");
        assert_eq!(inner.witness.axes().len(), 3);
        assert_eq!(after.witness.axes().len(), 2);
        assert_eq!(inner.witness.row_count(), 1);
        assert_eq!(after.witness.row_count(), 1);
        let point = |v| BigInt::from(v);
        assert!(inner.witness.intervals_at(0, 0).unwrap().eq([(&point(0), &point(127))]));
        assert!(inner.witness.intervals_at(0, 1).unwrap().eq([(&point(0), &point(0))]));
        assert!(inner.witness.intervals_at(0, 2).unwrap().eq([(&point(5), &point(5))]));
        assert!(after.witness.intervals_at(0, 0).unwrap().eq([(&point(-128), &point(-1))]));
        assert!(after.witness.intervals_at(0, 1).unwrap().eq([(&point(3), &point(3))]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_returns_preserve_caller_partitions() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let text = format!(r#"%%start
            i8(i8) kind = fn(n) {{ if (n < 0) {{ -7 }} else {{ 6 }} }};
            i8(i8) wrapped = fn(n) {{ i8 category = kind(n); category }};
            unit(i8) unused = fn(input) {{
                i8 observed = wrapped(input);
                if (input < 0) {{ core.require(observed == -7, {}); }}
                else {{ core.require(observed == 6, {}); }};
            }};
        %%end"#, report(7), report(11));
        let run = |revision, input: &str, scope| {
            let file = source("src/main.w", revision);
            let prepared = prepare_proof_target(&file, input).unwrap();
            run_closed_without_named_proofs(&prepared, scope, 1)
        };
        let main = run("symbolic-returns-main", &text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(!main.active && main.directives.is_empty());
        let pass = run("symbolic-returns-pass", &text, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let incorrect = text.replace("observed == -7", "observed == 6");
        let fail = run("symbolic-returns-fail", &incorrect, ScalarCheckerScope::CurrentPackage).unwrap();
        let [failed] = fail.directives.as_slice() else { panic!("the negative input partition should be the only failure"); };
        assert_eq!(failed.code(), &BigInt::from(7));
        assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&BigInt::from(-128), &BigInt::from(-1))]));
        assert!(failed.witness.intervals_at(0, 1).unwrap().eq([(&BigInt::from(-7), &BigInt::from(-7))]));

    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_identity_shares_live_input_axis() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let text = format!(r#"%%start
            i8(i8) identity = fn(value) {{ value }};
            i8(i8) wrapped = fn(value) {{ i8 intermediate = identity(value); intermediate }};
            unit(i8) unused = fn(input) {{
                i8 observed = wrapped(input);
                if (input < 0) {{
                    i8 nested_alias = identity(observed);
                    core.require(nested_alias < 0, {});
                }} else {{ core.require(observed >= 0, {}); }};
                core.narrow(observed >= 0, {});
                core.require(input >= 0, {});
            }};
        %%end"#, report(7), report(11), report(13), report(17));
        let run = |revision, input: &str| {
            let app = source("src/main.w", revision);
            let prepared = prepare_proof_target(&app, input).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-identity-pass", &text).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let failure = run("symbolic-identity-fail", &text.replace("nested_alias < 0", "nested_alias >= 0")).unwrap();
        let [failed] = failure.directives.as_slice() else { panic!("only the nested alias requirement fails"); };
        assert_eq!(failed.code(), &BigInt::from(7));
        assert_eq!(failed.witness.equal_axes(), &[(1, 0), (2, 0)]);
        for axis in 0..3 {
            assert!(failed.witness.intervals_at(0, axis).unwrap().eq([(&BigInt::from(-128), &BigInt::from(-1))]));
        }
        let changed = text.replace("if (input < 0)", "observed = 0; if (input < 0)");
        let changed = run("symbolic-identity-write", &changed).unwrap();
        assert_eq!(changed.directives.len(), 2);
        let zero = BigInt::from(0);
        let [low, high] = [-128, -1].map(BigInt::from);
        for (code, axes) in [(7, 3), (17, 2)] {
            let code = BigInt::from(code);
            let failed = changed.directives.iter().find(|failed| *failed.code() == code).unwrap();
            assert!(failed.witness.equal_axes().is_empty());
            assert_eq!(failed.witness.row_count(), 1);
            assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&low, &high)]));
            for axis in 1..axes {
                assert!(failed.witness.intervals_at(0, axis).unwrap().eq([(&zero, &zero)]));
            }
        }
        let mixed = text.replace("fn(value) { value }", "fn(value) { if (value < 0) { value } else { 1 } }");
        assert!(run("symbolic-identity-mixed", &mixed).unwrap().directives.is_empty());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_mixed_returns_preserve_row_local_equality() {
        let text = format!(r#"%%start
            i8(i8) select = fn(n) {{ if (n < 0) {{ n }} else {{ -1 }} }};
            i8(i8) wrap = fn(n) {{ i8 inner = select(n); inner }};
            unit(i8) unused = fn(input) {{
                i8 observed = wrap(input);
                if (observed < -1) {{ core.require(input < -1, {OUTPUT}); }}
                else {{ core.require(observed == -1, {OUTPUT}); }};
                core.narrow(observed == -1, {OUTPUT});
                if (input < 0) {{ core.require(input == -1, {OUTPUT}); }}
                else {{ core.require(observed == -1, {OUTPUT}); }};
                core.require(observed <= 0, {OUTPUT});
            }};
        %%end"#);
        let run = |revision, text: &str| {
            let app = source("src/main.w", revision);
            let prepared = prepare_proof_target(&app, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-mixed-pass", &text).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let failed = run("symbolic-mixed-fail", &text.replace("observed <= 0", "observed > 0")).unwrap();
        let [failed] = failed.directives.as_slice() else { panic!("only the final requirement fails"); };
        assert!(failed.witness.equal_axes().is_empty());
        assert_eq!(failed.witness.row_count(), 2);
        let [minus_one, zero, max] = [-1, 0, 127].map(BigInt::from);
        assert!((0..2).any(|row| failed.witness.row_equal_axes(row) == Some(&[(1, 0)])
            && failed.witness.intervals_at(row, 0).unwrap().eq([(&minus_one, &minus_one)])));
        assert!((0..2).any(|row| failed.witness.row_equal_axes(row) == Some(&[])
            && failed.witness.intervals_at(row, 0).unwrap().eq([(&zero, &max)])));
        let assigned = text.replace("core.require(observed <= 0,", "observed = 0; core.require(observed == 0,");
        assert!(run("symbolic-mixed-write", &assigned).unwrap().directives.is_empty());

        let siblings = format!(r#"%%start
            i8(i8) identity = fn(n) {{ n }};
            unit(i8) unused = fn(input) {{
                i8 changed = identity(input);
                i8 sibling = identity(changed);
                changed = 0;
                core.require(changed == 0, {OUTPUT});
                if (sibling < 0) {{
                    i8 nested = identity(sibling);
                    core.require(input < 0, {OUTPUT});
                    core.require(nested < 0, {OUTPUT});
                }};
                core.require(changed == 0, {OUTPUT});
            }};
        %%end"#);
        assert!(run("symbolic-sibling-write", &siblings).unwrap().directives.is_empty());
        let negative = siblings.replace("input < 0", "input >= 0");
        let negative = run("symbolic-sibling-write-fail", &negative).unwrap();
        let [failed] = negative.directives.as_slice() else { panic!("the input remains negative on the sibling branch"); };
        assert_eq!(failed.witness.equal_axes(), &[(2, 0), (3, 0)]);
        assert!(failed.witness.intervals_at(0, 1).unwrap().eq([(&zero, &zero)]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_formal_writes_preserve_immutable_entry_partitions() {
        let text = format!(r#"%%start
            i8(i8) identity = fn(n) {{ n }};
            i8(i8) reset = fn(n) {{ n = 0; n }};
            unit(i8) reset_unit = fn(n) {{ n = 0; core.narrow(n == 0, {OUTPUT}); }};
            i8(i8) copied = fn(n) {{
                i8 original = identity(n);
                n = 0;
                if (original < 0) {{ original }} else {{ n }}
            }};
            unit(i8) unused = fn(input) {{
                i8 written = reset(input);
                core.require(written == 0, {OUTPUT});
                reset_unit(input);
                i8 from_copy = copied(input);
                if (input < 0) {{ core.require(from_copy < 0, {OUTPUT}); }}
                else {{ core.require(from_copy == 0, {OUTPUT}); }};
                core.require(input >= -128, {OUTPUT});
            }};
        %%end"#);
        let run = |revision, text: &str| {
            let app = source("src/main.w", revision);
            let prepared = prepare_proof_target(&app, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-formal-write-pass", &text).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let failed = run("symbolic-formal-write-fail", &text.replace("input >= -128", "input == 0")).unwrap();
        let [failed] = failed.directives.as_slice() else { panic!("caller remains unrestricted by local formal writes"); };
        let [lower, minus_one, one, upper] = [-128, -1, 1, 127].map(BigInt::from);
        let (mut negative, mut positive) = (false, false);
        for row in 0..failed.witness.row_count() {
            match failed.witness.row_equal_axes(row) {
                Some([(2, 0)]) => {
                    assert!(failed.witness.intervals_at(row, 0).unwrap().eq([(&lower, &minus_one)]));
                    negative = true;
                }
                Some([]) => {
                    assert!(failed.witness.intervals_at(row, 0).unwrap().eq([(&one, &upper)]));
                    positive = true;
                }
                _ => panic!("unexpected entry relation"),
            }
        }
        assert!(negative && positive);
        assert_eq!(failed.witness.row_count(), 2);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_projects_either_slot_beside_a_fixed_argument() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = |expression: &str| format!(r#"%%start
            i8(u8, i8) last = fn(fixed, value) {{
                if (fixed == 9) {{ if (value < 0) {{ value }} else {{ 0 }} }} else {{ 1 }}
            }};
            i8(i8, u8) first = fn(value, fixed) {{
                i8 observed = last(fixed, value);
                value = 4;
                if (observed < 0) {{ observed }} else {{ value }}
            }};
            unit(u8, i8) guard = fn(fixed, value) {{
                if (fixed == 9) {{ core.narrow(value != -2, {}); }};
            }};
            unit(i8) unused = fn(input) {{
                i8 got = first(input, 9);
                guard(9, input);
                core.require(input != -2, {});
                if (input < 0) {{ core.require(got {expression} 0, {}); }}
                else {{ core.require(got == 4, {}); }};
            }};
        %%end"#, report(5), report(7), report(11), report(13));
        let run = |revision, text: &str| {
            let file = source("src/main.w", revision);
            let prepared = prepare_proof_target(&file, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-pair-projection-pass", &program("<")).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let fail = run("symbolic-pair-projection-fail", &program(">=")).unwrap();
        let [failure] = fail.directives.as_slice() else { panic!("only the negative caller partition fails"); };
        assert_eq!(failure.code(), &BigInt::from(11));
        assert_eq!(failure.witness.row_count(), 1);
        assert_eq!(failure.witness.equal_axes(), &[(1, 0)]);
        let [lower, before, after] = [-128, -3, -1].map(BigInt::from);
        for axis in 0..2 {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(&lower, &before), (&after, &after)]));
        }

        let varied = program("<").replace("unit(i8) unused = fn(input)", "unit(i8, u8) unused = fn(input, other)")
            .replace("first(input, 9)", "first(input, other)");
        assert!(matches!(run("symbolic-pair-projection-both-varied", &varied), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_preserves_equal_and_mixed_argument_rows() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            i8(i8) identity = fn(value) {{ value }};
            i8(i8, i8) choose = fn(left, right) {{
                if (left < 0) {{ right }} else {{ 0 }}
            }};
            unit(i8) guard = fn(value) {{
                i8 copy = identity(value);
                i8 first = choose(value, value);
                i8 second = choose(value, copy);
                if (value < 0) {{
                    core.require(first < 0, {});
                    core.require(second < 0, {});
                }} else {{
                    core.require(first == 0, {});
                    core.require(second == 0, {});
                }};
            }};
        %%end"#, report(5), report(7), report(11), report(13));
        let run = |revision, text: &str| {
            let file = source("src/main.w", revision);
            let prepared = prepare_proof_target(&file, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let yes = run("symbolic-equal-pair-pass", &program).unwrap();
        assert!(yes.active && yes.directives.is_empty());
        let no = run("symbolic-equal-pair-report", &program.replace("second < 0", "second >= 0")).unwrap();
        let [failure] = no.directives.as_slice() else { panic!("only the copied argument partition fails"); };
        assert_eq!(failure.code(), &BigInt::from(7));
        assert_eq!(failure.witness.row_count(), 1);
        assert_eq!(failure.witness.equal_axes(), &[(1, 0), (2, 0), (3, 0)]);
        let [low, high] = [-128, -1].map(BigInt::from);
        for axis in 0..4 {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(&low, &high)]));
        }

        let mixed = program.replace("i8 copy = identity(value);
                i8 first = choose(value, value);",
            "i8 first = choose(value, value);
                i8 copy = identity(first);");
        let mixed_pass = run("symbolic-equal-pair-mixed", &mixed).unwrap();
        assert!(mixed_pass.active && mixed_pass.directives.is_empty());
        let mixed_fail = run("symbolic-equal-pair-mixed-report", &mixed.replace("second == 0", "second < 0")).unwrap();
        let [failure] = mixed_fail.directives.as_slice() else { panic!("only the unequal nonnegative partition fails"); };
        assert_eq!(failure.code(), &BigInt::from(13));
        assert_eq!(failure.witness.row_count(), 1);
        let [zero, maximum] = [0, 127].map(BigInt::from);
        assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&zero, &maximum)]));
        for axis in 1..4 {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(&zero, &zero)]));
        }
        let assignment = program.replace("if (left < 0) { right } else { 0 }",
            "left = 0; if (left < 0) { right } else { 0 }");
        assert!(matches!(run("symbolic-equal-pair-assignment", &assignment), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_keeps_independent_unit_argument_rows_correlated() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            i8(i8) classify = fn(n) {{ if (n < 0) {{ -1 }} else {{ 1 }} }};
            unit(i8, i8) leaf = fn(n, tag) {{
                if (tag < 0) {{ core.narrow(n != -2, {}); }};
            }};
            unit(i8, i8) wrapper = fn(tag, n) {{ leaf(n, tag); }};
            unit(i8) unused = fn(n) {{
                i8 tag = classify(n);
                wrapper(tag, n);
                core.require(n != -2, {});
                if (tag < 0) {{ core.require(n < 0, {}); }};
            }};
        %%end"#, report(5), report(11), report(13));
        let run = |revision, text: &str| {
            let file = source("src/main.w", revision);
            let prepared = prepare_proof_target(&file, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let normal = run("symbolic-independent-pair-pass", &program).unwrap();
        assert!(normal.active && normal.directives.is_empty());

        let changed = program.replace("core.require(n != -2", "core.require(n != -1");
        let result = run("symbolic-independent-pair-report", &changed).unwrap();
        let [caller] = result.directives.as_slice() else { panic!("only the surviving caller value fails"); };
        assert_eq!(caller.code(), &BigInt::from(11));
        assert_eq!(caller.witness.row_count(), 1);
        let minus_one = BigInt::from(-1);
        for axis in 0..2 {
            assert!(caller.witness.intervals_at(0, axis).unwrap().eq([(&minus_one, &minus_one)]));
        }
        let assigns = program.replace("if (tag < 0) { core.narrow",
            "tag = 0; if (tag < 0) { core.narrow");
        assert!(matches!(run("symbolic-independent-pair-write-denied", &assigns), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_transports_independent_pair_return_aliases() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            i8(i8) classify = fn(n) {{ if (n < 0) {{ -1 }} else {{ 1 }} }};
            i8(i8, i8) leaf = fn(n, tag) {{
                if (tag < 0) {{ n }} else {{ if (tag > 0) {{ tag }} else {{ 0 }} }}
            }};
            i8(i8, i8) wrapper = fn(tag, n) {{ i8 result = leaf(n, tag); result }};
            unit(i8) unused = fn(n) {{
                i8 tag = classify(n);
                i8 selected = wrapper(tag, n);
                if (tag < 0) {{
                    if (selected == -2) {{ core.require(n == -2, {}); }};
                    core.require(selected < 0, {});
                }} else {{
                    core.require(selected == 1, {});
                }};
            }};
            unit(i8, i8) positive_alias = fn(x, tag) {{
                i8 selected = wrapper(tag, x);
                if (tag > 0) {{
                    if (selected == 2) {{ core.require(tag == 2, {}); }};
                }};
            }};
        %%end"#, report(5), report(7), report(11), report(13));
        let run = |revision, text: &str| {
            let file = source("src/main.w", revision);
            let prepared = prepare_proof_target(&file, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let positive = run("symbolic-pair-result-pass", &program).unwrap();
        assert!(positive.active && positive.directives.is_empty());

        let changed = program.replace("core.require(n == -2", "core.require(n == -1");
        let negative = run("symbolic-pair-result-report", &changed).unwrap();
        let [failure] = negative.directives.as_slice() else { panic!("only the feasible pair fails"); };
        assert_eq!(failure.code(), &BigInt::from(5));
        assert_eq!(failure.witness.row_count(), 1);
        let want = [BigInt::from(-2), BigInt::from(-1), BigInt::from(-2)];
        for (axis, value) in want.iter().enumerate() {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(value, value)]));
        }

        let assigns = program.replace("if (tag < 0) { n }", "tag = 0; if (tag < 0) { n }");
        assert!(matches!(run("symbolic-pair-result-write-denied", &assigns), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_named_proof_does_not_consume_ordinary_alias_effects() {
        let app = source("src/main.w", "named-proof-ordinary-alias-firewall");
        let text = r#"%%start
            i8(i8) identity = fn(value) { value };
            i8(i8) wrapped = fn(value) { i8 intermediate = identity(value); intermediate };
            proof point for wrapped(entry) -> (out) where (entry == 7) ensures (out == 7) {}
        %%end"#;
        let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[]).ok().unwrap();
        assert!(matches!(check_loaded_ordered_proof_diagnostics(
            Vec::new(), vec![(proof, Vec::new())], vec![app], inputs(), ScalarCheckerScope::CurrentPackage,
        ), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_returns_bound_cross_axis_transfer_work() {
        for (count, admitted) in [(5, true), (10, false)] {
            let types = vec!["i8"; count].join(", ");
            let params = (0..count).map(|index| format!("p{index}")).collect::<Vec<_>>().join(", ");
            let locals = (0..count).map(|index| format!("i8 v{index} = classify(p{index}); core.require(v{index} <= 1, {OUTPUT});")).collect::<Vec<_>>().join("\n");
            let text = format!(r#"%%start
                i8(i8) classify = fn(n) {{ if (n < 0) {{ -1 }} else {{ 1 }} }};
                unit({types}) unused = fn({params}) {{ {locals} }};
            %%end"#);
            let file = source("src/main.w", &format!("symbolic-returns-budget-{count}"));
            let prepared = prepare_proof_target(&file, &text).unwrap();
            let result = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1);
            if admitted {
                let result = result.unwrap();
                assert!(result.active && result.directives.is_empty());
            } else {
                assert!(matches!(result, Err(ProjectCheck::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_computes_pair_results_from_row_local_singletons() {
        let report = OUTPUT.replace(".code = 1", ".code = 23");
        let program = format!(r#"%%start
            i8(i8) classify = fn(n) {{ if (n < 0) {{ -1 }} else {{ 1 }} }};
            i8(i8, i8) leaf = fn(n, tag) {{
                if (tag == -1) {{
                    if (n == -2) {{ n + tag }} else {{ n }}
                }} else {{ tag }}
            }};
            i8(i8, i8) wrapper = fn(tag, n) {{ i8 result = leaf(n, tag); result }};
            unit(i8) unused = fn(n) {{
                i8 tag = classify(n);
                i8 result = wrapper(tag, n);
                if (n == -2) {{ core.require(result == -3, {report}); }}
                else {{ if (tag > 0) {{ core.require(result == 1, {report}); }}; }};
            }};
        %%end"#);
        let run = |revision, text: &str| {
            let file = source("src/main.w", revision);
            let prepared = prepare_proof_target(&file, text).unwrap();
            run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-computed-pair-sum", &program).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let bad = run("symbolic-computed-pair-witness", &program.replace("result == -3", "result == 0"))
            .unwrap();
        let [failure] = bad.directives.as_slice() else { panic!("only the exact sum path fails"); };
        assert_eq!(failure.code(), &BigInt::from(23));
        assert_eq!(failure.witness.row_count(), 1);
        for (axis, value) in [-2, -1, -3].map(BigInt::from).iter().enumerate() {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(value, value)]));
        }
        assert!(matches!(run("symbolic-computed-pair-broad", &program.replace("n == -2) { n + tag", "n < 0) { n + tag")),
            Err(ProjectCheck::Unsupported)));
        assert!(matches!(run("symbolic-computed-pair-overflow", &program.replace("n + tag", "n + -127")),
            Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_computes_single_axis_returns_without_losing_entry() {
        let report = OUTPUT.replace(".code = 1", ".code = 23");
        let program = format!(r#"%%start
            i8(i8) unary = fn(n) {{
                if (n == -2) {{ n = 3; n + 1 }} else {{ n }}
            }};
            i8(u8, i8) fixed = fn(tag, n) {{
                if (tag == 9) {{ if (n == -2) {{ n - 2 }} else {{ n }} }} else {{ 1 }}
            }};
            i8(i8, i8) equal = fn(n, same) {{
                if (n == -2) {{ n - same }} else {{ n }}
            }};
            unit(i8) unused = fn(input) {{
                i8 u = unary(input);
                i8 f = fixed(9, input);
                i8 e = equal(input, input);
                if (input == -2) {{
                    core.require(u == 4, {report});
                    core.require(f == -4, {report});
                    core.require(e == 0, {report});
                }} else {{
                    if (input < 0) {{
                        core.require(u < 0, {report});
                        core.require(f < 0, {report});
                        core.require(e < 0, {report});
                    }};
                }};
            }};
        %%end"#);
        let run = |revision, text: &str| {
            let source = source("src/main.w", revision);
            let target = prepare_proof_target(&source, text).unwrap();
            run_closed_without_named_proofs(&target, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-computed-single-pass", &program).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let bad = run("symbolic-computed-single-witness", &program.replace("u == 4", "u == 3")).unwrap();
        let [failure] = bad.directives.as_slice() else { panic!("only the overridden unary result fails"); };
        assert_eq!(failure.code(), &BigInt::from(23));
        assert_eq!(failure.witness.row_count(), 1);
        for (axis, value) in [-2, 4, -4, 0].map(BigInt::from).iter().enumerate() {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(value, value)]));
        }
        assert!(matches!(run("symbolic-computed-single-broad", &program.replace(
            "if (n == -2) { n = 3; n + 1 }", "if (n < 0) { n + n }")), Err(ProjectCheck::Unsupported)));
        assert!(matches!(run("symbolic-computed-single-overflow", &program.replace("n = 3; n + 1", "n = 127; n + 1")),
            Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_affine_return_narrows_both_ways_without_filling_gaps() {
        let report = OUTPUT.replace(".code = 1", ".code = 31");
        let program = format!(r#"%%start
            i8(i8, i8) affine = fn(n, tag) {{
                if (tag < 0) {{
                    if (n >= 0) {{
                        if (n <= 4) {{ if (n != 2) {{ n + 1 }} else {{ 0 }} }} else {{ 0 }}
                    }} else {{ 0 }}
                }} else {{ 0 }}
            }};
            unit(i8, i8) unused = fn(input, tag) {{
                i8 result = affine(input, tag);
                if (tag < 0) {{ if (input == 0) {{ core.require(result == 1, {report}); }}; }};
                if (result == 5) {{
                    core.require(input == 4, {report});
                    core.require(tag < 0, {report});
                }};
                core.require(result != 3, {report});
            }};
        %%end"#);
        let run = |revision, text: &str| {
            let source = source("src/main.w", revision);
            let target = prepare_proof_target(&source, text).unwrap();
            run_closed_without_named_proofs(&target, ScalarCheckerScope::CurrentPackage, 1)
        };
        let pass = run("symbolic-affine-pass", &program).unwrap();
        assert!(pass.active && pass.directives.is_empty());
        let bad = run("symbolic-affine-witness", &program.replace("input == 4", "input == 3")).unwrap();
        let [failure] = bad.directives.as_slice() else { panic!("only the affine witness fails"); };
        assert_eq!(failure.code(), &BigInt::from(31));
        assert_eq!(failure.witness.row_count(), 1);
        let [four, minimum, negative_one, five] = [4, -128, -1, 5].map(BigInt::from);
        assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&four, &four)]));
        assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&minimum, &negative_one)]));
        assert!(failure.witness.intervals_at(0, 2).unwrap().eq([(&five, &five)]));
        assert!(matches!(run("symbolic-affine-overflow", &program.replace("n <= 4", "n <= 127")),
            Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_symbolic_call_returns_resume_surviving_terminal_arm() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        for first_empty in [true, false] {
            let prune = "prune(); -9".to_owned();
            let keep = format!("core.require(false, {}); 7", report(5));
            let (yes, no) = if first_empty { (prune, keep) } else { (keep, prune) };
            let text = format!(r#"%%start
                unit() prune = fn {{ core.narrow(false, {}); }};
                i8(i8) select = fn(n) {{
                    if (n < 0) {{ {yes} }} else {{ {no} }}
                }};
                unit(i8) unused = fn(n) {{
                    i8 value = select(n);
                    core.require(value == 7, {});
                }};
            %%end"#, report(3), report(7));
            let file = source("src/main.w", &format!("symbolic-returns-narrowed-{first_empty}"));
            let prepared = prepare_proof_target(&file, &text).unwrap();
            let result = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
            let expected = if first_empty { [3, 5] } else { [5, 3] }.map(BigInt::from);
            assert!(result.active);
            let [prune_root, reached @ ..] = result.directives.as_slice() else { panic!("expected root and reached call diagnostics"); };
            assert_eq!(prune_root.code(), &BigInt::from(3));
            assert!(reached.iter().map(|failure| failure.code()).eq(expected.iter().cycle().take(4)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_exact_call_continuations_bind_return_values_in_order() {
        let report = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let text = format!(r#"%%start
            i8(i8) leaf = fn(n) {{
                core.require(n >= 0, {});
                if (n < 0) {{ -7 }} else {{ n }}
            }};
            i8(i8) middle = fn(n) {{
                i8 answer = leaf(n);
                core.require(answer > -6, {});
                answer
            }};
            i8(i8) outer = fn(n) {{
                if (n < 0) {{ i8 x = middle(n); x }} else {{ i8 y = middle(n); y }}
            }};
            i8 low = outer(-3);
            core.require(low == -8, {});
            i8 high = outer(3);
            core.require(high == 3, {});
        %%end"#, report(5), report(7), report(11), report(13));
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let file = source("src/main.w", "ordered-exact-return-chain");
            let prepared = prepare_proof_target(&file, &text).unwrap();
            let result = run_closed_without_named_proofs(&prepared, scope, 1).unwrap();
            assert!(result.active);
            let expected = [5, 7, 11].map(BigInt::from);
            assert!(result.directives.iter().map(|failure| failure.code()).eq(expected.iter()));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_exact_call_continuations_bound_acyclic_return_frames() {
        for (depth, admitted) in [(MAX_ACTUAL_CALL_DEPTH, true), (MAX_ACTUAL_CALL_DEPTH + 1, false)] {
            let mut program = "%%start\nu8(u8) f0 = fn(n) { n };\n".to_owned();
            for index in 1..depth {
                program.push_str(&format!("u8(u8) f{index} = fn(n) {{ u8 value{index} = f{}(n); value{index} }};\n", index - 1));
            }
            program.push_str(&format!("u8 result = f{}(1);\ncore.require(result == 1, {OUTPUT});\n%%end", depth - 1));
            let file = source("src/main.w", &format!("ordered-exact-return-depth-{depth}"));
            let prepared = prepare_proof_target(&file, &program).unwrap();
            let result = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::MainFlowOnly, 1);
            if admitted {
                let result = result.unwrap();
                assert!(result.active && result.directives.is_empty());
            } else {
                assert!(matches!(result, Err(ProjectCheck::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_actual_value_call_collects_require_then_uses_exact_return() {
        let app = source("src/main.w", "ordered-base-value-require");
        let report = |code: i32| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            i8(i8) evaluate = fn(input) {{
                core.require(input > 0, {});
                if (input < 0) {{ -7 }} else {{ input }}
            }};
            i8 first = evaluate(-3);
            core.require(first == -8, {});
            i8 second = evaluate(-3);
            core.require(second == -7, {});
            i8 third = evaluate(3);
            core.require(third == 3, {});
        %%end"#, report(7), report(13), report(17), report(23));
        let run = |text: &str, scope| {
            check_loaded_scalar_ordered_local_proof_contexts(
                vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
                inputs(), scope, Vec::new(),
            )
        };
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let receipt = run(&program, scope).unwrap_or_else(|error| panic!("proof-free exact actual calls in {scope:?}: {error:?}"));
            assert!(receipt.active && receipt.diagnostics.is_empty() && receipt.symbolic_diagnostic.is_none());
            let [first, caller, second] = receipt.directives.as_slice() else { panic!("two actual callee records with one caller record in between"); };
            let text = |span: ByteSpan| &program[span.start as usize..span.end as usize];
            assert_eq!(text(first.condition), "input > 0");
            assert_eq!(text(caller.condition), "first == -8");
            assert_eq!(text(second.condition), "input > 0");
            assert_eq!(first.code(), &BigInt::from(7));
            assert_eq!(caller.code(), &BigInt::from(13));
            assert_eq!(second.code(), &BigInt::from(7));
            assert_eq!(first.directive, second.directive);
            let entry = BigInt::from(-3);
            assert!(first.witness.intervals_at(0, 0).unwrap().eq([(&entry, &entry)]));
            assert_eq!(first.witness, second.witness);

            let incomplete = program.replace("core.require(input > 0", "core.narrow(input > 0")
                .replace("i8 first = evaluate(-3);", &format!("core.require(false, {}); i8 first = evaluate(-3);", report(31)));
            assert!(matches!(run(&incomplete, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_contextual_signed_initializer_keeps_exact_minimum_and_fails_closed() {
        let app = source("src/main.w", "ordered-contextual-negative-initializer");
        let program = format!(r#"%%start
            unit(i8) test_value = fn(entry) {{
                core.require(entry > 0, {OUTPUT});
                i8 core.context.count = -128;
                core.require(core.context.count == -127, {OUTPUT});
            }};
            test_value(1);
        %%end"#);
        let run = |text: &str, scope| check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), scope, Vec::new(),
        );
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let result = run(&program, scope).unwrap_or_else(|error| panic!("{scope:?}: {error:?}"));
            let [failure] = result.directives.as_slice() else { panic!("expected one authored requirement"); };
            let minimum = BigInt::from(i8::MIN);
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&minimum, &minimum)]));

            let expression = program.replace("= -128;", "= 0 - 3;");
            assert!(matches!(run(&expression, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let out_of_range = program.replace("= -128;", "= -129;");
            assert!(matches!(run(&out_of_range, scope), Err(ScalarOrderedCheckerOutcome::Invalid(_))));
            let unsigned = program.replace("i8 core.context.count = -128;", "u8 core.context.count = -1;")
                .replace("core.context.count == -127", "core.context.count == 1");
            assert!(matches!(run(&unsigned, scope), Err(ScalarOrderedCheckerOutcome::Invalid(_))));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_contextual_signed_overwrite_changes_only_the_active_place() {
        let app = source("src/main.w", "ordered-contextual-negative-overwrite");
        let program = format!(r#"%%start
            unit(i8) test_value = fn(entry) {{
                core.require(entry > 0, {OUTPUT});
                i8 core.context.count = 1;
                if (entry > 0) {{
                    i8 core.context.count = core.context.count;
                    core.context.count = -128;
                    core.require(core.context.count == -127, {OUTPUT});
                }};
                core.require(core.context.count == 1, {OUTPUT});
            }};
            test_value(1);
        %%end"#);
        let run = |text: &str, scope| check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), scope, Vec::new(),
        );
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let result = run(&program, scope).unwrap_or_else(|error| panic!("{scope:?}: {error:?}"));
            let [failure] = result.directives.as_slice() else { panic!("expected one authored requirement"); };
            let minimum = BigInt::from(i8::MIN);
            let original = BigInt::from(1);
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&original, &original)]));
            assert!(failure.witness.intervals_at(0, 2).unwrap().eq([(&minimum, &minimum)]));

            let expression = program.replace("= -128;", "= 0 - 3;");
            assert!(matches!(run(&expression, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let out_of_range = program.replace("= -128;", "= -129;");
            assert!(matches!(run(&out_of_range, scope), Err(ScalarOrderedCheckerOutcome::Invalid(_))));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_contextual_read_snapshots_the_active_place_into_an_ordinary_local() {
        let app = source("src/main.w", "ordered-contextual-read-local");
        let report = |code: i32| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            unit(i8) test_value = fn(entry) {{
                core.require(entry > 0, {});
                i8 core.context.count = 1;
                if (entry > 0) {{
                    i8 core.context.count = -128;
                    i8 saved = core.context.count;
                    core.context.count = 7;
                    core.require(core.context.count == 7, {});
                    core.require(saved == 7, {});
                }};
                core.require(core.context.count == 1, {});
            }};
            test_value(1);
        %%end"#, report(5), report(7), report(11), report(17));
        let run = |text: &str, scope| check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), scope, Vec::new(),
        );
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let (prepared, startup_count) = prepare_checked_package_scope(
                vec![(declarations(&app, &program), Vec::new())], vec![app.clone()], Vec::new(), inputs(), scope,
            ).unwrap();
            assert!(matches!(
                run_prepared_ordered_check(&prepared, scope, startup_count, OrderedReachPolicy::Closed, ()),
                Err(ProjectCheck::Unsupported)
            ));
            let receipt = run(&program, scope).unwrap_or_else(|error| panic!("{scope:?}: {error:?}"));
            let [failure] = receipt.directives.as_slice() else { panic!("expected one authored requirement"); };
            assert_eq!(failure.code(), &BigInt::from(11));
            let original = BigInt::from(1);
            let overwritten = BigInt::from(7);
            let saved = BigInt::from(i8::MIN);
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&original, &original)]));
            assert!(failure.witness.intervals_at(0, 2).unwrap().eq([(&overwritten, &overwritten)]));
            assert!(failure.witness.intervals_at(0, 3).unwrap().eq([(&saved, &saved)]));

            let ordinary = program.replace("i8 saved = core.context.count;", "i8 saved = entry;");
            let receipt = run(&ordinary, scope).unwrap_or_else(|error| panic!("{scope:?}: {error:?}"));
            let [failure] = receipt.directives.as_slice() else { panic!("expected the lexical snapshot failure"); };
            assert_eq!(failure.code(), &BigInt::from(11));
            assert!(failure.witness.intervals_at(0, 3).unwrap().eq([(&original, &original)]));
            let different_type = program.replace("i8 saved = core.context.count;", "u8 saved = core.context.count;");
            assert!(matches!(run(&different_type, scope), Err(ScalarOrderedCheckerOutcome::Invalid(_))));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_contextual_integer_write_restores_the_active_place_from_its_snapshot() {
        let app = source("src/main.w", "ordered-contextual-restore-local");
        let report = |code: i32| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            unit(i8) test_value = fn(entry) {{
                i8 core.context.count = 1;
                if (entry > 0) {{
                    i8 core.context.count = -128;
                    i8 saved = core.context.count;
                    core.context.count = 7;
                    core.context.count = saved;
                    core.require(core.context.count == -128, {});
                    core.require(core.context.count == 7, {});
                }};
                core.require(core.context.count == 1, {});
            }};
            test_value(1);
        %%end"#, report(7), report(11), report(17));
        let run = |text: &str, scope| check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), scope, Vec::new(),
        );
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let receipt = run(&program, scope).unwrap_or_else(|error| panic!("{scope:?}: {error:?}"));
            let [failure] = receipt.directives.as_slice() else { panic!("expected one authored requirement"); };
            assert_eq!(failure.code(), &BigInt::from(11));
            let outer = BigInt::from(1);
            let restored = BigInt::from(i8::MIN);
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&outer, &outer)]));
            assert!(failure.witness.intervals_at(0, 2).unwrap().eq([(&restored, &restored)]));
            assert!(failure.witness.intervals_at(0, 3).unwrap().eq([(&restored, &restored)]));

            let through_exit = program
                .replace("core.context.count == 7,", "core.context.count != 7,")
                .replace("core.context.count == 1,", "core.context.count == -128,");
            let receipt = run(&through_exit, scope).unwrap_or_else(|error| panic!("{scope:?}: {error:?}"));
            let [failure] = receipt.directives.as_slice() else { panic!("expected only the outer requirement"); };
            assert_eq!(failure.code(), &BigInt::from(17));
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&outer, &outer)]));
            assert!(failure.witness.intervals_at(0, 2).is_none());

            let literal_local = program.replace("i8 saved = core.context.count;", "i8 saved = -128;");
            let (prepared, startup_count) = prepare_checked_package_scope(
                vec![(declarations(&app, &literal_local), Vec::new())], vec![app.clone()], Vec::new(), inputs(), scope,
            ).unwrap();
            assert!(matches!(
                run_prepared_ordered_check(&prepared, scope, startup_count, OrderedReachPolicy::Closed, ()),
                Err(ProjectCheck::Unsupported)
            ));

            let ordinary_destination = program.replace(
                "core.context.count = saved;", "i8 ordinary = 0; ordinary = saved;",
            );
            assert!(matches!(run(&ordinary_destination, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));

            let different_type = literal_local.replace("i8 saved = -128;", "u8 saved = 128;");
            assert!(matches!(run(&different_type, scope), Err(ScalarOrderedCheckerOutcome::Invalid(_))));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn prepared_lexical_copy_has_an_independent_origin_and_checked_source() {
        use super::super::reference::PlainNameIdentity;
        let app = source("src/main.w", "lexical-copy-origin");
        let program = format!(r#"%%start
            unit(i8) snapshot = fn(entry) {{
                i8 first = entry;
                i8 second = first;
                core.require(second > 0, {OUTPUT});
            }};
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &program), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let (project, calls) = prepared.parts();
        let (module, item, _) = select_checked_root(project, &app, "snapshot").unwrap();
        let body = calls.function_body(module, item, None).unwrap();
        let key = |index| (body.structure.root, index);
        let copies = &body.structure.checker_ordinary_scalar_locals;
        let first = &copies[&key(0)];
        let second = &copies[&key(1)];
        assert_eq!(body.prepared_plain_name(first.source),
            Some(&PlainNameIdentity::Lexical(body.boundary.checker_parameter_bindings.as_ref().unwrap()[0].clone())));
        assert_eq!(body.prepared_plain_name(second.source), Some(&PlainNameIdentity::Lexical(first.binding.clone())));
        assert!(!body.structure.checker_integer_locals.contains_key(&key(0)));
        assert!(!body.structure.checker_integer_call_locals.contains_key(&key(0)));
        assert!(root_local_import_requirement(body).is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn staged_bool_uses_an_ordinary_local_and_separate_literal_receipt() {
        use super::super::reference::CheckerOrdinaryScalarLocalKind;
        let app = source("src/main.w", "staged-bool-origin");
        let program = format!(r#"%%start
            bool(i8,i8) choose = fn(value, bound) {{ if (value < bound) {{ true }} else {{ false }} }};
            unit(i8) check = fn(input) {{
                bool decision = choose(input, -1);
                bool other = choose(1, -1);
                core.require(decision || other, {OUTPUT});
            }};
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &program), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let (project, calls) = prepared.parts();
        let (module, item, _) = select_checked_root(project, &app, "check").unwrap();
        let body = calls.function_body(module, item, None).unwrap();
        let origin = &body.structure.checker_ordinary_scalar_locals[&(body.structure.root, 0)];
        assert!(matches!(&origin.kind, CheckerOrdinaryScalarLocalKind::BoolCall(call) if *call == origin.source));
        assert!(body.structure.checker_integer_call_locals.is_empty());
        assert_eq!(body.structure.checker_integer_literals.len(), 1);
        assert!(body.structure.checker_integer_literals.values().all(|literal| literal == &BigInt::from(1)));
        assert_eq!(body.structure.checker_ordinary_call_integer_literals.len(), 2);
        assert!(body.structure.checker_ordinary_call_integer_literals.values().all(|literal| literal == &BigInt::from(-1)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn prepared_inline_arithmetic_isolated_from_named_integer_evidence() {
        use super::super::reference::{CheckerOrdinaryScalarLocalKind, CheckerOrdinaryIntegerOperand, PlainNameIdentity};
        let app = source("src/main.w", "inline-arithmetic-origin");
        let program = format!(r#"%%start
            unit(i8) use_math = fn(entry) {{
                i8 flipped = -2 - entry;
                i8 adjusted = flipped + -1;
                core.require(adjusted > 0, {OUTPUT});
            }};
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &program), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let (project, calls) = prepared.parts();
        let (module, item, _) = select_checked_root(project, &app, "use_math").unwrap();
        let body = calls.function_body(module, item, None).unwrap();
        let key = |index| (body.structure.root, index);
        let locals = &body.structure.checker_ordinary_scalar_locals;
        let first = &locals[&key(0)];
        let second = &locals[&key(1)];
        let CheckerOrdinaryScalarLocalKind::Arithmetic {
            operator: super::super::BinaryOperator::Subtract,
            operands: [CheckerOrdinaryIntegerOperand::Literal(first_literal), CheckerOrdinaryIntegerOperand::Lexical(entry)],
        } = &first.kind else { panic!("missing checked signed subtraction"); };
        let CheckerOrdinaryScalarLocalKind::Arithmetic {
            operator: super::super::BinaryOperator::Add,
            operands: [CheckerOrdinaryIntegerOperand::Lexical(flipped), CheckerOrdinaryIntegerOperand::Literal(second_literal)],
        } = &second.kind else { panic!("missing checked signed addition"); };
        assert_eq!(first_literal, &BigInt::from(-2));
        assert_eq!(second_literal, &BigInt::from(-1));
        assert_eq!(body.prepared_plain_name(*entry), Some(&PlainNameIdentity::Lexical(
            body.boundary.checker_parameter_bindings.as_ref().unwrap()[0].clone())));
        assert_eq!(body.prepared_plain_name(*flipped), Some(&PlainNameIdentity::Lexical(first.binding.clone())));
        assert!(body.structure.checker_integer_final_operations.is_empty());
        assert!(body.structure.checker_integer_locals.is_empty());
        assert!(body.structure.checker_integer_call_locals.is_empty());
        assert!(root_local_import_requirement(body).is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn contextual_read_local_does_not_widen_named_bound_import_prefix() {
        let app = source("src/main.w", "contextual-local-import-prefix");
        for (saved, allowed) in [("3", true), ("core.context.count", false)] {
            let program = format!(r#"%%start
                u8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};
                u8() caller = fn {{
                    u8 core.context.count = 3;
                    u8 saved = {saved};
                    u8 result = clamp(7);
                    core.require(result <= 10, {OUTPUT});
                    result + saved + core.context.count
                }};
            %%end"#);
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &program), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            let (project, calls) = prepared.parts();
            let (module, item, _) = select_checked_root(project, &app, "caller").unwrap();
            let body = calls.function_body(module, item, None).unwrap();
            let key = (body.structure.root, 1);
            assert_eq!(body.structure.checker_integer_locals.contains_key(&key), allowed);
            assert_eq!(body.structure.checker_contextual_read_locals.contains_key(&key), !allowed);
            assert_eq!(root_local_import_requirement(body).map(|(position, _)| position), allowed.then_some(2));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn nominal_state_capture_leaves_only_lexical_coordinates_in_public_witness() {
        let app = source("src/main.w", "nominal-state-public-witness");
        let program = r#"%%start
            struct Snapshot { i8 saved; }
            unit(i8) check = fn(value) {
                Snapshot snapshot = { .saved = value; };
                value = 7;
                core.require(value != 7, { .stdout = ""; .stderr = "failure"; .code = 1; .files = [
                    { .path = "state.json"; .state = snapshot; }
                ]; });
            };
        %%end"#;
        let receipt = check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, program), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), ScalarCheckerScope::CurrentPackage, Vec::new(),
        ).unwrap();
        let [failure] = receipt.directives.as_slice() else { panic!("expected one failure"); };
        assert_eq!(failure.witness.axes().len(), 1);
        let seven = BigInt::from(7u8);
        assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
        assert!(failure.witness.intervals_at(0, 1).is_none());
        assert!(failure.witness.equal_axes().is_empty());
        assert!(failure.witness.row_equal_axes(0).unwrap().is_empty());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn authored_scalar_state_preserves_exact_types_and_disjoint_alternatives() {
        let maximum = BigInt::from(u128::MAX);
        let singleton = IntegerSet::singleton(&maximum);
        assert_eq!(serialize_scalar_analysis_state(&ScalarType::U128, &singleton).unwrap(),
            "{\"type\":{\"kind\":\"u128\"},\"value\":340282366920938463463374607431768211455}\n");
        let boolean = IntegerSet::singleton(&BigInt::from(0u8));
        assert_eq!(serialize_scalar_analysis_state(&ScalarType::Bool, &boolean).unwrap(),
            "{\"type\":{\"kind\":\"bool\"},\"value\":false}\n");
        let disjoint = boolean.join(IntegerSet::singleton(&BigInt::from(2u8)));
        assert_eq!(serialize_scalar_analysis_state(&ScalarType::U8, &disjoint).unwrap().trim_end(),
            r#"{"type":{"kind":"u8"},"value":{"kind":"alternatives","values":[0,2]}}"#);
        assert!(serialize_scalar_analysis_state(&ScalarType::Bool, &disjoint).is_none());
        assert_eq!(serialize_scalar_analysis_state(&ScalarType::Bool, &IntegerSet::ordinary_bool_domain()).unwrap().trim_end(),
            r#"{"type":{"kind":"bool"},"value":{"kind":"alternatives","values":[false,true]}}"#);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn scoped_context_declaration_is_prepared_without_a_context_read() {
        let app = source("src/main.w", "context-declaration-only");
        let program = format!(r#"%%start
            unit(i8) check = fn(input) {{
                i8 core.context.count = input;
                core.require(input >= 0, {OUTPUT});
            }};
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &program), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let (project, calls) = prepared.parts();
        let (module, item, _) = select_checked_root(project, &app, "check").unwrap();
        let body = calls.function_body(module, item, None).unwrap();
        let structure = &body.structure;
        assert!(structure.checker_contextual_occurrences.is_empty());
        assert!(structure.checker_contextual_read_locals.is_empty());
        assert!(structure.checker_integer_call_locals.is_empty());
        assert!(structure.checker_ordinary_scalar_locals.is_empty());
        assert!(structure.checker_contextual_declarations.contains(&(structure.root, 0)));
        assert!(structure.has_ordinary_contextual_places());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordinary_contextual_projection_preserves_active_scope_and_witness_identity() {
        let app = source("src/main.w", "ordinary-context-projection");
        let inner = OUTPUT.replace(".code = 1", ".code = 51");
        let outer = OUTPUT.replace(".code = 1", ".code = 52");
        let program = format!(r#"%%start
            unit(i8) keep = fn(entry) {{ core.narrow(entry >= 0, {OUTPUT}); }};
            unit(i8) caller = fn(input) {{
                i8 core.context.count = input;
                keep(core.context.count);
                core.require(input >= 0, {OUTPUT});
                if (input == 2) {{
                    i8 core.context.count = core.context.count;
                    core.context.count = 7;
                    core.require(core.context.count == 2, {inner});
                }};
                if (input == 2) {{ core.require(core.context.count == 7, {outer}); }};
            }};
            keep(0);
        %%end"#);
        let receipt = check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, &program), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), ScalarCheckerScope::CurrentPackage, Vec::new(),
        ).unwrap();
        let [first, second] = receipt.directives.as_slice() else { panic!("expected the inner and restored outer witnesses"); };
        assert_eq!(first.code(), &BigInt::from(51));
        assert_eq!(second.code(), &BigInt::from(52));
        let inner_axes = first.witness.axes().collect::<Vec<_>>();
        let outer_axes = second.witness.axes().collect::<Vec<_>>();
        assert_eq!((inner_axes.len(), outer_axes.len()), (3, 2));
        assert_ne!(inner_axes[1], inner_axes[2]);
        assert_eq!(inner_axes[1], outer_axes[1]);
        let two = BigInt::from(2);
        let seven = BigInt::from(7);
        assert!(first.witness.intervals_at(0, 2).unwrap().eq([(&seven, &seven)]));
        assert!(second.witness.intervals_at(0, 1).unwrap().eq([(&two, &two)]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_call_copies_exact_contextual_integer_into_callee_parameter() {
        let app = source("src/main.w", "ordered-contextual-call-argument");
        let report = |code: i32| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let program = format!(r#"%%start
            i8(i8) evaluate = fn(input) {{
                core.require(input < 0, {});
                if (input > 0) {{ 7 }} else {{ input }}
            }};
            unit(i8) caller = fn(entry) {{
                core.require(entry > 0, {});
                i8 core.context.count = 3;
                i8 result = evaluate(core.context.count);
                core.require(result == 8, {});
                core.require(core.context.count == 3, {});
            }};
            caller(1);
        %%end"#, report(7), report(5), report(13), report(17));
        let run = |text: &str, scope| check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, text), Vec::new())], Vec::new(), vec![app.clone()],
            inputs(), scope, Vec::new(),
        );
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let receipt = run(&program, scope)
                .unwrap_or_else(|error| panic!("proof-free contextual arguments in {scope:?}: {error:?}"));
            assert!(receipt.active && receipt.diagnostics.is_empty() && receipt.symbolic_diagnostic.is_none());
            let [callee, caller] = receipt.directives.as_slice() else { panic!("expected callee then caller"); };
            let text = |span: ByteSpan| &program[span.start as usize..span.end as usize];
            assert_eq!(text(callee.condition), "input < 0");
            assert_eq!(text(caller.condition), "result == 8");
            assert_eq!(callee.code(), &BigInt::from(7));
            assert_eq!(caller.code(), &BigInt::from(13));
            let input = BigInt::from(3);
            let result = BigInt::from(7);
            assert!(callee.witness.intervals_at(0, 0).unwrap().eq([(&input, &input)]));
            assert!(caller.witness.intervals_at(0, 1).unwrap().eq([(&input, &input)]));
            assert!(caller.witness.intervals_at(0, 2).unwrap().eq([(&result, &result)]));

            let narrowing = program.replace("core.require(input < 0", "core.narrow(input < 0");
            assert!(matches!(run(&narrowing, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let ordinary = program.replace("core.context.count", "count");
            let receipt = run(&ordinary, scope)
                .unwrap_or_else(|error| panic!("proof-free lexical argument in {scope:?}: {error:?}"));
            assert!(receipt.active && receipt.diagnostics.is_empty() && receipt.symbolic_diagnostic.is_none());
            let [callee, caller] = receipt.directives.as_slice() else { panic!("expected lexical callee then caller"); };
            assert_eq!(callee.code(), &BigInt::from(7));
            assert_eq!(caller.code(), &BigInt::from(13));
            assert!(callee.witness.intervals_at(0, 0).unwrap().eq([(&input, &input)]));
            assert!(caller.witness.intervals_at(0, 1).unwrap().eq([(&input, &input)]));
            assert!(caller.witness.intervals_at(0, 2).unwrap().eq([(&result, &result)]));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_failure_collection_does_not_publish_incomplete_roots() {
        let app = source("src/main.w", "collect-incomplete-package-failures");
        let prepare = |body: &str, startup: &str| {
            let text = format!(r#"%%start
                unit() helper = fn {{ helper(); }};
                unit(u8) unopened = fn(value) {{ core.require(value < 4, {OUTPUT}); {body} }};
                core.require(false, {OUTPUT});
                {startup}
            %%end"#);
            prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap()
        };
        let nested = prepare("helper();", "");
        assert_eq!(collect_ordered_package_failures(&nested, ScalarCheckerScope::CurrentPackage, 1), Err(ProjectCheck::Unsupported));
        let dead = prepare("", &format!("core.narrow(false, {OUTPUT});"));
        assert_eq!(collect_ordered_package_failures(&dead, ScalarCheckerScope::CurrentPackage, 1), Err(ProjectCheck::Unsupported));
        let (active, main_only) = collect_ordered_package_failures(&dead, ScalarCheckerScope::MainFlowOnly, 1).unwrap();
        assert!(active);
        assert_eq!(main_only.len(), 2);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_failure_collection_accepts_a_fatal_actual_call_without_pending_roots() {
        let app = source("src/main.w", "collect-ended-actual-root");
        let child = source("src/child.w", "collect-unvisited-child-root");
        let prepare = |unseen: bool| {
            let namespace = if unseen { "child = namespace demo \"src/child.w\";" } else { "" };
            let text = format!(r#"%%start
                unit(u8) actual = fn(value) {{ core.narrow(value < 2, {OUTPUT}); }};
                actual(3);
                {namespace}
                core.require(false, {OUTPUT});
            %%end"#);
            let mut sources = vec![app.clone()];
            let mut bindings = Vec::new();
            if unseen {
                let start = text.find(namespace).unwrap();
                bindings.push(ScalarNamespaceBinding {
                    binding: "child".into(), target: child.clone(),
                    span: ByteSpan::new(start as u32, (start + namespace.len()) as u32),
                });
                sources.push(child.clone());
            }
            let mut items = vec![(declarations(&app, &text), bindings)];
            if unseen {
                let child_text = format!(r#"%%start
                    unit(u8) pending = fn(value) {{ core.require(value == 0, {OUTPUT}); }};
                    core.require(false, {OUTPUT});
                %%end"#);
                items.push((declarations(&child, &child_text), Vec::new()));
            }
            prepare_checker_project(items, sources, inputs()).unwrap()
        };
        let prepare_without_pending = prepare(false);
        let (active, failures) = collect_ordered_package_failures(
            &prepare_without_pending, ScalarCheckerScope::CurrentPackage, 1,
        ).unwrap();
        assert!(active);
        assert!(matches!(failures.as_slice(), [ProjectCheck::Failed {
            operation: CheckerOperation::Narrow, source, ..
        }] if source == &app));

        let prepare_with_unseen_pending = prepare(true);
        assert_eq!(collect_ordered_package_failures(
            &prepare_with_unseen_pending, ScalarCheckerScope::CurrentPackage, 2,
        ), Err(ProjectCheck::Unsupported));
        assert_eq!(collect_ordered_package_failures(
            &prepare_with_unseen_pending, ScalarCheckerScope::MainFlowOnly, 2,
        ), Ok((active, failures)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_nested_calls_preserve_their_independent_canonical_contexts() {
        let app = source("src/main.w", "collect-nested-symbolic-roots");
        let prepare = |startup: &str, entry_filter: &str, nested: &str| {
            let text = format!(r#"%%start
                unit(u8) leaf = fn(value) {{
                    core.require(value < 2, {OUTPUT});
                    core.narrow(value < 4, {OUTPUT});
                }};
                unit(u8) unused = fn(value) {{
                    core.require(value < 3, {OUTPUT});
                    {entry_filter}
                    leaf({nested});
                    core.require(value < 4, {OUTPUT});
                }};
                {startup}
            %%end"#);
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, prepared)
        };
        let bounds = |start, end| vec![(BigInt::from(start), BigInt::from(end))];
        let receipt = |text: &str, prepared: &PreparedCheckerProject| {
            let (active, reports) = collect_ordered_package_failures(prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
            assert!(active);
            reports.into_iter().map(|report| {
                let ProjectCheck::Failed { condition, state, operation, .. } = report else { panic!("expected failure"); };
                let intervals: Vec<_> = state.intervals_at(0, 0).unwrap().map(|(lo, hi)| (lo.clone(), hi.clone())).collect();
                (text[condition.start as usize..condition.end as usize].to_owned(), operation, intervals)
            }).collect::<Vec<_>>()
        };
        use CheckerOperation::{Narrow, Require};
        let retain_three = format!("core.narrow(value == 3, {OUTPUT});");
        let (text, prepared) = prepare("", &retain_three, "value");
        assert_eq!(receipt(&text, &prepared), [
            ("value < 2".into(), Require, bounds(2, 255)),
            ("value < 3".into(), Require, bounds(3, 255)),
            ("value < 2".into(), Require, bounds(3, 3)),
        ]);
        let (text, prepared) = prepare("leaf(1);", &retain_three, "value");
        assert_eq!(receipt(&text, &prepared), [
            ("value < 3".into(), Require, bounds(3, 255)),
            ("value < 2".into(), Require, bounds(3, 3)),
        ]);
        let (text, prepared) = prepare("leaf(1);", "", "3");
        assert_eq!(receipt(&text, &prepared), [
            ("value < 3".into(), Require, bounds(3, 255)),
            ("value < 2".into(), Require, bounds(3, 3)),
            ("value < 4".into(), Require, bounds(4, 255)),
        ]);
        let (text, prepared) = prepare("leaf(1);", "", "4");
        assert_eq!(receipt(&text, &prepared), [
            ("value < 3".into(), Require, bounds(3, 255)),
            ("value < 2".into(), Require, bounds(4, 4)),
            ("value < 4".into(), Narrow, bounds(4, 4)),
        ]);
        let (text, prepared) = prepare("leaf(1);", "", "value");
        assert_eq!(receipt(&text, &prepared), [
            ("value < 3".into(), Require, bounds(3, 255)),
            ("value < 2".into(), Require, bounds(2, 255)),
        ]);
        let (_, prepared) = prepare("leaf(1);", "", "value + 1");
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 1),
            Err(ProjectCheck::Unsupported));
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 1),
            Ok((true, Vec::new())));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_exact_calls_use_completed_namespaces_without_marking_actual() {
        let app = source("src/main.w", "collect-symbolic-namespace-root");
        let child = source("src/child.w", "collect-symbolic-namespace-child");
        let prepare = |startup: &str, guard: &str| {
            let root_text = format!(r#"%%start
                lib = namespace demo "src/child.w";
                unit(u8) wrapper = fn(value) {{
                    core.require(value < 3, {OUTPUT});
                    {guard}
                    lib.leaf(value);
                    core.require(value != 3, {OUTPUT});
                }};
                {startup}
            %%end"#);
            let child_text = format!(r#"%%start
                core.require(true, {OUTPUT});
                unit(u8) leaf = fn(value) {{ core.require(value < 2, {OUTPUT}); }};
            %%end"#);
            let start = root_text.find("lib =").unwrap();
            let end = start + root_text[start..].find(';').unwrap() + 1;
            let prepared = prepare_checker_project(
                vec![
                    (declarations(&app, &root_text), vec![ScalarNamespaceBinding {
                        binding: "lib".into(), target: child.clone(), span: ByteSpan::new(start as u32, end as u32),
                    }]),
                    (declarations(&child, &child_text), Vec::new()),
                ],
                vec![app.clone(), child.clone()],
                inputs(),
            ).unwrap();
            (prepared, root_text, child_text)
        };
        let read = |prepared: &PreparedCheckerProject, root_text: &str, child_text: &str| {
            let (active, reports) = collect_ordered_package_failures(prepared, ScalarCheckerScope::CurrentPackage, 2)
                .expect("project with completed same-package namespace");
            assert!(active);
            reports.iter().map(|report| {
                let ProjectCheck::Failed { source, condition, axes, state, .. } = report else { panic!("expected authored report"); };
                assert_eq!(axes.len(), 1);
                assert_eq!(&axes[0].0.source, source);
                let mut values = state.intervals_at(0, 0).unwrap();
                let (lo, hi) = values.next().unwrap();
                assert!(values.next().is_none());
                let from_root = source == &app;
                assert!(from_root || source == &child);
                let text = if from_root { root_text } else { child_text };
                (from_root, text[condition.start as usize..condition.end as usize].to_owned(), lo.clone(), hi.clone())
            }).collect::<Vec<_>>()
        };
        let keep = format!("core.narrow(value == 3, {OUTPUT});");
        let expected = |from_root, text: &str, lo, hi| (from_root, text.to_owned(), BigInt::from(lo), BigInt::from(hi));
        let (prepared, root, lib) = prepare("", &keep);
        assert_eq!(read(&prepared, &root, &lib), [
            expected(true, "value < 3", 3, 255),
            expected(false, "value < 2", 3, 3),
            expected(true, "value != 3", 3, 3),
            expected(false, "value < 2", 2, 255),
        ]);
        let (prepared, root, lib) = prepare("lib.leaf(1);", &keep);
        assert_eq!(read(&prepared, &root, &lib), [
            expected(true, "value < 3", 3, 255),
            expected(false, "value < 2", 3, 3),
            expected(true, "value != 3", 3, 3),
        ]);
        let (prepared, _, _) = prepare("lib.leaf(1);", "");
        use super::super::checker_flow::PROJECTED_ARGUMENT_MATERIALIZATIONS;
        let before = PROJECTED_ARGUMENT_MATERIALIZATIONS.get();
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 2),
            Err(ProjectCheck::Unsupported));
        assert_eq!(PROJECTED_ARGUMENT_MATERIALIZATIONS.get(), before);
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 2),
            Ok((true, Vec::new())));
    }

    #[test]
    fn ordered_symbolic_census_never_indexes_without_authored_seeds() {
        let roots = close_ordered_symbolic_roots(HashMap::new(), || panic!("indexed graph without a seed")).unwrap();
        assert!(roots.is_empty());

        let roots = close_ordered_symbolic_roots(HashMap::from([(1, vec![7])]), || {
            Ok(HashMap::from([((1, 7), vec![(0, 2)]), ((0, 2), vec![(0, 3)])]))
        }).unwrap();
        assert_eq!(roots, HashMap::from([(0, HashSet::from([2, 3])), (1, HashSet::from([7]))]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_discovery_follows_cross_only_namespace_wrappers() {
        let app = source("src/main.w", "collect-unselected-namespace-wrapper");
        let child = source("src/child.w", "collect-obligated-namespace-leaf");
        let prepare = |body: &str, startup: &str| {
            let root_text = format!(r#"%%start
                lib = namespace demo "src/child.w";
                alias = namespace demo "src/child.w";
                unit(u8) wrapper = fn(value) {{ if (value == 0) {{}}; {body} }};
                {startup}
            %%end"#);
            let child_text = format!("%%start\nunit(u8) leaf = fn(value) {{ core.require(value < 2, {OUTPUT}); }};\n%%end");
            let namespace = |name: &str| {
                let start = root_text.find(&format!("{name} =")).unwrap();
                let end = start + root_text[start..].find(';').unwrap() + 1;
                ScalarNamespaceBinding {
                    binding: name.into(), target: child.clone(), span: ByteSpan::new(start as u32, end as u32),
                }
            };
            prepare_checker_project(
                vec![
                    (declarations(&app, &root_text), vec![namespace("lib"), namespace("alias")]),
                    (declarations(&child, &child_text), Vec::new()),
                ],
                vec![app.clone(), child.clone()],
                inputs(),
            ).unwrap()
        };
        let read = |prepared: &PreparedCheckerProject| {
            let (active, reports) = collect_ordered_package_failures(prepared, ScalarCheckerScope::CurrentPackage, 2)
                .expect("complete selected namespace roots");
            assert!(active);
            reports.into_iter().map(|report| {
                let ProjectCheck::Failed { source, axes, state, .. } = report else { panic!("expected authored report"); };
                assert_eq!(source, child);
                assert_eq!(axes[0].0.source, source);
                let mut values = state.intervals_at(0, 0).unwrap();
                let (lo, hi) = values.next().unwrap();
                assert!(values.next().is_none());
                (lo.clone(), hi.clone())
            }).collect::<Vec<_>>()
        };
        let domains = |pairs: &[(i32, i32)]| pairs.iter().map(|(lo, hi)| (BigInt::from(*lo), BigInt::from(*hi))).collect::<Vec<_>>();
        let aliases = "lib.leaf(3); alias.leaf(4);";
        let prepared = prepare(aliases, "");
        assert_eq!(read(&prepared), domains(&[(3, 3), (4, 4), (2, 255)]));
        let prepared = prepare(aliases, "lib.leaf(1);");
        assert_eq!(read(&prepared), domains(&[(3, 3), (4, 4)]));
        assert_eq!(check_package_roots(&prepared), Ok(()));
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 2),
            Ok((true, Vec::new())));

        let prepared = prepare("alias.leaf(value);", "lib.leaf(1);");
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 2), Err(ProjectCheck::Unsupported));
        let prepared = prepare("if (false) { alias.leaf(value); };", "lib.leaf(1);");
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 2),
            Ok((true, Vec::new())));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_directive_disjunction_preserves_exact_holes_and_narrowing() {
        use super::super::checker_flow::DIRECTIVE_DISJUNCTION_NORMALIZATIONS;
        let normalizations = || DIRECTIVE_DISJUNCTION_NORMALIZATIONS.with(|count| count.replace(0));
        let _ = normalizations();
        let app = source("src/main.w", "directive-disjunction");
        let prepare = |guard: &str, actual: &str| {
            let text = format!(r#"%%start
                unit(u8, u8) checked = fn(x, y) {{
                    if (y == 1) {{}};
                    if (false) {{ core.require(x == 1 || x == 7, {OUTPUT}); }};
                    core.require({guard}, {OUTPUT});
                    core.require(x != 4, {OUTPUT});
                    core.narrow(x == 1 || x == 7, {OUTPUT});
                    core.require(x == 1, {OUTPUT});
                }};
                {actual}
            %%end"#);
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, prepared)
        };
        let report = |text: &str, prepared: &PreparedCheckerProject, scope| {
            let (active, failures) = collect_ordered_package_failures(prepared, scope, 1).unwrap();
            assert!(active);
            failures.into_iter().map(|failure| {
                let ProjectCheck::Failed { condition, state, .. } = failure else { panic!("expected failure"); };
                let intervals = state.intervals_at(0, 0).unwrap()
                    .map(|(lo, hi)| (lo.clone(), hi.clone())).collect::<Vec<_>>();
                (text[condition.start as usize..condition.end as usize].to_owned(), intervals)
            }).collect::<Vec<_>>()
        };
        let expected = |condition: &str, intervals: &[(u8, u8)]| (condition.to_owned(),
            intervals.iter().map(|&(lo, hi)| (BigInt::from(lo), BigInt::from(hi))).collect::<Vec<_>>());
        let guard = "x < 2 || x > 6";
        let (text, project) = prepare(guard, "");
        assert_eq!(normalizations(), 0);
        assert_eq!(report(&text, &project, ScalarCheckerScope::CurrentPackage), [
            expected(guard, &[(2, 6)]),
            expected("x != 4", &[(4, 4)]),
            expected("x == 1", &[(7, 7)]),
        ]);
        assert_eq!(normalizations(), 2);
        let (text, project) = prepare(guard, "checked(4, 1);");
        assert_eq!(normalizations(), 0);
        assert_eq!(check_package_roots(&project), Err(ProjectCheck::Unsupported));
        assert_eq!(normalizations(), 0);
        assert_eq!(report(&text, &project, ScalarCheckerScope::MainFlowOnly), [
            expected(guard, &[(4, 4)]),
            expected("x != 4", &[(4, 4)]),
            expected("x == 1 || x == 7", &[(4, 4)]),
        ]);
        assert_eq!(normalizations(), 2);

        let (_, project) = prepare("x < 2 || y > 6", "checked(4, 1);");
        assert_eq!(collect_ordered_package_failures(&project, ScalarCheckerScope::MainFlowOnly, 1),
            Err(ProjectCheck::Unsupported));
        assert_eq!(normalizations(), 0);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_root_disjunctive_branch_keeps_exact_holes_and_dead_lanes() {
        let app = source("src/main.w", "root-branch-disjunction");
        let prepare = |condition: &str, actual: &str| {
            let text = format!(r#"%%start
                unit(u8, u8) checked = fn(x, y) {{
                    if (y == 1) {{}};
                    core.narrow(x != 4, {OUTPUT});
                    if ({condition}) {{
                        core.require(x != 7, {OUTPUT});
                        core.narrow(false, {OUTPUT});
                        core.require(false, {OUTPUT});
                    }} else {{
                        core.require(x != 3, {OUTPUT});
                    }};
                    core.require(x >= 2, {OUTPUT});
                    core.require(x <= 6, {OUTPUT});
                    core.require(x != 4, {OUTPUT});
                    core.require(x != 5, {OUTPUT});
                }};
                {actual}
            %%end"#);
            let project = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, project)
        };
        let guard = "x < 2 || x > 6";
        let (text, prepared) = prepare(guard, "");
        let (active, failures) = collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        assert!(active);
        let records = failures.into_iter().map(|failure| {
            let ProjectCheck::Failed { condition, state, .. } = failure else { panic!("expected callable failure"); };
            let intervals = |axis| state.intervals_at(0, axis).unwrap()
                .map(|(first, last)| (first.clone(), last.clone())).collect::<Vec<_>>();
            assert_eq!(intervals(1), [(BigInt::from(0), BigInt::from(255))]);
            (text[condition.start as usize..condition.end as usize].to_owned(), intervals(0))
        }).collect::<Vec<_>>();
        let expected = |condition: &str, ranges: &[(u8, u8)]| (condition.to_owned(), ranges.iter()
            .map(|&(first, last)| (BigInt::from(first), BigInt::from(last))).collect::<Vec<_>>());
        assert_eq!(records, [
            expected("x != 7", &[(7, 7)]),
            expected("false", &[(0, 1), (7, 255)]),
            expected("x != 3", &[(3, 3)]),
            expected("x != 5", &[(5, 5)]),
        ]);
        assert_eq!(check_package_roots(&prepared), Err(ProjectCheck::Unsupported));
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 1), Ok((false, Vec::new())));
        let (_, actual) = prepare(guard, "checked(7, 1);");
        assert_eq!(collect_ordered_package_failures(&actual, ScalarCheckerScope::MainFlowOnly, 1), Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_cross_axis_disjunction_preserves_both_true_causes_after_join() {
        let app = source("src/main.w", "root-branch-cross-axis-disjunction");
        let body = format!(r#"%%start
            unit(u8, i8) checked = fn(x, y) {{
                if (x < 2 || y >= -2) {{
                    core.require(false, {OUTPUT});
                }} else {{
                    core.require(x >= 2, {OUTPUT});
                    core.require(y < -2, {OUTPUT});
                    core.narrow(false, {OUTPUT});
                    core.require(false, {OUTPUT});
                }};
                core.require(x != 2, {OUTPUT});
            }};
            %%end"#);
        let prepare = |text: &str| prepare_checker_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let project = prepare(&body);
        let (active, failures) = collect_ordered_package_failures(&project, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        assert!(active);
        let records = failures.iter().map(|failure| {
            let ProjectCheck::Failed { condition, state, operation, .. } = failure else { panic!("expected callable failure"); };
            (&body[condition.start as usize..condition.end as usize], *operation, state)
        }).collect::<Vec<_>>();
        let [("false", CheckerOperation::Require, entered), ("false", CheckerOperation::Narrow, removed),
             ("x != 2", CheckerOperation::Require, after)] = records.as_slice()
        else { panic!("expected separate true, false and continued paths"); };
        assert!(entered.includes_signed(&[0, -128]));
        assert!(entered.includes_signed(&[2, -2]));
        assert!(!entered.includes_signed(&[2, -3]));
        assert!(removed.includes_signed(&[2, -3]));
        assert!(!removed.includes_signed(&[2, -2]));
        assert!(!removed.includes_signed(&[1, -3]));
        assert!(after.includes_signed(&[2, -2]));
        assert!(!after.includes_signed(&[2, -3]));
        assert!(!after.includes_signed(&[1, -2]));
        assert_eq!(collect_ordered_package_failures(&project, ScalarCheckerScope::MainFlowOnly, 1), Ok((false, Vec::new())));
        let outside = body.replace("y >= -2)", "y >= y)");
        assert_ne!(outside, body);
        assert_eq!(collect_ordered_package_failures(&prepare(&outside), ScalarCheckerScope::CurrentPackage, 1),
            Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_base_conjunctive_branch_preserves_both_false_causes_after_join() {
        let app = source("src/main.w", "root-branch-conjunction");
        let body = format!(r#"%%start
            unit(u8, i8) checked = fn(x, y) {{
                if (x < 2 && y >= -2) {{
                    core.require(x < 2, {OUTPUT});
                    core.require(y >= -2, {OUTPUT});
                    core.require(x != 1, {OUTPUT});
                    core.narrow(false, {OUTPUT});
                    core.require(false, {OUTPUT});
                }} else {{
                    core.require(false, {OUTPUT});
                }};
                core.require(x != 0, {OUTPUT});
            }};
            %%end"#);
        let prepare = |text: &str| prepare_checker_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let project = prepare(&body);
        let (active, failures) = collect_ordered_package_failures(&project, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        assert!(active);
        let failures = failures.iter().map(|failure| {
            let ProjectCheck::Failed { condition, state, operation, .. } = failure else { panic!("expected callable failure"); };
            (&body[condition.start as usize..condition.end as usize], *operation, state)
        }).collect::<Vec<_>>();
        let [(first, CheckerOperation::Require, then), (second, CheckerOperation::Narrow, removed),
             (third, CheckerOperation::Require, otherwise), (fourth, CheckerOperation::Require, after)] = failures.as_slice()
        else { panic!("expected true witness, removed lane, false witness, and continuation"); };
        assert_eq!((*first, *second, *third, *fourth), ("x != 1", "false", "false", "x != 0"));
        assert!(then.includes_signed(&[1, -2]));
        assert!(!then.includes_signed(&[1, -3]));
        assert!(!then.includes_signed(&[2, -2]));
        assert!(removed.includes_signed(&[0, 127]));
        assert!(!removed.includes_signed(&[0, -3]));
        assert!(otherwise.includes_signed(&[255, 127]));
        assert!(otherwise.includes_signed(&[2, -2]));
        assert!(otherwise.includes_signed(&[0, -3]));
        assert!(!otherwise.includes_signed(&[0, -2]));
        assert!(after.includes_signed(&[0, -3]));
        assert!(!after.includes_signed(&[0, -2]));
        assert!(!after.includes_signed(&[2, -3]));
        assert_eq!(collect_ordered_package_failures(&project, ScalarCheckerScope::MainFlowOnly, 1), Ok((false, Vec::new())));
        assert_eq!(check_package_roots(&project), Err(ProjectCheck::Unsupported));

        let outside = body.replace("y >= -2) {", "y >= y) {");
        assert_ne!(outside, body);
        assert_eq!(collect_ordered_package_failures(&prepare(&outside), ScalarCheckerScope::CurrentPackage, 1),
            Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_reachable_local_proof_catalogs_report_exact_cross_file_sources() {
        let app = source("src/main.w", "ordered-local-app");
        let lib = source("src/lib.w", "ordered-local-lib");
        let app_text = r#"%%start
            lib = namespace demo "src/lib.w";
            i8(i8) main_echo = fn(value) { value };
            proof shared for main_echo(entry) -> (out) ensures (out < 5) {}
            i8 first = main_echo(7);
            i8 second = lib.echo(8);
        %%end"#;
        let lib_text = r#"%%start
            i8(i8) echo = fn(value) { value };
            i8(i8) unused = fn(value) { value };
            proof shared for echo(entry) -> (out) ensures (out < 5) {}
            proof uncovered for unused(entry) -> (out) ensures (out != 7) {}
        %%end"#;
        let run = |lib: &SourceIdentity, lib_text: &str, scope| {
            let (_, root_catalog) = crate::parse_proof_source(app.clone(), app_text.into(), &[]).unwrap_or_else(|_| panic!("valid root proof source"));
            let (_, lib_catalog) = crate::parse_proof_source(lib.clone(), lib_text.into(), &[]).unwrap_or_else(|_| panic!("valid local proof source"));
            let binding = ScalarNamespaceBinding {
                binding: "lib".into(), target: lib.clone(), span: ByteSpan::new(0, 0),
            };
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(lib_catalog, Vec::new()), (root_catalog, vec![binding])],
                vec![app.clone(), lib.clone()], inputs(), scope, Vec::new(),
            )
        };
        let report = run(&lib, lib_text, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(report.active && report.directives.is_empty());
        let [first, second] = report.diagnostics.as_slice() else { panic!("both actual proof failures"); };
        assert_eq!(first.proof.source, app);
        assert_eq!(second.proof.source, lib);
        assert_eq!(second.invocation.call.source, app);
        let range = second.invocation.call.range;
        assert_eq!(&app_text[range.start as usize..range.end as usize], "lib.echo(8)");
        let main_flow = run(&lib, lib_text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(main_flow.active && main_flow.directives.is_empty());
        assert_eq!(main_flow.diagnostics.len(), report.diagnostics.len());
        for (main, package) in main_flow.diagnostics.iter().zip(&report.diagnostics) {
            assert_eq!(main.proof, package.proof);
            assert_eq!(main.goal, package.goal);
            assert_eq!(main.invocation.call, package.invocation.call);
            assert_eq!(main.invocation.arguments, package.invocation.arguments);
            assert_eq!(main.invocation.returned, package.invocation.returned);
        }
        assert!(main_flow.symbolic_diagnostic.is_none());
        assert!(main_flow.additional_symbolic_diagnostics.is_empty());
        let symbolic = report.symbolic_diagnostic.unwrap();
        assert_eq!(symbolic.callable.source, lib);
        assert_eq!(symbolic.proof.source, lib);
        assert!(report.additional_symbolic_diagnostics.is_empty());
        assert!(matches!(run(&lib, &lib_text.replace("proof uncovered", "proof shared"), ScalarCheckerScope::CurrentPackage),
            Err(ScalarOrderedCheckerOutcome::Unsupported)));
        let foreign = SourceIdentity { project: "foreign-project".into(), ..lib.clone() };
        assert!(matches!(run(&foreign, lib_text, ScalarCheckerScope::CurrentPackage), Err(ScalarOrderedCheckerOutcome::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_dormant_local_proofs_are_symbolic_without_executing_initialization() {
        let app = source("src/main.w", "ordered-dormant-app");
        let local = source("src/dormant.w", "ordered-dormant-local");
        let app_text = "%%start i8 value = 1; %%end";
        let local_text = format!(r#"%%start
            core.require(false, {OUTPUT});
            i8(i8) unused = fn(value) {{ value }};
            proof uncovered for unused(entry) -> (out) ensures (out != 7) {{}}
        %%end"#);
        let run = |scope, identities: &[&SourceIdentity]| {
            let catalog = |source: &SourceIdentity| crate::parse_proof_source(
                source.clone(), local_text.clone(), &[],
            ).unwrap_or_else(|_| panic!("valid dormant proof source")).1;
            check_loaded_scalar_ordered_local_proof_contexts(
                vec![(declarations(&app, app_text), Vec::new())], [],
                vec![app.clone()], inputs(), scope,
                identities.iter().map(|source| catalog(source)).collect(),
            )
        };
        let package = run(ScalarCheckerScope::CurrentPackage, &[&local]).unwrap();
        assert!(package.active && package.directives.is_empty() && package.diagnostics.is_empty());
        let failure = package.symbolic_diagnostic.unwrap();
        assert_eq!(failure.proof.source, local);
        assert_eq!(failure.callable.source, local);
        assert_eq!(failure.returned, BigInt::from(7));
        assert!(package.additional_symbolic_diagnostics.is_empty());

        let main = run(ScalarCheckerScope::MainFlowOnly, &[&local]).unwrap();
        assert!(!main.active && main.directives.is_empty() && main.diagnostics.is_empty());
        assert!(main.symbolic_diagnostic.is_none() && main.additional_symbolic_diagnostics.is_empty());

        let foreign = SourceIdentity { package: "foreign".into(), ..local.clone() };
        let alias = source("src/main.w", "different-revision");
        for disallowed in [&[&local, &local][..], &[&foreign], &[&alias]] {
            assert!(matches!(run(ScalarCheckerScope::CurrentPackage, disallowed),
                Err(ScalarOrderedCheckerOutcome::Unsupported)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_auto_proof_diagnostics_use_exact_call_sites_without_authored_effects() {
        let app = source("src/main.w", "ordered-automatic-proof");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_main_flow_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(),
            )
        };
        let authored = format!(r#"%%start
            i8(i8) identity = fn(value) {{ value }};
            proof bound for identity(entry) -> (out) where (entry >= 6) ensures (out <= 7) {{}}
            core.require(false, {OUTPUT});
            i8 excluded = identity(4);
            i8 proved = identity(6);
            i8 first = identity(8);
            i8 second = identity(8);
            core.require(false, {OUTPUT});
        %%end"#);
        let receipt = run(&authored).unwrap();
        assert!(receipt.active);
        assert_eq!(receipt.directives.len(), 2);
        assert_eq!(receipt.diagnostics.len(), 2);
        let snippet = |span: ByteSpan| &authored[span.start as usize..span.end as usize];
        let [first, second] = receipt.diagnostics.as_slice() else { unreachable!(); };
        for diagnostic in [first, second] {
            assert_eq!(diagnostic.proof.source, app);
            assert_eq!(diagnostic.goal.source, app);
            assert_eq!(diagnostic.invocation.call.source, app);
            assert!(snippet(diagnostic.proof.range).starts_with("proof bound"));
            assert_eq!(snippet(diagnostic.goal.range), "out <= 7");
            assert_eq!(snippet(diagnostic.invocation.call.range), "identity(8)");
            assert_eq!(diagnostic.invocation.directive_position, 1);
            assert_eq!(diagnostic.invocation.arguments, [(ScalarType::I8, BigInt::from(8))]);
            assert_eq!(diagnostic.invocation.returned, BigInt::from(8));
        }
        assert!(receipt.directives[0].directive.start < first.invocation.call.range.start);
        assert!(first.invocation.call.range.start < second.invocation.call.range.start);
        assert!(second.invocation.call.range.start < receipt.directives[1].directive.start);
        let ordinary = prepare_checker_project(
            vec![(declarations(&app, &authored), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let (active, effects) = collect_ordered_checker_records(&ordinary, ScalarCheckerScope::MainFlowOnly, 1).unwrap();
        assert!(active);
        assert_eq!(effects, receipt.directives);
        let proof_only = |call: &str| format!(r#"%%start
            i8(i8) identity = fn(value) {{ value }};
            proof bound for identity(entry) -> (out) where (entry >= 6) ensures (out <= 7) {{}}
            i8 result = identity({call});
        %%end"#);
        let refuted = run(&proof_only("8")).unwrap();
        assert!(refuted.active && refuted.directives.is_empty() && refuted.diagnostics.len() == 1);
        let excluded = run(&proof_only("4")).unwrap();
        assert!(!excluded.active && excluded.directives.is_empty() && excluded.diagnostics.is_empty());
        let incomplete = proof_only("8").replace("ensures (out <= 7) {}", "ensures (out <= 7) { core.show(out <= 7); core.show(true); }");
        assert!(matches!(run(&incomplete), Err(ProjectCheck::Unsupported)));
        let mixed = proof_only("8").replace("%%end", &format!(r#"
            unit(i8) inner = fn(arg) {{
                i8 inside = identity(arg);
                core.require(inside == 8, {OUTPUT});
            }};
            unit(i8) wrapper = fn(arg) {{ inner(arg); }};
            wrapper(8);
            %%end"#));
        assert!(matches!(run(&mixed), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_nested_scalar_preserves_exact_startup_instances_and_directives() {
        let app = source("src/main.w", "ordered-automatic-proof-actual-nested-scalar");
        let text = format!(r#"%%start
            i8(i8) identity = fn(value) {{ value }};
            unit(i8) wrapper = fn(arg) {{
                i8 result = identity(arg);
                core.require(result == 6, {OUTPUT});
            }};
            proof bound for identity(entry) -> (out) ensures (out <= 5) {{}}
            wrapper(6);
            core.require(false, {OUTPUT});
            wrapper(9);
        %%end"#);
        let (_, proof) = crate::parse_proof_source(app.clone(), text.clone(), &[])
            .unwrap_or_else(|_| panic!("valid proof source"));
        let report = check_loaded_ordered_proof_diagnostics(
            Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::MainFlowOnly,
        ).unwrap();
        let declaration = "proof bound for identity(entry) -> (out) ensures (out <= 5) {}";
        let without = text.replace(declaration, &" ".repeat(declaration.len()));
        let ordinary = check_loaded_ordered_proof_diagnostics(
            vec![(declarations(&app, &without), Vec::new())], Vec::new(),
            vec![app.clone()], inputs(), ScalarCheckerScope::MainFlowOnly,
        ).unwrap();
        let legacy = check_scalar_package_ordered_with_dormant_sources(
            vec![(declarations(&app, &without), Vec::new())], vec![app.clone()], Vec::new(),
            inputs(), ScalarCheckerScope::MainFlowOnly,
        );
        assert!(matches!(legacy, ScalarOrderedCheckerOutcome::Unsupported));
        assert!(report.active && ordinary.active && ordinary.diagnostics.is_empty());
        assert_eq!(report.directives, ordinary.directives);
        let [startup, requirement] = report.directives.as_slice() else { panic!("startup and exact downstream failures"); };
        assert!(requirement.directive.start < startup.directive.start);
        let nine = BigInt::from(9);
        assert_eq!(requirement.witness.intervals_at(0, 1).unwrap().collect::<Vec<_>>(), [(&nine, &nine)]);
        let [first, second] = report.diagnostics.as_slice() else { panic!("two authentic actual calls"); };
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(first.invocation.call, second.invocation.call);
        assert_eq!(snippet(first.invocation.call.range), "identity(arg)");
        assert_eq!(snippet(first.invocation.outer_call.as_ref().unwrap().range), "wrapper(6)");
        assert_eq!(snippet(second.invocation.outer_call.as_ref().unwrap().range), "wrapper(9)");
        assert_eq!(first.invocation.arguments, [(ScalarType::I8, BigInt::from(6))]);
        assert_eq!(second.invocation.arguments, [(ScalarType::I8, nine)]);
        assert_eq!(first.invocation.directive_position, 0);
        assert_eq!(second.invocation.directive_position, 1);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_nested_scalar_chains_exact_root_lexical_values() {
        let app = source("src/main.w", "ordered-automatic-proof-actual-nested-lexical");
        let text = format!(r#"%%start
            i8(i8) transform = fn(value) {{ if (value == 6) {{ 7 }} else {{ 10 }} }};
            i8(i8) identity = fn(value) {{ value }};
            unit(i8) wrapper = fn(arg) {{
                i8 first = transform(arg);
                core.require(first == 8, {OUTPUT});
                i8 second = identity(first);
                core.require(second == 8, {OUTPUT});
            }};
            proof before for transform(entry) -> (out) ensures (out <= 5) {{}}
            proof after for identity(entry) -> (out) ensures (out <= 5) {{}}
            wrapper(6);
            wrapper(9);
        %%end"#);
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid canonical proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::MainFlowOnly,
            )
        };
        let report = run(&text).unwrap();
        assert!(report.active);
        assert_eq!(report.directives.len(), 4);
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        for (index, (diagnostic, (outer, inner, argument, returned))) in report.diagnostics.iter().zip([
            ("wrapper(6)", "transform(arg)", 6, 7), ("wrapper(6)", "identity(first)", 7, 7),
            ("wrapper(9)", "transform(arg)", 9, 10), ("wrapper(9)", "identity(first)", 10, 10),
        ]).enumerate() {
            assert_eq!(snippet(diagnostic.invocation.outer_call.as_ref().unwrap().range), outer);
            assert_eq!(snippet(diagnostic.invocation.call.range), inner);
            assert_eq!(diagnostic.invocation.arguments, [(ScalarType::I8, BigInt::from(argument))]);
            assert_eq!(diagnostic.invocation.returned, BigInt::from(returned));
            assert_eq!(diagnostic.invocation.directive_position, index);
            let witness = &report.directives[index].witness;
            let axis = 1 + index % 2;
            let returned = BigInt::from(returned);
            assert_eq!(witness.intervals_at(0, axis).unwrap().collect::<Vec<_>>(), [(&returned, &returned)]);
        }
        assert_eq!(report.diagnostics.len(), 4);
        let without = text.lines().map(|line| {
            if line.trim_start().starts_with("proof ") { " ".repeat(line.len()) } else { line.to_owned() }
        }).collect::<Vec<_>>().join("\n");
        let ordinary = check_loaded_ordered_proof_diagnostics(
            vec![(declarations(&app, &without), Vec::new())], Vec::new(), vec![app.clone()], inputs(),
            ScalarCheckerScope::MainFlowOnly,
        ).unwrap();
        assert!(ordinary.diagnostics.is_empty());
        assert_eq!(report.directives, ordinary.directives);
        assert!(matches!(check_scalar_package_ordered_with_dormant_sources(
            vec![(declarations(&app, &without), Vec::new())], vec![app.clone()], Vec::new(), inputs(),
            ScalarCheckerScope::MainFlowOnly,
        ), ScalarOrderedCheckerOutcome::Unsupported));
        let deeper = text.replace("unit(i8) wrapper = fn(arg)", "unit(i8) inner = fn(arg)")
            .replace("            wrapper(6);", "            unit(i8) wrapper = fn(arg) { inner(arg); };\n            wrapper(6);");
        assert!(matches!(run(&deeper), Err(ProjectCheck::Unsupported)));
        let late = text.replace("i8 second = identity(first);", "i8 second = identity(7);");
        assert!(matches!(run(&late), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_auto_proof_distinguishes_failed_literal_show_from_ensures() {
        let app = source("src/main.w", "ordered-automatic-proof-actual-show");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = format!(r#"%%start
            i8(i8) identity = fn(value) {{ value }};
            proof before for identity(entry) -> (out) where (entry == 6) ensures (out < 6) {{}}
            proof both for identity(entry) -> (out) where (entry >= 5) ensures (out <= 8) {{
                core.show(out >= 7);
            }}
            proof duplicate for identity(entry) -> (out) where (entry == 6) ensures (out < 6) {{
                core.show(out < 6);
            }}
            proof after for identity(entry) -> (out) where (entry == 6) ensures (out != 6) {{}}
            proof shared_bound for identity(entry) -> (out) where (entry == 6) ensures (out > 7) {{
                core.show(out < 7);
            }}
            i8 excluded = identity(4);
            core.require(false, {OUTPUT});
            i8 bad_step = identity(6);
            i8 passes = identity(7);
            i8 bad_goal = identity(9);
            core.require(false, {OUTPUT});
        %%end"#);
        let report = run(&text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(report.active && report.directives.len() == 2);
        let [before, step, duplicate, after, shared_bound, goal] = report.diagnostics.as_slice() else {
            panic!("six actual diagnostics");
        };
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert!(snippet(before.proof.range).starts_with("proof before") && before.failed_step.is_none());
        assert!(snippet(after.proof.range).starts_with("proof after") && after.failed_step.is_none());
        assert!(snippet(shared_bound.proof.range).starts_with("proof shared_bound") && shared_bound.failed_step.is_none());
        assert!(Rc::ptr_eq(&before.invocation, &after.invocation));
        assert_eq!(snippet(step.failed_step.as_ref().unwrap().range), "core.show(out >= 7)");
        assert_eq!(snippet(step.goal.range), "out <= 8");
        assert_eq!(snippet(duplicate.failed_step.as_ref().unwrap().range), "core.show(out < 6)");
        assert_eq!(snippet(duplicate.goal.range), "out < 6");
        assert!(Rc::ptr_eq(&step.invocation, &duplicate.invocation));
        assert!(goal.failed_step.is_none());
        assert_eq!(snippet(goal.goal.range), "out <= 8");
        assert_eq!(snippet(step.invocation.call.range), "identity(6)");
        assert_eq!(snippet(goal.invocation.call.range), "identity(9)");
        assert_eq!(step.invocation.directive_position, 1);
        assert_eq!(goal.invocation.directive_position, 1);

        let unused = r#"%%start
            i8(i8) identity = fn(value) { value };
            proof both for identity(entry) -> (out) ensures (out <= 8) { core.show(out >= 7); }
        %%end"#;
        assert!(!run(unused, ScalarCheckerScope::MainFlowOnly).unwrap().active);
        assert!(run(unused, ScalarCheckerScope::CurrentPackage).unwrap().symbolic_diagnostic.is_some());
        let invalid = unused.replace("ensures", "where (false) ensures").replace("out >= 7", "out >= 128");
        assert!(matches!(run(&invalid, ScalarCheckerScope::MainFlowOnly), Err(ProjectCheck::InvalidProof)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_bare_old_goal_uses_distinct_actual_snapshots() {
        let app = source("src/main.w", "ordered-automatic-proof-actual-old-goal");
        let program = r#"%%start
            i16(i16, i16) choose = fn(left, right) { if (right < 0) { left } else { right } };
            proof identity for choose(entry, selector) -> (out)
                where (entry >= -5) ensures (out == core.old(entry)) {}
        %%end"#;
        let actual = program.replace("%%end", &format!(r#"
            i16 passes = choose(2, -3);
            i16 first = choose(4, 8);
            core.require(false, {OUTPUT});
            i16 second = choose(4, 8);
            i16 excluded = choose(-6, 8);
        %%end"#));
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid actual entry-goal proof"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let receipt = run(&actual, scope).unwrap();
            assert!(receipt.active && receipt.directives.len() == 1 && receipt.symbolic_diagnostic.is_none());
            let [first, second] = receipt.diagnostics.as_slice() else { panic!("distinct repeated call failures"); };
            let snippet = |span: ByteSpan| &actual[span.start as usize..span.end as usize];
            assert_eq!(snippet(first.goal.range), "out == core.old(entry)");
            assert_eq!(first.goal, second.goal);
            assert!(first.failed_step.is_none() && second.failed_step.is_none());
            assert_eq!(snippet(first.invocation.call.range), "choose(4, 8)");
            assert_eq!(snippet(second.invocation.call.range), "choose(4, 8)");
            assert_ne!(first.invocation.call.range, second.invocation.call.range);
            assert!(!Rc::ptr_eq(&first.invocation, &second.invocation));
            assert_eq!(first.invocation.arguments, [(ScalarType::I16, BigInt::from(4)), (ScalarType::I16, BigInt::from(8))]);
            assert_eq!(first.invocation.arguments, second.invocation.arguments);
            assert_eq!(first.invocation.returned, BigInt::from(8));
            assert_eq!(second.invocation.returned, BigInt::from(8));
            assert_eq!((first.invocation.directive_position, second.invocation.directive_position), (0, 1));
        }
        assert!(!run(program, ScalarCheckerScope::MainFlowOnly).unwrap().active);
        let symbolic = run(program, ScalarCheckerScope::CurrentPackage).unwrap();
        let failed = symbolic.symbolic_diagnostic.expect("unused choose has a signed counterexample");
        assert_eq!(failed.returned, BigInt::from(0));
        let original = BigInt::from(i16::MAX);
        let returned = BigInt::from(0);
        assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&original, &original)]));
        assert!(failed.witness.intervals_at(0, 1).unwrap().eq([(&returned, &returned)]));
        let unknown = actual.replace("core.old(entry)", "core.old(unknown)");
        assert!(matches!(run(&unknown, ScalarCheckerScope::MainFlowOnly), Err(ProjectCheck::InvalidProof)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_bare_old_goal_keeps_unguarded_directives_and_exact_witness() {
        let app = source("src/main.w", "ordered-symbolic-old-goal");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid symbolic entry-goal proof"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            ).unwrap()
        };
        let program = format!(r#"%%start
            i8(i8, i8) choose = fn(value, gate) {{
                core.require(gate != -4, {OUTPUT});
                if (gate != -3) {{ value }} else {{ -2 }}
            }};
            proof entry_value for choose(entry, selector) -> (out)
                where (selector == -3) ensures (out == core.old(entry)) {{}}
        %%end"#);
        assert!(!run(&program, ScalarCheckerScope::MainFlowOnly).active);
        let receipt = run(&program, ScalarCheckerScope::CurrentPackage);
        let diagnostic = receipt.symbolic_diagnostic.expect("constant differs from retained entry");
        assert_eq!(diagnostic.returned, BigInt::from(-2));
        assert_eq!(diagnostic.directive_position, 1);
        let snippet = |span: ByteSpan| &program[span.start as usize..span.end as usize];
        assert_eq!(snippet(diagnostic.goal.range), "out == core.old(entry)");
        assert_eq!(snippet(diagnostic.output.range), "-2");
        let minimum = BigInt::from(i8::MIN);
        let gate = BigInt::from(-3);
        let outside_guard = BigInt::from(-4);
        assert!(diagnostic.witness.intervals_at(0, 0).unwrap().eq([(&minimum, &minimum)]));
        assert!(diagnostic.witness.intervals_at(0, 1).unwrap().eq([(&gate, &gate)]));
        let [directive] = receipt.directives.as_slice() else { panic!("proof guard must not erase authored directive"); };
        assert!(directive.witness.intervals_at(0, 1).unwrap().eq([(&outside_guard, &outside_guard)]));

        let passing = program.replace("else { -2 }", "else { value }");
        let receipt = run(&passing, ScalarCheckerScope::CurrentPackage);
        assert!(receipt.active && receipt.symbolic_diagnostic.is_none());
        assert_eq!(receipt.directives.len(), 1);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_auto_proof_old_show_uses_exact_checked_entry_slot() {
        let app = source("src/main.w", "ordered-automatic-proof-actual-old-show");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = r#"%%start
            i16(i16, i16) choose = fn(tag, original) {
                if (original == 0) {};
                if (tag == 0) { 7 } else { original }
            };
            proof signed for choose(flag, entry) -> (out) where (entry < 0) ensures (out <= 7) {
                core.show(out > core.old(flag));
            }
            proof bound for choose(flag, entry) -> (out) where (entry >= 0) ensures (out < 7) {
                core.show(out <= core.old(entry));
            }
            i16 first = choose(1, -3);
            i16 second = choose(0, 6);
            i16 third = choose(0, 7);
        %%end"#;
        let receipt = run(text, ScalarCheckerScope::MainFlowOnly).unwrap();
        let [first, second, goal] = receipt.diagnostics.as_slice() else { panic!("three actual failures"); };
        assert!(receipt.active && receipt.directives.is_empty() && receipt.symbolic_diagnostic.is_none());
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(snippet(first.failed_step.as_ref().unwrap().range), "core.show(out > core.old(flag))");
        assert_eq!(first.invocation.returned, BigInt::from(-3));
        assert_eq!(first.invocation.arguments, [(ScalarType::I16, BigInt::from(1)), (ScalarType::I16, BigInt::from(-3))]);
        assert_eq!(snippet(second.failed_step.as_ref().unwrap().range), "core.show(out <= core.old(entry))");
        assert_eq!(second.invocation.arguments[1].1, BigInt::from(6));
        assert!(goal.failed_step.is_none());
        assert_eq!(snippet(goal.goal.range), "out < 7");
        assert_eq!(goal.invocation.arguments[1].1, BigInt::from(7));
        let invalid = text.replace("entry < 0", "false").replace("core.old(flag)", "core.old(unknown)");
        assert!(matches!(run(&invalid, ScalarCheckerScope::MainFlowOnly), Err(ProjectCheck::InvalidProof)));
        let wrong_width = text.replace("i16(i16, i16)", "i16(u8, i16)");
        assert!(matches!(run(&wrong_width, ScalarCheckerScope::MainFlowOnly), Err(ProjectCheck::Unsupported)));
        let unused = text.replace("            i16 first = choose(1, -3);\n            i16 second = choose(0, 6);\n            i16 third = choose(0, 7);\n", "");
        assert!(!run(&unused, ScalarCheckerScope::MainFlowOnly).unwrap().active);
        assert!(matches!(run(&unused, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_auto_proof_multi_show_fails_at_first_authored_step() {
        let app = source("src/main.w", "ordered-automatic-proof-actual-multi-show");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = r#"%%start
            i8(i8) identity = fn(value) { value };
            proof bound for identity(entry) -> (out) where (entry > 3) ensures (out != 6) {
                core.show(out < 9);
                core.show(out == 6);
                core.show(out == 6);
            }
            i8 excluded = identity(2);
            i8 both = identity(10);
            i8 later = identity(5);
            i8 goal = identity(6);
        %%end"#;
        let receipt = run(text, ScalarCheckerScope::MainFlowOnly).unwrap();
        let [first, later, goal] = receipt.diagnostics.as_slice() else { panic!("three actual failures"); };
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert!(receipt.active && receipt.directives.is_empty() && receipt.symbolic_diagnostic.is_none());
        assert_eq!(snippet(first.failed_step.as_ref().unwrap().range), "core.show(out < 9)");
        let second_span = later.failed_step.as_ref().unwrap().range;
        assert_eq!(snippet(second_span), "core.show(out == 6)");
        assert_eq!(second_span.start as usize, text.find("core.show(out == 6)").unwrap());
        assert!(goal.failed_step.is_none());
        assert_eq!(snippet(goal.goal.range), "out != 6");
        assert_eq!([&first.invocation.returned, &later.invocation.returned, &goal.invocation.returned],
            [&BigInt::from(10), &BigInt::from(5), &BigInt::from(6)]);
        let invalid = text.replace("entry > 3", "false").replace("out == 6", "out == 128");
        assert!(matches!(run(&invalid, ScalarCheckerScope::MainFlowOnly), Err(ProjectCheck::InvalidProof)));
        let unused = &text[..text.find("            i8 excluded").unwrap()];
        let unused = format!("{unused}%%end");
        assert!(!run(&unused, ScalarCheckerScope::MainFlowOnly).unwrap().active);
        assert!(matches!(run(&unused, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_auto_proofs_follow_call_then_declaration_order() {
        let app = source("src/main.w", "ordered-automatic-proof-multiple");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_main_flow_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(),
            )
        };
        let text = format!(r#"%%start
            i8(i8) primary = fn(value) {{ value }};
            i8(i8) secondary = fn(value) {{ value }};
            i8(i8) unrelated = fn(value) {{ value }};
            proof zeta for primary(entry) -> (out) ensures (out < 8) {{}}
            proof beta for secondary(entry) -> (out) ensures (out < 4) {{}}
            proof alpha for primary(entry) -> (out) ensures (out < 7) {{}}
            proof ignored for primary(entry) -> (out) where (entry < 0) ensures (out == 0) {{}}
            i8 noise = unrelated(9);
            i8 before = secondary(4);
            core.require(false, {OUTPUT});
            i8 first = primary(8);
            i8 second = primary(8);
        %%end"#);
        let receipt = run(&text).unwrap();
        assert!(receipt.active && receipt.directives.len() == 1);
        let [beta, first_zeta, first_alpha, second_zeta, second_alpha] = receipt.diagnostics.as_slice()
            else { panic!("expected five failing proof declarations"); };
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        for (diagnostic, name, call, prior) in [
            (beta, "proof beta", "secondary(4)", 0),
            (first_zeta, "proof zeta", "primary(8)", 1),
            (first_alpha, "proof alpha", "primary(8)", 1),
            (second_zeta, "proof zeta", "primary(8)", 1),
            (second_alpha, "proof alpha", "primary(8)", 1),
        ] {
            assert!(snippet(diagnostic.proof.range).starts_with(name));
            assert_eq!(snippet(diagnostic.invocation.call.range), call);
            assert_eq!(diagnostic.invocation.directive_position, prior);
        }
        assert!(Rc::ptr_eq(&first_zeta.invocation, &first_alpha.invocation));
        assert!(Rc::ptr_eq(&second_zeta.invocation, &second_alpha.invocation));
        assert!(!Rc::ptr_eq(&first_zeta.invocation, &second_zeta.invocation));
        assert!(first_zeta.invocation.call.range.start < second_zeta.invocation.call.range.start);
        assert!(matches!(run(&text.replace("proof alpha", "proof zeta")), Err(ProjectCheck::AmbiguousRoot)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_main_flow_auto_proof_does_not_cover_an_unlocated_nested_call_with_a_direct_call() {
        let app = source("src/main.w", "ordered-automatic-proof-mixed-context");
        let span = ByteSpan::new(1, 2);
        let bound = BigInt::from(7);
        let value = BigInt::from(8);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, "%%start
i8(i8) identity = fn(value) { value }; i8 out = identity(8);
%%end"), Vec::new())],
            vec![app.clone()], inputs(),
        ).unwrap();
        let region = prepared.parts().1.initialization_region(0).unwrap();
        let ModuleInitializationDetail::Occurrences(topology) = &region.detail else { unreachable!(); };
        let occurrence = *topology.call_targets.keys().next().unwrap();
        let mut observer = MainFlowProofObserver::new((0, 0), (MainFlowProofCase {
            proof_source: &app, proof: span, goal: span, filter: &[],
            guard: CheckedIntegerGuardSlots { arity: 1, always_rejects: false, comparisons: Vec::new(), domain: None },
            relation: IntegerRelation::from_operator(&BinaryOperator::LessEqual).unwrap(), bound: MainFlowProofBound::Literal(&bound),
        }, None));
        let effects = OrderedFailures::default();
        observer.observe_initializer(
            (0, 0), Some(InitializerProofSite { source: &app, call: InitializerProofOrigin::Direct { topology, occurrence, directive_position: effects.records.len() } }),
            &[ScalarType::I8], &[&value], &value,
        );
        assert!(observer.first.1.seen && !observer.incomplete && observer.diagnostics.len() == 1);
        observer.observe_initializer((0, 1), None, &[ScalarType::I8], &[&value], &value);
        assert!(!observer.incomplete);
        observer.observe_initializer((0, 0), None, &[ScalarType::I8], &[&value], &value);
        assert!(observer.first.1.seen && observer.incomplete && observer.diagnostics.len() == 1);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proof_shares_the_one_unused_invocation_with_directives() {
        let app = source("src/main.w", "ordered-automatic-proof-unused");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let authored = format!(r#"%%start
            u8(u8) checked = fn(value) {{
                if (value <= 7) {{
                    core.require(value < 7, {OUTPUT});
                    value
                }} else {{
                    core.narrow(false, {OUTPUT});
                    core.require(false, {OUTPUT});
                    value
                }}
            }};
            proof bound for checked(entry) -> (out) where (true) ensures (out < 7) {{}}
        %%end"#);
        let report = run(&authored, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(report.active && report.diagnostics.is_empty());
        let [required, narrowed] = report.directives.as_slice() else { panic!("two authored failure receipts"); };
        assert_eq!(required.operation, ScalarCheckerOperation::Require);
        assert_eq!(narrowed.operation, ScalarCheckerOperation::Narrow);
        let diagnostic = report.symbolic_diagnostic.expect("failed symbolic goal receipt");
        let snippet = |span: ByteSpan| &authored[span.start as usize..span.end as usize];
        assert_eq!(diagnostic.proof.source, app);
        assert_eq!(diagnostic.goal.source, app);
        assert_eq!(diagnostic.callable.source, app);
        assert_eq!(diagnostic.output.source, app);
        assert!(snippet(diagnostic.proof.range).starts_with("proof bound"));
        assert_eq!(snippet(diagnostic.goal.range), "out < 7");
        assert_eq!(snippet(diagnostic.callable.range), "checked");
        assert_eq!(snippet(diagnostic.output.range), "value");
        assert_eq!(diagnostic.directive_position, 2);
        assert_eq!(diagnostic.returned, BigInt::from(7));
        assert_eq!(diagnostic.witness.axes().map(|(_, ty)| ty).collect::<Vec<_>>(), [&ScalarType::U8]);
        assert_eq!(diagnostic.witness.intervals_at(0, 0).unwrap().map(|(a, b)| (a.clone(), b.clone())).collect::<Vec<_>>(),
            [(BigInt::from(7), BigInt::from(7))]);
        let main = run(&authored, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(!main.active && main.directives.is_empty() && main.diagnostics.is_empty() && main.symbolic_diagnostic.is_none());
        let proof_only = r#"%%start
            u8(u8) identity = fn(value) { value };
            proof bound for identity(entry) -> (out) ensures (out <= 1) {}
        %%end"#;
        let selected = run(proof_only, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(selected.active && selected.directives.is_empty() && selected.symbolic_diagnostic.is_some());
        let used = proof_only.replace("%%end", "u8 live = identity(1);
%%end");
        let used = run(&used, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(used.active && used.directives.is_empty() && used.diagnostics.is_empty() && used.symbolic_diagnostic.is_none());
        let entirely_dead = format!(r#"%%start
            u8(u8) identity = fn(value) {{ core.narrow(false, {OUTPUT}); value }};
            proof bound for identity(entry) -> (out) ensures (out <= 1) {{}}
        %%end"#);
        let entirely_dead = run(&entirely_dead, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(entirely_dead.active && entirely_dead.directives.len() == 1 && entirely_dead.symbolic_diagnostic.is_none());
        let unsupported_tail = authored.replace("\n                    value\n                } else",
            "\n                    value + 1\n                } else");
        assert_ne!(unsupported_tail, authored);
        assert!(matches!(run(&unsupported_tail, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
        let restricted = proof_only.replace("ensures", "where (entry >= 1 && entry <= 2) ensures");
        assert!(matches!(run(&restricted, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proof_prioritizes_failed_literal_show_in_unused_return_bundle() {
        let app = source("src/main.w", "ordered-automatic-proof-unused-show");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = format!(r#"%%start
            u8(u8) checked = fn(value) {{
                if (value < 7) {{
                    core.require(false, {OUTPUT});
                    8
                }} else {{
                    core.require(false, {OUTPUT});
                    6
                }}
            }};
            proof bound for checked(entry) -> (out) where (true && true) ensures (out < 8) {{
                core.show(out >= 7);
            }}
        %%end"#);
        let report = run(&text, ScalarCheckerScope::CurrentPackage).unwrap();
        let [first, second] = report.directives.as_slice() else { panic!("two directive receipts"); };
        assert!(first.directive.start < second.directive.start);
        assert!(report.active && report.diagnostics.is_empty() && report.additional_symbolic_diagnostics.is_empty());
        let diagnostic = report.symbolic_diagnostic.unwrap();
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(diagnostic.goal.source, app);
        assert_eq!(snippet(diagnostic.goal.range), "out < 8");
        let step = diagnostic.failed_step.unwrap();
        assert_eq!(step.source, app);
        assert_eq!(snippet(step.range), "core.show(out >= 7)");
        assert_eq!(snippet(diagnostic.output.range), "6");
        assert_eq!(diagnostic.returned, BigInt::from(6));
        assert_eq!(diagnostic.directive_position, 2);
        assert_eq!(diagnostic.witness.intervals_at(0, 0).unwrap().next().unwrap().0, &BigInt::from(7));

        let goal_only = text.replace("out >= 7", "out >= 6");
        let goal = run(&goal_only, ScalarCheckerScope::CurrentPackage).unwrap().symbolic_diagnostic.unwrap();
        assert!(goal.failed_step.is_none());
        assert_eq!(goal.returned, BigInt::from(8));

        let duplicate = text.replace("out >= 7", "out < 8");
        let duplicate = run(&duplicate, ScalarCheckerScope::CurrentPackage).unwrap().symbolic_diagnostic.unwrap();
        assert_eq!(duplicate.failed_step.unwrap().source, app);
        assert_eq!(duplicate.returned, BigInt::from(8));

        let dead = text.replace("core.require", "core.narrow");
        let dead = run(&dead, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(dead.active && dead.directives.len() == 2 && dead.symbolic_diagnostic.is_none());
        let main = run(&text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(!main.active && main.directives.is_empty() && main.symbolic_diagnostic.is_none());
        let guarded = text.replace("true && true", "entry >= 1");
        assert!(matches!(run(&guarded, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
        let mixed = text.replace("%%end", "proof other for checked(entry) -> (out) ensures (out < 9) {}\n%%end");
        assert!(matches!(run(&mixed, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
        let incomplete = text.replace("\n                    6\n", "\n                    value + 1\n");
        assert!(matches!(run(&incomplete, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proof_multi_show_uses_earliest_step_across_constant_returns() {
        let app = source("src/main.w", "ordered-automatic-proof-unused-multi-show");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = format!(r#"%%start
            u8(u8) checked = fn(value) {{
                if (value < 7) {{
                    core.require(false, {OUTPUT});
                    8
                }} else {{
                    core.require(false, {OUTPUT});
                    6
                }}
            }};
            proof bound for checked(entry) -> (out) where (true && true) ensures (out == 7) {{
                core.show(out >= 7);
                core.show(out <= 7);
            }}
        %%end"#);
        let report = run(&text, ScalarCheckerScope::CurrentPackage).unwrap();
        let [first, second] = report.directives.as_slice() else { panic!("two directive receipts"); };
        assert!(first.directive.start < second.directive.start);
        assert!(report.active && report.diagnostics.is_empty() && report.additional_symbolic_diagnostics.is_empty());
        let diagnostic = report.symbolic_diagnostic.unwrap();
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        let step = diagnostic.failed_step.unwrap();
        assert_eq!(step.source, app);
        assert_eq!(snippet(step.range), "core.show(out >= 7)");
        assert_eq!(snippet(diagnostic.goal.range), "out == 7");
        assert_eq!(snippet(diagnostic.output.range), "6");
        assert_eq!(diagnostic.returned, BigInt::from(6));
        assert_eq!(diagnostic.directive_position, 2);
        assert_eq!(diagnostic.witness.intervals_at(0, 0).unwrap().next().unwrap().0, &BigInt::from(7));

        let goal_only = text.replace("out >= 7", "out >= 6").replace("out <= 7", "out <= 8");
        let goal = run(&goal_only, ScalarCheckerScope::CurrentPackage).unwrap().symbolic_diagnostic.unwrap();
        assert!(goal.failed_step.is_none());
        assert_eq!(goal.returned, BigInt::from(8));

        let tie = text.replace("out >= 7", "out < 6");
        let tied = run(&tie, ScalarCheckerScope::CurrentPackage).unwrap().symbolic_diagnostic.unwrap();
        assert!(tied.failed_step.is_some());
        assert_eq!(tied.returned, BigInt::from(8));

        let alias = tie.replace("\n                    6\n", "\n                    value + 1\n");
        assert_ne!(alias, tie);
        assert!(matches!(run(&alias, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
        let dead = alias.replace("core.require", "core.narrow");
        let dead = run(&dead, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(dead.active && dead.directives.len() == 2 && dead.symbolic_diagnostic.is_none());
        let main = run(&text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(!main.active && main.directives.is_empty() && main.symbolic_diagnostic.is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proof_multi_show_preserves_exact_alias_holes_and_axes() {
        let app = source("src/main.w", "ordered-automatic-proof-unused-multi-show-alias");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let text = format!(r#"%%start
            u8(u8, u8) checked = fn(value, other) {{
                core.narrow(other == 11, {OUTPUT});
                core.narrow(value <= 2 || value >= 5, {OUTPUT});
                value
            }};
            proof bound for checked(entry, untouched) -> (out) ensures (out != 3) {{
                core.show(out != 3);
                core.show(out != 6);
                core.show(out != 8);
                core.show(out < 9);
            }}
        %%end"#);
        let receipt = run(&text).unwrap();
        assert!(receipt.active && receipt.directives.is_empty() && receipt.diagnostics.is_empty());
        let diagnostic = receipt.symbolic_diagnostic.unwrap();
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(snippet(diagnostic.failed_step.unwrap().range), "core.show(out != 6)");
        assert_eq!(diagnostic.returned, BigInt::from(6));
        let singleton = |axis, value| {
            let mut intervals = diagnostic.witness.intervals_at(0, axis).unwrap();
            let expected = BigInt::from(value);
            assert_eq!(intervals.next(), Some((&expected, &expected)));
            assert!(intervals.next().is_none());
        };
        singleton(0, 6);
        singleton(1, 11);

        let goal_only = text.replace("core.show(out != 6);", "core.show(out != 4);")
            .replace("core.show(out != 8);", "core.show(out >= 0);")
            .replace("core.show(out < 9);", "core.show(out <= 255);")
            .replace("ensures (out != 3)", "ensures (out != 6)");
        let goal = run(&goal_only).unwrap().symbolic_diagnostic.unwrap();
        assert!(goal.failed_step.is_none());
        assert_eq!(goal.returned, BigInt::from(6));

        let first_endpoint = text.replace("core.show(out != 3);", "core.show(out < 9);");
        let first_endpoint = run(&first_endpoint).unwrap().symbolic_diagnostic.unwrap();
        assert!(first_endpoint.failed_step.is_some());
        assert_eq!(first_endpoint.returned, BigInt::from(255));

        let single_equality = text.replace("core.show(out != 3);", "core.show(out >= 0);")
            .replace("core.show(out != 8);", "core.show(out <= 255);");
        let direct = run(&single_equality).unwrap().symbolic_diagnostic.unwrap();
        assert!(direct.failed_step.is_some());
        assert_eq!(direct.returned, BigInt::from(6));

        let no_equalities = text.replace("core.show(out != 3);", "core.show(out >= 0);")
            .replace("core.show(out != 6);", "core.show(out <= 255);")
            .replace("core.show(out != 8);", "core.show(out < 9);");
        let endpoint = run(&no_equalities).unwrap().symbolic_diagnostic.unwrap();
        assert!(endpoint.failed_step.is_some());
        assert_eq!(endpoint.returned, BigInt::from(255));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_later_requirement_uses_independently_verified_total_equality() {
        let app = source("src/main.w", "ordered-automatic-proof-exact-summary");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let producer = r#"
            i16(i16) producer = fn(input) {
                if (input < 0) { 7 } else { 7 }
            };
        "#;
        let caller = format!(r#"
            unit(i16) client = fn(input) {{
                i16 result = producer(input);
                core.require(result == 7, {OUTPUT});
            }};
        "#);
        let module = |before: &str, after: &str, goal: &str| format!(r#"%%start
            {before}
            {after}
            proof bound for producer(entry) -> (out) ensures ({goal}) {{}}
            core.require(false, {OUTPUT});
        %%end"#);
        let text = module(producer, &caller, "out == 7");
        let report = run(&text).unwrap();
        assert!(report.active && report.directives.len() == 1);
        assert!(report.diagnostics.is_empty() && report.symbolic_diagnostic.is_none());

        let false_requirement = caller.replace("result == 7", "result < 7");
        let result = run(&module(producer, &false_requirement, "out == 7")).unwrap();
        let [startup, failure] = result.directives.as_slice() else { panic!("startup and caller failures"); };
        assert!(failure.directive.start < startup.directive.start);
        let witnesses = failure.witness.intervals_at(0, 1).unwrap().collect::<Vec<_>>();
        let bound = BigInt::from(7);
        assert_eq!(witnesses, [(&bound, &bound)]);

        let second_caller = false_requirement.replace("client", "other");
        let two_callers = format!("{caller}{second_caller}");
        let two_text = module(producer, &two_callers, "out == 7");
        let two = run(&two_text).unwrap();
        assert!(two.active && two.diagnostics.is_empty() && two.symbolic_diagnostic.is_none());
        let [early, later] = two.directives.as_slice() else { panic!("startup and second caller failures"); };
        let snippet = |span: ByteSpan| &two_text[span.start as usize..span.end as usize];
        assert_eq!(snippet(early.condition), "false");
        assert_eq!(snippet(later.condition), "result < 7");
        assert_eq!(later.witness.intervals_at(0, 1).unwrap().collect::<Vec<_>>(), [(&bound, &bound)]);

        let refuted = producer.replace("else { 7 }", "else { 8 }");
        assert!(matches!(run(&module(&refuted, &caller, "out == 7")), Err(ProjectCheck::Unsupported)));
        let discarded = producer.replace("if (input < 0)", &format!("core.narrow(input < 0, {OUTPUT}); if (input < 0)"));
        assert!(matches!(run(&module(&discarded, &caller, "out == 7")), Err(ProjectCheck::Unsupported)));
        let actual = format!("{producer} i16 used = producer(1);");
        assert!(matches!(run(&module(&actual, &caller, "out == 7")), Err(ProjectCheck::Unsupported)));
        assert!(matches!(run(&module(&caller, producer, "out == 7")), Err(ProjectCheck::Unsupported)));
        let sibling = producer.replace("producer", "sibling");
        let both = format!("{producer}{sibling}");
        let redirected = caller.replace("producer(input)", "sibling(input)");
        assert!(matches!(run(&module(&both, &redirected, "out == 7")), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_reuses_distinct_earlier_total_summaries() {
        let app = source("src/main.w", "ordered-automatic-proof-distinct-summaries");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let text = format!(r#"%%start
            i16(i16) low = fn(input) {{ if (input < 0) {{ 7 }} else {{ 7 }} }};
            i16(i16) high = fn(input) {{ if (input < 0) {{ 11 }} else {{ 11 }} }};
            unit(i16) first = fn(input) {{ i16 result = low(input); core.require(result == 11, {OUTPUT}); }};
            unit(i16) second = fn(input) {{ i16 result = high(input); core.require(result == 7, {OUTPUT}); }};
            proof upper for high(entry) -> (out) ensures (out == 11) {{}}
            proof lower for low(entry) -> (out) ensures (out == 7) {{}}
        %%end"#);
        let report = run(&text).unwrap();
        assert!(report.active && report.diagnostics.is_empty() && report.symbolic_diagnostic.is_none());
        let [first, second] = report.directives.as_slice() else { panic!("two false requirements"); };
        assert!(first.directive.start < second.directive.start);
        let seven = BigInt::from(7);
        let eleven = BigInt::from(11);
        assert!(first.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));
        assert!(second.witness.intervals_at(0, 1).unwrap().eq([(&eleven, &eleven)]));

        let refuted = text.replace("else { 11 }", "else { 12 }");
        assert!(matches!(run(&refuted), Err(ProjectCheck::Unsupported)));
        let second_caller = text.lines().find(|line| line.contains("unit(i16) second")).unwrap();
        let early = text.replace(second_caller, "").replace("i16(i16) high", &format!("{second_caller}\n            i16(i16) high"));
        assert!(matches!(run(&early), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_named_proof_checks_actual_return_partitions_from_calling_producer() {
        let app = source("src/main.w", "named-proof-calling-producer");
        let text = format!(r#"%%start
            unit() checked_step = fn {{ core.require(false, {OUTPUT}); }};
            i8(i8) leaf = fn(n) {{ if (n < 0) {{ -3 }} else {{ 7 }} }};
            i8(i8) middle = fn(n) {{ i8 mid = leaf(n); mid }};
            i8(i8) producer = fn(n) {{ checked_step(); i8 produced = middle(n); produced }};
            proof guarded for producer(entry) -> (out) where (entry >= 0) ensures (out == 7) {{}}
        %%end"#);
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[]).ok().unwrap();
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let report = run(&text).unwrap();
        assert!(report.active && report.diagnostics.is_empty() && report.symbolic_diagnostic.is_none());
        assert!(report.directives.len() >= 2);
        let invalid = text.replace("ensures (out == 7)", "ensures (out == 8)");
        let failed = run(&invalid).unwrap();
        let proof = failed.symbolic_diagnostic.unwrap();
        assert_eq!(proof.returned, BigInt::from(7));
        assert!(proof.witness.intervals_at(0, 0).unwrap().eq([(&BigInt::from(0), &BigInt::from(127))]));
        assert!(proof.directive_position > 0);
        let empty = text.replace("entry >= 0", "entry < -128").replace("ensures (out == 7)", "ensures (out == 8)");
        let vacuous = run(&empty).unwrap();
        assert!(vacuous.symbolic_diagnostic.is_none());

        let with_client = text.replace("proof guarded", &format!(
            "unit(i8) client = fn(n) {{ core.narrow(n >= 0, {OUTPUT}); i8 observed = producer(n); core.require(observed == 7, {OUTPUT}); }};\nproof guarded",
        ));
        assert!(matches!(run(&with_client), Err(ProjectCheck::Unsupported)));
        let clean = with_client.replacen(&format!("core.require(false, {OUTPUT});"), "", 1);
        assert!(run(&clean).unwrap().symbolic_diagnostic.is_none());
        let cyclic = text.replace("middle(n); produced", "producer(n); produced");
        assert!(matches!(run(&cyclic), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_named_proof_checks_calling_producer_against_its_own_entry() {
        let app = source("src/main.w", "named-proof-calling-producer-entry");
        let text = format!(r#"%%start
            unit() checked_step = fn {{ core.require(false, {OUTPUT}); }};
            i8(i8) leaf = fn(input) {{ input }};
            i8(i8) wrap = fn(value) {{ i8 middle = leaf(value); middle }};
            i8(i8) producer = fn(value) {{ checked_step(); i8 result = wrap(value); result }};
            proof guarded for producer(entry) -> (out) where (entry >= 0) ensures (out == core.old(entry)) {{}}
        %%end"#);
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[]).ok().unwrap();
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let proved = run(&text).unwrap();
        assert!(proved.active && proved.diagnostics.is_empty() && proved.symbolic_diagnostic.is_none());
        assert!(!proved.directives.is_empty());

        let constant = text.replace("{ input }", "{ if (input < 0) { -3 } else { 7 } }");
        let failed = run(&constant).unwrap();
        let proof = failed.symbolic_diagnostic.unwrap();
        assert_eq!(proof.returned, BigInt::from(7));
        assert!(proof.witness.intervals_at(0, 0).unwrap().eq([
            (&BigInt::from(0), &BigInt::from(6)), (&BigInt::from(8), &BigInt::from(127)),
        ]));
        assert!(proof.directive_position > 0);

        for guard in ["entry == 7", "entry < -128"] {
            let unchanged = constant.replace("entry >= 0", guard);
            assert!(run(&unchanged).unwrap().symbolic_diagnostic.is_none());
        }
        let transitive_write = text.replace("{ input }", "{ input = 0; input }");
        assert!(matches!(run(&transitive_write), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_named_proof_calling_producer_does_not_summarize_narrowed_inputs() {
        let app = source("src/main.w", "named-proof-calling-producer-narrowed");
        let text = format!(r#"%%start
            unit(i8) guard = fn(n) {{ core.narrow(n >= 0, {OUTPUT}); }};
            i8(i8) leaf = fn(n) {{ if (n < 0) {{ -3 }} else {{ 7 }} }};
            i8(i8) producer = fn(n) {{ guard(n); i8 produced = leaf(n); produced }};
            proof guarded for producer(entry) -> (out) ensures (out == 7) {{}}
        %%end"#);
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[]).ok().unwrap();
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let proved = run(&text).unwrap();
        assert!(proved.active && proved.directives.is_empty() && proved.symbolic_diagnostic.is_none());
        let would_manufacture_output = text.replace("proof guarded", &format!(
            "unit(i8) client = fn(n) {{ i8 observed = producer(n); core.require(observed == 7, {OUTPUT}); }};\nproof guarded",
        ));
        assert!(matches!(run(&would_manufacture_output), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_named_proof_calling_producer_obeys_logical_depth() {
        for (depth, admitted) in [(MAX_ACTUAL_CALL_DEPTH, true), (MAX_ACTUAL_CALL_DEPTH + 1, false)] {
            let mut text = "%%start\ni8(i8) f0 = fn(input) { if (input < 0) { -3 } else { 7 } };\n".to_owned();
            for index in 1..depth {
                text.push_str(&format!("i8(i8) f{index} = fn(input) {{ i8 bridge{index} = f{}(input); bridge{index} }};\n", index - 1));
            }
            text.push_str(&format!("proof guarded for f{}(entry) -> (out) where (entry >= 0) ensures (out == 7) {{}}\n%%end", depth - 1));
            let app = source("src/main.w", &format!("named-proof-calling-depth-{depth}"));
            let (_, proof) = crate::parse_proof_source(app.clone(), text, &[]).ok().unwrap();
            let result = check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app], inputs(), ScalarCheckerScope::CurrentPackage,
            );
            if admitted {
                let result = result.unwrap();
                assert!(result.active && result.diagnostics.is_empty() && result.symbolic_diagnostic.is_none());
            } else {
                assert!(matches!(result, Err(ProjectCheck::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_verified_binding_uses_existing_ordered_continuation() {
        let app = source("src/main.w", "ordered-automatic-proof-continuation");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let text = format!(r#"%%start
            i16(i16) producer = fn(input) {{ if (input < 0) {{ input + 1 }} else {{ 7 }} }};
            unit(i16) client = fn(input) {{
                core.narrow(input >= 0, {OUTPUT});
                i16 result = producer(input);
                core.require(result < 7 || result > 7, {OUTPUT});
                core.require(result == 8, {OUTPUT});
            }};
            proof bound for producer(entry) -> (out) where (entry >= 0) ensures (out == 7) {{}}
        %%end"#);
        let report = run(&text).unwrap();
        let [first, second] = report.directives.as_slice() else { panic!("two failed requirements"); };
        assert_eq!(&text[first.condition.start as usize..first.condition.end as usize], "result < 7 || result > 7");
        assert_eq!(&text[second.condition.start as usize..second.condition.end as usize], "result == 8");
        let seven = BigInt::from(7);
        for failure in [first, second] {
            assert_eq!(failure.operation, ScalarCheckerOperation::Require);
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));
        }
        let late = text.replace("result == 8", "input < 7 || result > 7");
        assert!(matches!(run(&late), Err(ProjectCheck::Unsupported)));
        let dead = text.replace("input >= 0", "input < -32768");
        let report = run(&dead).unwrap();
        let [failure] = report.directives.as_slice() else { panic!("only the fatal Narrow"); };
        assert_eq!(failure.operation, ScalarCheckerOperation::Narrow);
        let overlap = text.replace("input >= 0", "input >= -1");
        assert!(matches!(run(&overlap), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_reuses_two_verified_bindings_in_one_caller() {
        let app = source("src/main.w", "ordered-automatic-proof-two-bindings");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let text = format!(r#"%%start
            i16(i16) low = fn(input) {{ if (input < 0) {{ 7 }} else {{ 7 }} }};
            i16(i16) high = fn(input) {{ if (input < 5) {{ input + 1 }} else {{ 11 }} }};
            unit(i16) client = fn(input) {{
                i16 first = low(input);
                core.require(first == 8, {OUTPUT});
                i16 second = high(first);
                core.require(second == 12, {OUTPUT});
            }};
            proof upper for high(entry) -> (out) where (entry >= 5) ensures (out == 11) {{}}
            proof lower for low(entry) -> (out) ensures (out == 7) {{}}
        %%end"#);
        let report = run(&text).unwrap();
        assert!(report.active && report.diagnostics.is_empty() && report.symbolic_diagnostic.is_none());
        let [first, second] = report.directives.as_slice() else { panic!("two ordered requirements"); };
        let span_text = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(span_text(first.condition), "first == 8");
        assert_eq!(span_text(second.condition), "second == 12");
        let seven = BigInt::from(7);
        let eleven = BigInt::from(11);
        assert!(first.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));
        assert!(first.witness.intervals_at(0, 2).is_none());
        assert!(second.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));
        assert!(second.witness.intervals_at(0, 2).unwrap().eq([(&eleven, &eleven)]));

        let valid = text.replace("first == 8", "first == 7").replace("second == 12", "second == 11");
        let all_pass = run(&valid).unwrap();
        assert!(all_pass.active && all_pass.directives.is_empty()
            && all_pass.diagnostics.is_empty() && all_pass.symbolic_diagnostic.is_none());

        let outside = text.replace("high(first)", "high(4)");
        assert!(matches!(run(&outside), Err(ProjectCheck::Unsupported)));
        let fatal = outside.replace("i16 second = high(4);", &format!("core.narrow(false, {OUTPUT}); i16 second = high(4);"));
        let stopped = run(&fatal).unwrap();
        let [before, narrow] = stopped.directives.as_slice() else { panic!("only checks before the unreachable binding"); };
        assert_eq!(before.operation, ScalarCheckerOperation::Require);
        assert_eq!(narrow.operation, ScalarCheckerOperation::Narrow);
        assert!(narrow.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));
        assert!(narrow.witness.intervals_at(0, 2).is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_reuses_the_full_sequence_of_verified_root_bindings() {
        let app = source("src/main.w", "ordered-automatic-proof-complete-bindings");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let text = format!(r#"%%start
            i16(i16) low = fn(input) {{ if (input < 0) {{ 7 }} else {{ 7 }} }};
            i16(i16) high = fn(input) {{ if (input < 5) {{ input + 1 }} else {{ 11 }} }};
            unit(i16) client = fn(input) {{
                i16 first = low(input);
                core.require(first == 8, {OUTPUT});
                i16 second = high(first);
                i16 third = low(second);
                core.require(third == 8, {OUTPUT});
                i16 fourth = high(third);
                core.require(fourth == 12, {OUTPUT});
            }};
            proof upper for high(entry) -> (out) where (entry >= 5) ensures (out == 11) {{}}
            proof lower for low(entry) -> (out) ensures (out == 7) {{}}
        %%end"#);
        let report = run(&text).unwrap();
        assert!(report.active && report.diagnostics.is_empty() && report.symbolic_diagnostic.is_none());
        let [first, third, fourth] = report.directives.as_slice() else { panic!("three ordered requirements"); };
        let span_text = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!(span_text(first.condition), "first == 8");
        assert_eq!(span_text(third.condition), "third == 8");
        assert_eq!(span_text(fourth.condition), "fourth == 12");
        let seven = BigInt::from(7);
        let eleven = BigInt::from(11);
        assert!(first.witness.intervals_at(0, 2).is_none());
        assert!(third.witness.intervals_at(0, 2).unwrap().eq([(&eleven, &eleven)]));
        assert!(third.witness.intervals_at(0, 3).unwrap().eq([(&seven, &seven)]));
        assert!(third.witness.intervals_at(0, 4).is_none());
        assert!(fourth.witness.intervals_at(0, 4).unwrap().eq([(&eleven, &eleven)]));

        let valid = text.replace("first == 8", "first == 7")
            .replace("third == 8", "third == 7").replace("fourth == 12", "fourth == 11");
        let valid = run(&valid).unwrap();
        assert!(valid.active && valid.directives.is_empty() && valid.diagnostics.is_empty());

        let outside = text.replace("high(third)", "high(4)");
        assert!(matches!(run(&outside), Err(ProjectCheck::Unsupported)));
        let fatal = outside.replace("i16 fourth = high(4);", &format!("core.narrow(false, {OUTPUT}); i16 fourth = high(4);"));
        let stopped = run(&fatal).unwrap();
        let [before, later, narrow] = stopped.directives.as_slice() else { panic!("only checks before the unreachable binding"); };
        assert_eq!(before.operation, ScalarCheckerOperation::Require);
        assert_eq!(later.operation, ScalarCheckerOperation::Require);
        assert_eq!(narrow.operation, ScalarCheckerOperation::Narrow);
        assert!(narrow.witness.intervals_at(0, 3).unwrap().eq([(&seven, &seven)]));
        assert!(narrow.witness.intervals_at(0, 4).is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_reuses_guarded_proof_of_a_conjunctive_producer() {
        let app = source("src/main.w", "ordered-conjunctive-proof-producer");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let producer = "i8(i8, u8) producer = fn(entry, limit) { if (entry >= 0 && limit >= 0) { 7 } else { 9 } };";
        let caller = format!("unit() client = fn() {{ i8 result = producer(0, 0); core.require(result == 8, {OUTPUT}); }};");
        let package = |producer: &str, caller: &str| format!(r#"%%start
            {producer}
            {caller}
            proof bound for producer(entry, limit) -> (out) where (entry >= 0) ensures (out == 7) {{}}
        %%end"#);
        let valid = run(&package(producer, &caller)).unwrap();
        assert!(valid.active && valid.diagnostics.is_empty() && valid.symbolic_diagnostic.is_none());
        let [failure] = valid.directives.as_slice() else { panic!("one caller requirement"); };
        let seven = BigInt::from(7);
        assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
        let either = producer.replace("&& limit >= 0", "|| limit > 0");
        let disjunctive = run(&package(&either, &caller)).unwrap();
        assert!(disjunctive.active && disjunctive.diagnostics.is_empty() && disjunctive.symbolic_diagnostic.is_none());
        let [failure] = disjunctive.directives.as_slice() else { panic!("one caller requirement"); };
        assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
        let outside = caller.replace("producer(0, 0)", "producer(-1, 1)");
        assert!(matches!(run(&package(&either, &outside)), Err(ProjectCheck::Unsupported)));
        let outside_conjunction = caller.replace("producer(0, 0)", "producer(-1, 0)");
        assert!(matches!(run(&package(producer, &outside_conjunction)), Err(ProjectCheck::Unsupported)));

        let bad = producer.replace("limit >= 0", "limit > 0");
        let rejected = run(&package(&bad, "")).unwrap();
        assert!(rejected.active && rejected.directives.is_empty() && rejected.symbolic_diagnostic.is_some());
        let computed = producer.replace("limit >= 0", "limit >= limit");
        assert!(matches!(run(&package(&computed, "")), Err(ProjectCheck::Unsupported)));
        let effectful = producer.replace("limit >= 0", "predicate(limit) >= 0");
        let effectful = format!(r#"u8(u8) predicate = fn(limit) {{
            core.require(limit == 0, {OUTPUT});
            limit
        }}; {effectful}"#);
        assert!(matches!(run(&package(&effectful, &caller)), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_guarded_equality_requires_total_admitted_producer_and_contained_caller() {
        let app = source("src/main.w", "ordered-automatic-proof-guarded-summary");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let producer = "i16(i16) producer = fn(input) { if (input < 0) { input + 1 } else { 7 } };";
        let caller = format!("unit() client = fn() {{ i16 result = producer(0); core.require(result == 7, {OUTPUT}); }};");
        let module = |producer: &str, caller: &str, guard: &str| format!(r#"%%start
            {producer}
            {caller}
            proof bound for producer(entry) -> (out) where ({guard}) ensures (out == 7) {{}}
        %%end"#);
        let exact = module(producer, &caller, "entry >= 0");
        let report = run(&exact).unwrap();
        assert!(report.active && report.directives.is_empty() && report.symbolic_diagnostic.is_none());

        let outside = caller.replace("producer(0)", "producer(-1)");
        assert!(matches!(run(&module(producer, &outside, "entry >= 0")), Err(ProjectCheck::Unsupported)));
        let overlap = caller.replace("unit() client = fn()", "unit(i16) client = fn(input)")
            .replace("producer(0)", "producer(input)");
        assert!(matches!(run(&module(producer, &overlap, "entry >= 0")), Err(ProjectCheck::Unsupported)));

        let narrowed = overlap.replace("i16 result = producer(input);", &format!("core.narrow(input >= 0, {OUTPUT}); i16 result = producer(input);"));
        let used = run(&module(producer, &narrowed, "entry >= 0")).unwrap();
        assert!(used.active && used.directives.is_empty() && used.symbolic_diagnostic.is_none());
        let covered = narrowed.replace("result == 7", "result < 6 || result > 6");
        let covered = run(&module(producer, &covered, "entry >= 0")).unwrap();
        assert!(covered.active && covered.directives.is_empty() && covered.symbolic_diagnostic.is_none());
        let hole = narrowed.replace("result == 7", "result < 7 || result > 7");
        let hole_source = module(producer, &hole, "entry >= 0");
        let failed = run(&hole_source).unwrap();
        let [failure] = failed.directives.as_slice() else { panic!("only the disjunctive Require fails"); };
        assert_eq!(failure.operation, ScalarCheckerOperation::Require);
        assert_eq!(failure.condition.start as usize, hole_source.find("result < 7").unwrap());
        let seven = BigInt::from(7);
        assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));
        assert!(failed.symbolic_diagnostic.is_none());
        let wrong_axis = hole.replace("result < 7 || result > 7", "input < 7 || input > 7");
        assert!(matches!(run(&module(producer, &wrong_axis, "entry >= 0")),
            Err(ProjectCheck::Invalid(diagnostics)) if diagnostics.iter().any(|diagnostic| diagnostic.code == "B0009")));
        let mixed_axes = hole.replace("result < 7 || result > 7", "input < 7 || result > 7");
        assert!(matches!(run(&module(producer, &mixed_axes, "entry >= 0")), Err(ProjectCheck::Unsupported)));

        let partial = narrowed.replace("input >= 0", "input >= -1");
        assert!(matches!(run(&module(producer, &partial, "entry >= 0")), Err(ProjectCheck::Unsupported)));
        let late = partial.replace("client", "late");
        assert!(matches!(run(&module(producer, &format!("{hole} {late}"), "entry >= 0")), Err(ProjectCheck::Unsupported)));
        let empty_caller = narrowed.replace("input >= 0", "input < -32768");
        let dead = run(&module(producer, &empty_caller, "entry >= 0")).unwrap();
        assert!(dead.active && dead.symbolic_diagnostic.is_none());
        let [failure] = dead.directives.as_slice() else { panic!("only the fatal Narrow is reached"); };
        assert_eq!(failure.operation, ScalarCheckerOperation::Narrow);

        let inside = producer.replace("{ input + 1 } else { 7 }", "{ 7 } else { input + 1 }");
        assert!(matches!(run(&module(&inside, &caller, "entry >= 0")), Err(ProjectCheck::Unsupported)));
        let empty = run(&module(producer, "", "entry < -32768")).unwrap();
        assert!(!empty.active && empty.directives.is_empty() && empty.symbolic_diagnostic.is_none());
        assert!(matches!(run(&module(producer, &caller, "entry < -32768")), Err(ProjectCheck::Unsupported)));
        let effectful = producer.replace("if (input < 0)", &format!("core.require(true, {OUTPUT}); if (input < 0)"));
        assert!(matches!(run(&module(&effectful, &caller, "entry >= 0")), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proof_filters_return_witnesses_without_filtering_directives() {
        let app = source("src/main.w", "ordered-automatic-proof-unused-where");
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), ScalarCheckerScope::CurrentPackage,
            )
        };
        let guarded = |where_guard: &str, goal: &str| format!(r#"%%start
            u8(u8) checked = fn(value) {{ core.require(value < 2, {OUTPUT}); value }};
            proof bound for checked(entry) -> (out) where ({where_guard}) ensures ({goal}) {{}}
        %%end"#);
        let intervals = |witness: &ScalarCheckerIntegerWitness, axis| witness.intervals_at(0, axis).unwrap()
            .map(|(first, last)| (first.clone(), last.clone())).collect::<Vec<_>>();
        let passing = run(&guarded("entry != 3", "out != 3")).unwrap();
        assert!(passing.active && passing.diagnostics.is_empty() && passing.symbolic_diagnostic.is_none());
        assert_eq!(passing.directives.len(), 1);
        assert_eq!(intervals(&passing.directives[0].witness, 0), [(BigInt::from(2), BigInt::from(255))]);
        let refuted = run(&guarded("entry > 6", "out != 7")).unwrap();
        let failure = refuted.symbolic_diagnostic.unwrap();
        assert_eq!(failure.directive_position, 1);
        assert_eq!(failure.returned, BigInt::from(7));
        assert_eq!(intervals(&failure.witness, 0), [(BigInt::from(7), BigInt::from(7))]);
        assert_eq!(intervals(&refuted.directives[0].witness, 0), [(BigInt::from(2), BigInt::from(255))]);
        let tautology = run(&guarded("true && true", "out != 7")).unwrap();
        assert_eq!(tautology.directives.len(), 1);
        let tautology_failure = tautology.symbolic_diagnostic.unwrap();
        assert_eq!(tautology_failure.returned, BigInt::from(7));
        let no_entry = r#"%%start
            u8(u8) checked = fn(value) { value };
            proof bound for checked(entry) -> (out) where (entry > 255) ensures (out == 0) {}
        %%end"#;
        let no_entry = run(no_entry).unwrap();
        assert!(!no_entry.active && no_entry.directives.is_empty() && no_entry.symbolic_diagnostic.is_none());
        let no_return = format!(r#"%%start
            u8(u8) checked = fn(value) {{
                if (value > 6) {{ core.narrow(false, {OUTPUT}); value }} else {{ value }}
            }};
            proof bound for checked(entry) -> (out) where (entry > 6) ensures (out != 7) {{}}
        %%end"#);
        let no_return = run(&no_return).unwrap();
        assert!(no_return.active && no_return.directives.len() == 1 && no_return.symbolic_diagnostic.is_none());

        let two_axes = |goal: &str| format!(r#"%%start
            u16(u8, u16) checked = fn(tag, value) {{
                if (tag == 1) {{ if (value == 1000) {{ value }} else {{ 0 }} }} else {{ value }}
            }};
            proof bound for checked(kind, entry) -> (out) where (kind == 1) ensures ({goal}) {{}}
        %%end"#);
        let separated = run(&two_axes("out <= 1000")).unwrap();
        assert!(separated.active && separated.directives.is_empty() && separated.symbolic_diagnostic.is_none());
        let exact = run(&two_axes("out < 1000")).unwrap().symbolic_diagnostic.unwrap();
        assert_eq!(exact.returned, BigInt::from(1000));
        assert_eq!(intervals(&exact.witness, 0), [(BigInt::from(1), BigInt::from(1))]);
        assert_eq!(intervals(&exact.witness, 1), [(BigInt::from(1000), BigInt::from(1000))]);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_unused_ranked_proof_preserves_dormant_source_and_directive_order() {
        let app = source("src/main.w", "ordered-ranked-app");
        let dormant = source("src/routes.w", "ordered-ranked-routes");
        let main = format!("%%start core.require(false, {OUTPUT}); %%end");
        let routes = r#"%%start
            u8(u8) descend = fn(value) { if (value < 3) { 4 } else { descend(value - 3) } };
            proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out <= 3) decreases (entry) {
                if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
            }
        %%end"#;
        let catalog = |identity: &SourceIdentity, text: &str| crate::parse_proof_source(
            identity.clone(), text.to_owned(), &[],
        ).unwrap_or_else(|_| panic!("valid ranked proof source")).1;
        let run = |text: &str, scope| check_loaded_scalar_ordered_local_proof_contexts(
            vec![(declarations(&app, &main), Vec::new())], [], vec![app.clone()], inputs(), scope,
            vec![catalog(&dormant, text)],
        ).unwrap();
        let failed = run(routes, ScalarCheckerScope::CurrentPackage);
        let passed = run(&routes.replace("out <= 3", "out <= 4"), ScalarCheckerScope::CurrentPackage);
        let excluded = run(routes, ScalarCheckerScope::MainFlowOnly);
        assert_eq!(failed.directives.len(), 1);
        assert_eq!(failed.directives, passed.directives);
        assert_eq!(failed.directives, excluded.directives);
        assert!(passed.symbolic_diagnostic.is_none() && excluded.symbolic_diagnostic.is_none());
        let symbolic = failed.symbolic_diagnostic.unwrap();
        let snippet = |span: &wosy_syntax::SourceSpan| {
            assert_eq!(span.source, dormant);
            &routes[span.range.start as usize..span.range.end as usize]
        };
        assert_eq!(snippet(&symbolic.goal), "out <= 3");
        assert_eq!(snippet(&symbolic.callable), "descend");
        assert_eq!(snippet(&symbolic.output), "4");
        assert!(symbolic.failed_step.is_none());
        assert_eq!(symbolic.returned, BigInt::from(4));
        assert_eq!(symbolic.directive_position, 1);
        let zero = BigInt::from(0);
        assert_eq!(symbolic.witness.intervals_at(0, 0).unwrap().collect::<Vec<_>>(), [(&zero, &zero)]);

        let reached = routes.replace("%%end", "u8 reached = descend(0); %%end");
        let mixed = routes.replace("%%end", "proof bare for descend(entry) -> (out) ensures (out == 4) {} %%end");
        for source in [&reached, &mixed] {
            for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
                let receipt = check_loaded_scalar_ordered_local_proof_contexts(
                    Vec::new(), [(catalog(&app, source), Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
                );
                assert!(matches!(receipt, Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_reached_ranked_equality_uses_only_independently_admitted_calls() {
        let app = source("src/main.w", "ordered-reached-rank");
        let text = r#"%%start
            u8(u8) descend = fn(value) { if (value < 3) { 4 } else { descend(value - 3) } };
            proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
                if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
            }
            core.require(false, { .stdout = "before"; .stderr = ""; .code = 17; .files = []; });
            u8 base = descend(2);
            u8 recursive = descend(8);
            core.require(base == 4, { .stdout = "bad base"; .stderr = ""; .code = 18; .files = []; });
            core.require(recursive == 4, { .stdout = "bad recursion"; .stderr = ""; .code = 19; .files = []; });
            core.require(false, { .stdout = "after"; .stderr = ""; .code = 20; .files = []; });
        %%end"#;
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid ranked proof source"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let checked = run(text, scope).unwrap();
            assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
            assert!(checked.additional_symbolic_diagnostics.is_empty());
            assert_eq!(checked.directives.iter().map(|record| record.stdout()).collect::<Vec<_>>(),
                [b"before".as_slice(), b"after".as_slice()]);
            for rejected in [
                text.replace("descend(8);", "descend(9);"),
                text.replace("core.show(out == 4)", "core.show(out == 5)"),
                text.replace("core.use(rank(entry - 3))", "core.use(rank(entry - 2))"),
                text.replace("ensures (out == 4)", "ensures (out <= 4)"),
            ] {
                assert!(matches!(run(&rejected, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_signed_rank_keeps_exact_disjoint_entry_domains() {
        let app = source("src/main.w", "ordered-signed-rank-domains");
        let proof_source = r#"%%start
            i8(i8) descend = fn(value) { if (value < -119) { -7 } else { descend(value - 10) } };
            proof rank for descend(entry) -> (out)
                where (entry == -120 || entry == -110) ensures (out == -7) decreases (entry) {
                if (entry < -119) { core.show(out == -7); } else { core.use(rank(entry - 10)); }
            }
        %%end"#;
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid signed rank"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        let startup = format!(r#"
            i8 recursive = descend(-110);
            core.require(recursive == -7, {OUTPUT});
            core.require(false, {OUTPUT});
        %%end"#);
        let reached = proof_source.replace("%%end", &startup);
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            let checked = run(&reached, scope).unwrap();
            assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
            let [directive] = checked.directives.as_slice() else { panic!("one unchanged actual directive"); };
            assert_eq!(&reached[directive.condition.start as usize..directive.condition.end as usize], "false");
            let hole = reached.replace("descend(-110)", "descend(-115)");
            let outside = reached.replace("descend(-110)", "descend(-121)");
            let nonclosed = reached.replace(" - 10", " - 9");
            let underflow = reached.replace("entry == -110", "entry == -128")
                .replace(" < -119", " < -128").replace("descend(-110)", "descend(-128)");
            for invalid in [hole, outside, nonclosed, underflow] {
                assert!(matches!(run(&invalid, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
        let refuted = proof_source.replace("out == -7) decreases", "out == -8) decreases");
        let checked = run(&refuted, ScalarCheckerScope::CurrentPackage).unwrap();
        let witness = checked.symbolic_diagnostic.unwrap();
        assert_eq!(witness.returned, BigInt::from(-7));
        assert!(witness.failed_step.is_none());
        assert_eq!(&refuted[witness.goal.range.start as usize..witness.goal.range.end as usize], "out == -8");
        let entry = BigInt::from(-120);
        assert!(witness.witness.intervals_at(0, 0).unwrap().eq([(&entry, &entry)]));
        let dead = proof_source.replace("where (entry == -120 || entry == -110)",
            "where ((entry == -120 || entry == -110) && entry == -115)");
        let checked = run(&dead, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(!checked.active && checked.symbolic_diagnostic.is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_lexicographic_prefix_decrease_admits_only_exact_suffix_reset() {
        let app = source("src/main.w", "ordered-prefix-reset");
        let proof_source = r#"%%start
            u8(u8, u8) descend = fn(page, slot) {
                if (page == 0) { if (slot >= 1) { 7 } else { 8 } } else { descend(page - 1, 5) }
            };
            proof rank for descend(page, slot) -> (out)
                where (page <= 4 && slot >= 1 && slot <= 5) ensures (out == 7) decreases (page, slot) {
                if (page == 0) { core.show(out == 7); } else { core.use(rank(page - 1, 5)); }
            }
        %%end"#;
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid prefix-reset rank"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        let startup = format!(r#"
            u8 actual = descend(4, 1);
            core.require(actual == 7, {OUTPUT});
            core.require(false, {OUTPUT});
        %%end"#);
        let reached = proof_source.replace("%%end", &startup);
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            let checked = run(&reached, scope).unwrap();
            assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
            let [directive] = checked.directives.as_slice() else { panic!("one unchanged actual directive"); };
            assert_eq!(&reached[directive.condition.start as usize..directive.condition.end as usize], "false");
            for invalid in [
                reached.replace("rank(page - 1, 5)", "rank(page - 1, 4)"),
                reached.replace("descend(page - 1, 5)", "descend(page - 1, slot)"),
                reached.replace("page - 1", "page - 0"),
                reached.replace("page == 0", "page < 0"),
                reached.replace("page <= 4", "page >= 1"),
                reached.replace("page - 1, 5", "page - 1, 0"),
                reached.replace("descend(4, 1)", "descend(4, 0)"),
                reached.replace("&& slot >= 1 &&", "&&"),
            ] {
                assert!(matches!(run(&invalid, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
        let refuted = proof_source.replace("out == 7) decreases", "out == 8) decreases");
        let checked = run(&refuted, ScalarCheckerScope::CurrentPackage).unwrap();
        let witness = checked.symbolic_diagnostic.unwrap();
        assert_eq!(witness.returned, BigInt::from(7));
        assert!(witness.failed_step.is_none());
        assert_eq!(&refuted[witness.goal.range.start as usize..witness.goal.range.end as usize], "out == 8");
        assert_eq!(&refuted[witness.output.range.start as usize..witness.output.range.end as usize], "7");
        let (page, slot) = (BigInt::from(0), BigInt::from(1));
        assert!(witness.witness.intervals_at(0, 0).unwrap().eq([(&page, &page)]));
        assert!(witness.witness.intervals_at(0, 1).unwrap().eq([(&slot, &slot)]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_lexicographic_rank_checks_exact_calls_and_base_witness() {
        let app = source("src/main.w", "ordered-lexicographic-rank");
        let proof_source = r#"%%start
            u8(i16, u8) descend = fn(tag, value) { if (value < 3) { 4 } else { descend(tag, value - 3) } };
            proof rank for descend(tag, entry) -> (out)
                where (tag >= 11 && entry <= 8) ensures (out == 4) decreases (tag, entry) {
                if (entry < 3) { core.show(out == 4); } else { core.use(rank(tag, entry - 3)); }
            }
        %%end"#;
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid lexicographic rank"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        let startup = format!(r#"
            u8 actual = descend(11, 8);
            core.require(actual == 4, {OUTPUT});
            core.require(false, {OUTPUT});
        %%end"#);
        let reached = proof_source.replace("%%end", &startup);
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            let checked = run(&reached, scope).unwrap();
            assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
            assert_eq!(checked.directives.len(), 1);
            let condition = checked.directives[0].condition;
            assert_eq!(&reached[condition.start as usize..condition.end as usize], "false");
            for invalid in [
                reached.replace("descend(11, 8)", "descend(10, 8)"),
                reached.replace("descend(11, 8)", "descend(11, 9)"),
                reached.replace("descend(tag, value - 3)", "descend(tag + 1, value - 3)"),
                reached.replace("rank(tag, entry - 3)", "rank(tag + 1, entry - 3)"),
            ] {
                assert!(matches!(run(&invalid, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
        let refuted = proof_source.replace("out == 4) decreases", "out == 5) decreases")
            .replace("entry <= 8", "entry == 2");
        let current = run(&refuted, ScalarCheckerScope::CurrentPackage).unwrap();
        let witness = current.symbolic_diagnostic.unwrap();
        assert_eq!(witness.returned, BigInt::from(4));
        assert!(witness.failed_step.is_none());
        assert_eq!(&refuted[witness.goal.range.start as usize..witness.goal.range.end as usize], "out == 5");
        let [tag, value] = [BigInt::from(11), BigInt::from(2)];
        assert!(witness.witness.intervals_at(0, 0).unwrap().eq([(&tag, &tag)]));
        assert!(witness.witness.intervals_at(0, 1).unwrap().eq([(&value, &value)]));
        let excluded = run(&refuted, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(!excluded.active && excluded.symbolic_diagnostic.is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_qualified_ranked_equality_uses_only_owning_reachable_local_catalog() {
        let app = source("src/main.w", "ordered-qualified-rank-app");
        let local = source("src/lib.w", "ordered-qualified-rank-library");
        let main = r#"%%start
            lib = namespace demo "src/lib.w";
            u8 base = lib.descend(2);
            u8 recursed = lib.descend(8);
            core.require(base == 4, { .stdout = "wrong base"; .stderr = ""; .code = 20; .files = []; });
            core.require(recursed == 4, { .stdout = "wrong recurrence"; .stderr = ""; .code = 21; .files = []; });
            core.require(false, { .stdout = "root only"; .stderr = ""; .code = 22; .files = []; });
        %%end"#;
        let library = r#"%%start
            u8(u8) descend = fn(value) { if (value < 3) { 4 } else { descend(value - 3) } };
            proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
                if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
            }
            core.require(false, { .stdout = "lib once"; .stderr = ""; .code = 17; .files = []; });
        %%end"#;
        let run = |main: &str, library: &str, owner: &SourceIdentity, scope| {
            let (_, catalog) = crate::parse_proof_source(owner.clone(), library.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid qualified ranked proof"));
            let import = ScalarNamespaceBinding {
                binding: "lib".into(), target: owner.clone(), span: ByteSpan::new(0, 0),
            };
            check_loaded_scalar_ordered_local_proof_contexts(
                vec![(declarations(&app, main), vec![import])], [(catalog, Vec::new())],
                vec![app.clone(), owner.clone()], inputs(), scope, Vec::new(),
            )
        };
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            let receipt = run(main, library, &local, scope).unwrap();
            assert!(receipt.active && receipt.diagnostics.is_empty() && receipt.symbolic_diagnostic.is_none());
            assert!(receipt.additional_symbolic_diagnostics.is_empty());
            let [first, second] = receipt.directives.as_slice() else { panic!("exactly one each in initialization order"); };
            assert_eq!((&first.source, first.stdout()), (&local, b"lib once".as_slice()));
            assert_eq!((&second.source, second.stdout()), (&app, b"root only".as_slice()));
            assert!(matches!(run(&main.replace("descend(8)", "descend(9)"), library, &local, scope),
                Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let foreign = SourceIdentity { project: "other-project".into(), ..local.clone() };
            assert!(matches!(run(main, library, &foreign, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_distinct_ranked_and_bare_proofs_keep_actual_and_symbolic_order() {
        let app = source("src/main.w", "ordered-mixed-rank");
        let ranked = r#"proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
            if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
        }"#;
        let bare = "proof leaf_cap for leaf(entry) -> (out) ensures (out != 7) {}";
        let actual = r#"
            u8 recursed = descend(8);
            u8 nominal = leaf(7);
            core.require(recursed == 4, { .stdout = "wrong result"; .stderr = ""; .code = 17; .files = []; });
            core.require(false, { .stdout = "only once"; .stderr = ""; .code = 18; .files = []; });
        "#;
        let run = |proofs: [&str; 2], startup: &str, scope| {
            let [first, second] = proofs;
            let text = format!(r#"%%start
                u8(u8) descend = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend(value - 3) }} }};
                u8(u8) leaf = fn(value) {{ value }};
                {first}
                {second}
                {startup}
            %%end"#);
            let (_, proof) = crate::parse_proof_source(app.clone(), text, &[])
                .unwrap_or_else(|_| panic!("valid distinct proofs"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        for proofs in [[ranked, bare], [bare, ranked]] {
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let receipt = run(proofs, actual, scope).unwrap();
                let [diagnostic] = receipt.diagnostics.as_slice() else { panic!("one exact reached bare failure"); };
                assert_eq!(diagnostic.invocation.returned, BigInt::from(7));
                assert_eq!(diagnostic.proof.source, app);
                assert!(receipt.symbolic_diagnostic.is_none() && receipt.additional_symbolic_diagnostics.is_empty());
                let [directive] = receipt.directives.as_slice() else { panic!("one untouched directive"); };
                assert_eq!(directive.stdout(), b"only once");
            }
        }
        let refuted = ranked.replace("ensures (out == 4)", "ensures (out == 3)");
        for proofs in [[refuted.as_str(), bare], [bare, refuted.as_str()]] {
            let receipt = run(proofs, "", ScalarCheckerScope::CurrentPackage).unwrap();
            assert!(receipt.active && receipt.diagnostics.is_empty() && receipt.directives.is_empty());
            assert_eq!(receipt.symbolic_diagnostic.unwrap().returned, BigInt::from(4));
            let [additional] = receipt.additional_symbolic_diagnostics.as_slice() else { panic!("second symbolic failure"); };
            assert_eq!(additional.returned, BigInt::from(7));
            let excluded = run(proofs, "", ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!excluded.active && excluded.symbolic_diagnostic.is_none());
        }
        let duplicate = bare.replace("for leaf", "for descend");
        let duplicate_ranked = ranked.replace("proof rank", "proof alternate").replace("core.use(rank(", "core.use(alternate(");
        for proofs in [[ranked, duplicate.as_str()], [duplicate.as_str(), ranked], [ranked, duplicate_ranked.as_str()]] {
            assert!(matches!(run(proofs, actual, ScalarCheckerScope::CurrentPackage),
                Err(ScalarOrderedCheckerOutcome::Unsupported)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_rank_types_are_independent_of_checked_result_types() {
        let app = source("src/main.w", "ordered-rank-result-types");
        let program = r#"%%start
            u16(i8) descend = fn(value) { if (value == -100) { 1000 } else { descend(value - 1) } };
            proof rank for descend(entry) -> (out) where (entry >= -100) ensures (out == 1000) decreases (entry) {
                if (entry == -100) { core.show(out == 1000); } else { core.use(rank(entry - 1)); }
            }
        %%end"#;
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid mixed-width recurrence"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        let actual = program.replace("%%end", &format!(r#"
            u16 answer = descend(-95);
            core.require(answer == 1000, {OUTPUT});
            core.require(false, {OUTPUT});
        %%end"#));
        for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
            let receipt = run(&actual, scope).unwrap();
            let [directive] = receipt.directives.as_slice() else { panic!("only the unconditional directive"); };
            assert_eq!(&actual[directive.condition.start as usize..directive.condition.end as usize], "false");
            assert!(receipt.symbolic_diagnostic.is_none() && receipt.diagnostics.is_empty());
            let outside = actual.replace("descend(-95)", "descend(-101)");
            assert!(matches!(run(&outside, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
        }
        let symbolic = program.replace("%%end", &format!(r#"
            unit(i8) consumer = fn(input) {{
                core.narrow(input >= -100, {OUTPUT});
                u16 answer = descend(input);
                core.require(answer == 1000, {OUTPUT});
                core.require(answer == 1001, {OUTPUT});
            }};
        %%end"#));
        let receipt = run(&symbolic, ScalarCheckerScope::CurrentPackage).unwrap();
        let [directive] = receipt.directives.as_slice() else { panic!("one symbolic failure"); };
        let answer = BigInt::from(1000);
        assert!(directive.witness.intervals_at(0, 1).unwrap().eq([(&answer, &answer)]));

        let refuted = program.replace("ensures (out == 1000)", "ensures (out == 1001)");
        let receipt = run(&refuted, ScalarCheckerScope::CurrentPackage).unwrap();
        let diagnostic = receipt.symbolic_diagnostic.unwrap();
        let base = BigInt::from(-100);
        assert_eq!(diagnostic.returned, answer);
        assert_eq!(diagnostic.witness.axes[0].1, ScalarType::I8);
        assert!(diagnostic.witness.intervals_at(0, 0).unwrap().eq([(&base, &base)]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_distinct_ranked_targets_share_one_actual_and_symbolic_flow() {
        let app = source("src/main.w", "ordered-multiple-rank");
        let rank_a = r#"proof rank_a for descend_a(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
            if (entry < 3) { core.show(out == 4); } else { core.use(rank_a(entry - 3)); }
        }"#;
        let rank_b = r#"proof rank_b for descend_b(entry) -> (out) where (entry <= 6) ensures (out == 7) decreases (entry) {
            if (entry < 2) { core.show(out == 7); } else { core.use(rank_b(entry - 2)); }
        }"#;
        let actual = r#"
            u8 a = descend_a(8);
            core.require(a == 4, { .stdout = "bad a"; .stderr = ""; .code = 17; .files = []; });
            core.require(false, { .stdout = "between"; .stderr = ""; .code = 18; .files = []; });
            u8 b = descend_b(6);
            core.require(b == 7, { .stdout = "bad b"; .stderr = ""; .code = 19; .files = []; });
            core.require(false, { .stdout = "last"; .stderr = ""; .code = 20; .files = []; });
        "#;
        let symbolic = format!(r#"
            unit(u8) client = fn(input) {{
                core.narrow(input <= 6, {OUTPUT});
                u8 a = descend_a(input);
                u8 b = descend_b(input);
                core.require(a == 4, {OUTPUT});
                core.require(b == 7, {OUTPUT});
                core.require(b == 8, {OUTPUT});
            }};
        "#);
        let run = |proofs: [&str; 2], body: &str, scope| {
            let [first, second] = proofs;
            let text = format!(r#"%%start
                u8(u8) descend_a = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend_a(value - 3) }} }};
                u8(u8) descend_b = fn(value) {{ if (value < 2) {{ 7 }} else {{ descend_b(value - 2) }} }};
                {first}
                {second}
                {body}
            %%end"#);
            let (_, proof) = crate::parse_proof_source(app.clone(), text, &[])
                .unwrap_or_else(|_| panic!("valid distinct ranked targets"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        for proofs in [[rank_a, rank_b], [rank_b, rank_a]] {
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let checked = run(proofs, actual, scope).unwrap();
                assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
                assert_eq!(checked.directives.iter().map(|record| record.stdout()).collect::<Vec<_>>(),
                    [b"between".as_slice(), b"last".as_slice()]);
                for bad in [
                    actual.replace("descend_a(8)", "descend_a(9)"),
                    actual.replace("descend_b(6)", "descend_b(7)"),
                ] {
                    assert!(matches!(run(proofs, &bad, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
                }
            }
            let checked = run(proofs, &symbolic, ScalarCheckerScope::CurrentPackage).unwrap();
            let [failure] = checked.directives.as_slice() else { panic!("one failure after both summary imports"); };
            let (four, seven) = (BigInt::from(4), BigInt::from(7));
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&four, &four)]));
            assert!(failure.witness.intervals_at(0, 2).unwrap().eq([(&seven, &seven)]));
        }
        let wrong_a = rank_a.replace("ensures (out == 4)", "ensures (out == 5)");
        let wrong_b = rank_b.replace("ensures (out == 7)", "ensures (out == 8)");
        let checked = run([&wrong_b, &wrong_a], "", ScalarCheckerScope::CurrentPackage).unwrap();
        assert_eq!(checked.symbolic_diagnostic.unwrap().returned, BigInt::from(4));
        let [later] = checked.additional_symbolic_diagnostics.as_slice() else { panic!("second failure"); };
        assert_eq!(later.returned, BigInt::from(7));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_verified_ranked_equality_refines_later_symbolic_calls() {
        let app = source("src/main.w", "ordered-ranked-symbolic-import");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid ranked symbolic import source"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        let ranked = r#"proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
            if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
        }"#;
        let bare = "proof bound for fixed(entry) -> (out) ensures (out == 7) {}";
        for mixed in [false, true] {
            let (fixed, call, proof, other_requirement) = if mixed {
                (
                    "u8(u8) fixed = fn(value) { if (value == 0) { 7 } else { 7 } };",
                    "u8 other = fixed(input);", bare, format!("core.require(other == 7, {OUTPUT});"),
                )
            } else {
                ("", "", "", String::new())
            };
            let text = format!(r#"%%start
                u8(u8) descend = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend(value - 3) }} }};
                {fixed}
                unit(u8) client = fn(input) {{
                    core.narrow(input <= 8, {OUTPUT});
                    u8 value = descend(input);
                    {call}
                    core.require(value == 4, {OUTPUT});
                    {other_requirement}
                    core.require(value == 5, {OUTPUT});
                }};
                {proof}
                {ranked}
            %%end"#);
            let checked = run(&text, ScalarCheckerScope::CurrentPackage).unwrap();
            assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
            let [failure] = checked.directives.as_slice() else { panic!("one later symbolic failure"); };
            assert_eq!(&text[failure.condition.start as usize..failure.condition.end as usize], "value == 5");
            let four = BigInt::from(4);
            assert!(failure.witness.intervals_at(0, 1).unwrap().eq([(&four, &four)]));
            let main = run(&text, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!main.active && main.directives.is_empty());
            for rejected in [
                text.replace("core.narrow(input <= 8", "core.narrow(input <= 9"),
                text.replace("ensures (out == 4)", "ensures (out == 5)"),
                text.replace("entry <= 8", "false"),
                text.replace("%%end", "u8 reached = descend(8); %%end"),
            ] {
                assert!(matches!(run(&rejected, ScalarCheckerScope::CurrentPackage),
                    Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_ranked_summary_preserves_exact_conjunctive_and_disjoint_guards() {
        let app = source("src/main.w", "ordered-ranked-domain-import");
        let text = format!(r#"%%start
            u8(u8, u8) descend = fn(tag, value) {{ if (value < 3) {{ 7 }} else {{ descend(tag, value - 3) }} }};
            proof rank for descend(tag, entry) -> (out) where (tag >= 2 && (entry == 2 || entry == 5))
                ensures (out == 7) decreases (tag, entry) {{
                if (entry < 3) {{ core.show(out == 7); }} else {{ core.use(rank(tag, entry - 3)); }}
            }}
            unit(u8, u8) client = fn(tag, input) {{
                core.narrow(tag >= 2, {OUTPUT});
                core.narrow(input == 2 || input == 5, {OUTPUT});
                u8 result = descend(tag, input);
                core.require(result == 7, {OUTPUT});
                core.require(result == 8, {OUTPUT});
            }};
        %%end"#);
        let run = |text: &str| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid ranked contextual import"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(),
                ScalarCheckerScope::CurrentPackage, Vec::new(),
            )
        };
        let checked = run(&text).unwrap();
        assert!(checked.active && checked.diagnostics.is_empty() && checked.symbolic_diagnostic.is_none());
        let [failure] = checked.directives.as_slice() else { panic!("one rejected symbolic requirement"); };
        assert_eq!(&text[failure.condition.start as usize..failure.condition.end as usize], "result == 8");
        let seven = BigInt::from(7);
        assert!(failure.witness.intervals_at(0, 2).unwrap().eq([(&seven, &seven)]));
        let literal = text.replace("descend(tag, input);", "descend(tag, 5);");
        assert!(run(&literal).is_ok());
        for rejected in [
            text.replace("core.narrow(tag >= 2", "core.narrow(tag >= 1"),
            text.replace("input == 2 || input == 5", "input == 2 || input == 4"),
            literal.replace("descend(tag, 5);", "descend(tag, 4);"),
        ] {
            assert!(matches!(run(&rejected), Err(ScalarOrderedCheckerOutcome::Unsupported)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_mixed_ranked_and_bare_reuses_only_verified_bare_equality() {
        let app = source("src/main.w", "ordered-mixed-summary");
        let ranked = r#"proof rank for descend(entry) -> (out) where (entry <= 8) ensures (out == 4) decreases (entry) {
            if (entry < 3) { core.show(out == 4); } else { core.use(rank(entry - 3)); }
        }"#;
        let bare = "proof bound for producer(entry) -> (out) where (entry >= 0) ensures (out == 7) {}";
        let run = |text: String, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text, &[])
                .unwrap_or_else(|_| panic!("valid mixed summary source"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            )
        };
        for (first, second) in [(ranked, bare), (bare, ranked)] {
            let text = format!(r#"%%start
                u8(u8) descend = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend(value - 3) }} }};
                i16(i16) producer = fn(input) {{ if (input < 0) {{ input + 1 }} else {{ 7 }} }};
                unit(i16) client = fn(input) {{
                    core.narrow(input >= 0, {OUTPUT});
                    i16 result = producer(input);
                    core.require(result == 7, {OUTPUT});
                    core.require(result == 8, {OUTPUT});
                }};
                {first}
                {second}
                u8 recursed = descend(8);
                core.require(recursed == 4, {OUTPUT});
                core.require(false, {OUTPUT});
            %%end"#);
            let receipt = run(text.clone(), ScalarCheckerScope::CurrentPackage).unwrap();
            assert!(receipt.active && receipt.diagnostics.is_empty());
            assert!(receipt.symbolic_diagnostic.is_none() && receipt.additional_symbolic_diagnostics.is_empty());
            let [actual, symbolic] = receipt.directives.as_slice() else {
                panic!("one actual directive and one failed symbolic requirement");
            };
            let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
            assert_eq!(snippet(actual.condition), "false");
            assert_eq!(snippet(symbolic.condition), "result == 8");
            let seven = BigInt::from(7);
            assert!(symbolic.witness.intervals_at(0, 1).unwrap().eq([(&seven, &seven)]));

            let main = run(text.clone(), ScalarCheckerScope::MainFlowOnly).unwrap();
            let [actual] = main.directives.as_slice() else { panic!("only reached directive"); };
            assert_eq!(snippet(actual.condition), "false");
            for invalid in [
                text.replace("else { 7 }", "else { 8 }"),
                text.replace("core.narrow(input >= 0", "core.narrow(input >= -1"),
                text.replace("u8 recursed = descend(8);", "i16 used = producer(1); u8 recursed = descend(8);"),
            ] {
                assert!(matches!(run(invalid, ScalarCheckerScope::CurrentPackage),
                    Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_guarded_unused_universal_preserves_actual_calls_and_exact_scope() {
        let app = source("src/main.w", "ordered-unused-universal");
        let run = |upper, scope| {
            let text = format!(r#"%%start
                u8(u8) active_band = fn(value) {{ value }};
                u8(u8) unused_band = fn(value) {{ value }};
                proof actual for active_band(entry) -> (out) ensures (out < 8) {{}}
                proof unused for unused_band(entry) -> (out) where (entry <= {upper})
                    forall (u8 ceiling where ceiling >= 6 && ceiling != 6) ensures (out < ceiling) {{}}
                u8 reached = active_band(8);
            %%end"#);
            let (_, proof) = crate::parse_proof_source(app.clone(), text, &[])
                .unwrap_or_else(|_| panic!("valid guarded universal proof"));
            check_loaded_scalar_ordered_local_proof_contexts(
                Vec::new(), [(proof, Vec::new())], vec![app.clone()], inputs(), scope, Vec::new(),
            ).unwrap()
        };
        let full = run(7, ScalarCheckerScope::CurrentPackage);
        let main = run(7, ScalarCheckerScope::MainFlowOnly);
        let tight = run(6, ScalarCheckerScope::CurrentPackage);
        for receipt in [&full, &main, &tight] {
            assert!(receipt.active && receipt.directives.is_empty());
            let [reached] = receipt.diagnostics.as_slice() else { panic!("one actual proof failure"); };
            assert_eq!(reached.proof.source, app);
            assert_eq!(reached.invocation.returned, BigInt::from(8));
            assert!(receipt.additional_symbolic_diagnostics.is_empty());
        }
        let symbolic = full.symbolic_diagnostic.unwrap();
        let seven = BigInt::from(7);
        assert_eq!(symbolic.proof.source, app);
        assert_eq!(symbolic.returned, seven);
        assert_eq!(symbolic.witness.intervals_at(0, 0).unwrap().collect::<Vec<_>>(), [(&seven, &seven)]);
        assert!(main.symbolic_diagnostic.is_none() && tight.symbolic_diagnostic.is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proofs_share_unused_returns_in_declaration_order() {
        let app = source("src/main.w", "ordered-automatic-proof-unused-group");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = format!(r#"%%start
            u8(u8) checked = fn(value) {{
                core.narrow(value != 2, {OUTPUT});
                core.require(value < 2, {OUTPUT});
                value
            }};
            proof zeta for checked(entry) -> (out) ensures (out != 1) {{}}
            proof gap for checked(entry) -> (out) where (true) ensures (out != 2) {{}}
            proof alpha for checked(entry) -> (out) where (true && true) ensures (out != 3) {{}}
        %%end"#);
        let report = run(&text, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(report.active && report.directives.len() == 1 && report.diagnostics.is_empty());
        let first = report.symbolic_diagnostic.unwrap();
        let [second] = report.additional_symbolic_diagnostics.as_slice() else { panic!("second failed goal"); };
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        for (diagnostic, name, point) in [(&first, "proof zeta", 1), (second, "proof alpha", 3)] {
            assert!(snippet(diagnostic.proof.range).starts_with(name));
            assert_eq!(diagnostic.directive_position, 1);
            assert_eq!(diagnostic.returned, BigInt::from(point));
            let intervals = diagnostic.witness.intervals_at(0, 0).unwrap()
                .map(|(first, last)| (first.clone(), last.clone())).collect::<Vec<_>>();
            assert_eq!(intervals, [(BigInt::from(point), BigInt::from(point))]);
        }
        let actual_only = run(&text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(!actual_only.active && actual_only.directives.is_empty() && actual_only.diagnostics.is_empty());
        assert!(actual_only.symbolic_diagnostic.is_none() && actual_only.additional_symbolic_diagnostics.is_empty());
        let mismatched = text.replace("where (true && true)", "where (entry != 0)");
        assert!(matches!(run(&mismatched, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
        let no_returns = text.replace("core.narrow(value != 2,", "core.narrow(false,");
        let no_returns = run(&no_returns, ScalarCheckerScope::CurrentPackage).unwrap();
        assert_eq!(no_returns.directives.len(), 1);
        assert_eq!(no_returns.directives[0].operation, ScalarCheckerOperation::Narrow);
        assert!(no_returns.symbolic_diagnostic.is_none() && no_returns.additional_symbolic_diagnostics.is_empty());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_current_package_auto_proofs_follow_canonical_unused_roots_after_actual_calls() {
        let app = source("src/main.w", "ordered-automatic-proof-mixed-roots");
        let run = |text: &str, scope| {
            let (_, proof) = crate::parse_proof_source(app.clone(), text.to_owned(), &[])
                .unwrap_or_else(|_| panic!("valid proof source"));
            check_loaded_ordered_proof_diagnostics(
                Vec::new(), vec![(proof, Vec::new())], vec![app.clone()], inputs(), scope,
            )
        };
        let text = format!(r#"%%start
            u8(u8) early = fn(value) {{ core.require(value != 3, {OUTPUT}); value }};
            u8(u8) directives = fn(value) {{ core.require(value != 5, {OUTPUT}); value }};
            u8(u8) later = fn(value) {{ value }};
            u8(u8) discovered = fn(value) {{ value }};
            u8(u8) unrelated = fn(value) {{ value + 1 }};
            proof later_point for later(entry) -> (out) where (entry == 8) ensures (out != 8) {{}}
            proof actual_pass for discovered(entry) -> (out) where (entry > 0) ensures (out <= 1) {{}}
            proof early_zeta for early(entry) -> (out) ensures (out != 3) {{}}
            proof actual_fail for discovered(entry) -> (out) where (entry <= 1) ensures (out != 1) {{}}
            proof early_alpha for early(entry) -> (out) ensures (out != 6) {{}}
            u8 observed = discovered(1);
        %%end"#);
        let report = run(&text, ScalarCheckerScope::CurrentPackage).unwrap();
        assert!(report.active);
        let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
        let [actual] = report.diagnostics.as_slice() else { panic!("one actual proof failure"); };
        assert!(snippet(actual.proof.range).starts_with("proof actual_fail"));
        assert_eq!(snippet(actual.invocation.call.range), "discovered(1)");
        assert_eq!(actual.invocation.directive_position, 0);
        assert_eq!(report.directives.iter().map(|effect| snippet(effect.condition)).collect::<Vec<_>>(), ["value != 3", "value != 5"]);
        let first = report.symbolic_diagnostic.unwrap();
        let [second, third] = report.additional_symbolic_diagnostics.as_slice() else { panic!("two further symbolic failures"); };
        for (diagnostic, proof, target, point, after) in [
            (&first, "proof early_zeta", "early", 3, 1),
            (second, "proof early_alpha", "early", 6, 1),
            (third, "proof later_point", "later", 8, 2),
        ] {
            assert!(snippet(diagnostic.proof.range).starts_with(proof));
            assert_eq!(snippet(diagnostic.callable.range), target);
            assert_eq!(diagnostic.returned, BigInt::from(point));
            assert_eq!(diagnostic.directive_position, after);
        }
        let main = run(&text, ScalarCheckerScope::MainFlowOnly).unwrap();
        assert!(main.active && main.diagnostics.len() == 1 && main.directives.is_empty());
        assert!(main.symbolic_diagnostic.is_none() && main.additional_symbolic_diagnostics.is_empty());
        let without_proofs = text.lines().map(|line| {
            if line.trim_start().starts_with("proof ") { " ".repeat(line.len()) } else { line.to_owned() }
        }).collect::<Vec<_>>().join("\n");
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let no_catalog = check_loaded_ordered_proof_diagnostics(
                vec![(declarations(&app, &without_proofs), Vec::new())], Vec::new(), vec![app.clone()], inputs(), scope,
            ).unwrap();
            let empty_catalog = run(&without_proofs, scope).unwrap();
            let legacy = check_scalar_package_ordered_with_dormant_sources(
                vec![(declarations(&app, &without_proofs), Vec::new())],
                vec![app.clone()], Vec::new(), inputs(), scope,
            );
            assert_eq!(no_catalog.directives, empty_catalog.directives);
            assert!(no_catalog.diagnostics.is_empty() && no_catalog.symbolic_diagnostic.is_none());
            assert_eq!(no_catalog.active, empty_catalog.active);
            match scope {
                ScalarCheckerScope::MainFlowOnly => assert!(matches!(legacy, ScalarOrderedCheckerOutcome::NoChecks)),
                ScalarCheckerScope::CurrentPackage => {
                    assert_eq!(legacy, ScalarOrderedCheckerOutcome::Failed(no_catalog.directives));
                }
            }
        }
        let not_reached = text.replace("u8 observed = discovered(1);", "");
        assert!(matches!(run(&not_reached, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
        let incomplete = text.replace("later = fn(value) { value }", "later = fn(value) { value + 1 }");
        assert!(matches!(run(&incomplete, ScalarCheckerScope::CurrentPackage), Err(ProjectCheck::Unsupported)));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_terminal_disjunction_keeps_the_surviving_return_arm() {
        let app = source("src/main.w", "terminal-branch-disjunction");
        let prepare = |actual: &str| {
            let text = format!(r#"%%start
                u8(u8) checked = fn(x) {{
                    core.narrow(x != 4, {OUTPUT});
                    if (x < 2 || x > 6) {{
                        core.require(x != 7, {OUTPUT});
                        core.narrow(false, {OUTPUT});
                        core.require(false, {OUTPUT});
                        0
                    }} else {{
                        core.require(x != 3, {OUTPUT});
                        core.require(x != 4, {OUTPUT});
                        x
                    }}
                }};
                {actual}
            %%end"#);
            let project = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, project)
        };
        let (text, prepared) = prepare("");
        let (active, failures) = collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        assert!(active);
        let records = failures.into_iter().map(|failure| {
            let ProjectCheck::Failed { operation, condition, state, .. } = failure else { panic!("expected callable failure"); };
            let intervals = state.intervals_at(0, 0).unwrap()
                .map(|(first, last)| (first.clone(), last.clone())).collect::<Vec<_>>();
            (operation, text[condition.start as usize..condition.end as usize].to_owned(), intervals)
        }).collect::<Vec<_>>();
        let expected = |operation, condition: &str, ranges: &[(u8, u8)]| (operation, condition.to_owned(), ranges.iter()
            .map(|&(first, last)| (BigInt::from(first), BigInt::from(last))).collect::<Vec<_>>());
        assert_eq!(records, [
            expected(CheckerOperation::Require, "x != 7", &[(7, 7)]),
            expected(CheckerOperation::Narrow, "false", &[(0, 1), (7, 255)]),
            expected(CheckerOperation::Require, "x != 3", &[(3, 3)]),
        ]);
        assert_eq!(check_package_roots(&prepared), Err(ProjectCheck::Unsupported));
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 1), Ok((false, Vec::new())));
        let (_, actual) = prepare("u8 used = checked(7);");
        assert_eq!(collect_ordered_package_failures(&actual, ScalarCheckerScope::MainFlowOnly, 1), Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_projection_preserves_disjoint_axis_and_caller_correlations() {
        let app = source("src/main.w", "collect-projected-symbolic-roots");
        let prepare = |leaf_body: &str, caller_prune: &str| {
            let text = format!(r#"%%start
                unit(u8) leaf = fn(value) {{ {leaf_body} }};
                unit(u8, u8) unused = fn(x, y) {{
                    if (x < 4) {{
                        core.narrow(x <= 2, {OUTPUT});
                        core.narrow(y == 1, {OUTPUT});
                    }} else {{
                        core.narrow(x >= 6, {OUTPUT});
                        core.narrow(y == 2, {OUTPUT});
                    }};
                    {caller_prune}
                    leaf(x);
                    if (y == 1) {{
                        core.require(x == 0, {OUTPUT});
                    }} else {{
                        core.require(x < 7, {OUTPUT});
                    }};
                }};
                leaf(1);
            %%end"#);
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, prepared)
        };
        let read_reports = |text: &str, prepared: &PreparedCheckerProject| {
            let (active, reports) = collect_ordered_package_failures(prepared, ScalarCheckerScope::CurrentPackage, 1)
                .expect("checked disjoint projection");
            assert!(active);
            reports.iter().map(|report| {
                let ProjectCheck::Failed { condition, state, axes, .. } = report else { panic!("expected failure"); };
                assert_eq!(state.row_count(), 1);
                let domains = axes.iter().enumerate().map(|(axis, (binding, ty))| {
                    assert_eq!(*ty, ScalarType::U8);
                    let at = binding.declaration_span;
                    let domain = state.intervals_at(0, axis).unwrap()
                        .map(|(lo, hi)| (lo.clone(), hi.clone())).collect::<Vec<_>>();
                    (text[at.start as usize..at.end as usize].to_owned(), domain)
                }).collect::<Vec<_>>();
                (text[condition.start as usize..condition.end as usize].to_owned(), domains)
            }).collect::<Vec<_>>()
        };
        let limits = |intervals: &[(i32, i32)]| intervals.iter().map(|(lo, hi)| (BigInt::from(*lo), BigInt::from(*hi))).collect::<Vec<_>>();
        let plain = format!("core.require(value < 2, {OUTPUT});");
        let (text, prepared) = prepare(&plain, "");
        assert_eq!(read_reports(&text, &prepared), [
            ("value < 2".into(), vec![("value".into(), limits(&[(2, 2), (6, 255)]))]),
            ("x == 0".into(), vec![("x".into(), limits(&[(1, 2)])), ("y".into(), limits(&[(1, 1)]))]),
            ("x < 7".into(), vec![("x".into(), limits(&[(7, 255)])), ("y".into(), limits(&[(2, 2)]))]),
        ]);

        let partial = format!("{plain} core.narrow(value != 2, {OUTPUT}); core.narrow(value != 8, {OUTPUT}); core.narrow(value < 12, {OUTPUT});");
        let (text, prepared) = prepare(&partial, "");
        assert_eq!(read_reports(&text, &prepared), [
            ("value < 2".into(), vec![("value".into(), limits(&[(2, 2), (6, 255)]))]),
            ("x == 0".into(), vec![("x".into(), limits(&[(1, 1)])), ("y".into(), limits(&[(1, 1)]))]),
            ("x < 7".into(), vec![("x".into(), limits(&[(7, 7), (9, 11)])), ("y".into(), limits(&[(2, 2)]))]),
        ]);

        let all_empty = format!("{plain} core.narrow(value == 1, {OUTPUT});");
        let (text, prepared) = prepare(&all_empty, &format!("core.narrow(x != 1, {OUTPUT});"));
        assert_eq!(read_reports(&text, &prepared), [
            ("value < 2".into(), vec![("value".into(), limits(&[(2, 2), (6, 255)]))]),
            ("value == 1".into(), vec![("value".into(), limits(&[(0, 0), (2, 2), (6, 255)]))]),
        ]);

        let (_, prepared) = prepare(&format!("{plain} value = 1;"), "");
        use super::super::checker_flow::PROJECTED_ARGUMENT_MATERIALIZATIONS;
        let before = PROJECTED_ARGUMENT_MATERIALIZATIONS.get();
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 1),
            Err(ProjectCheck::Unsupported));
        assert_eq!(PROJECTED_ARGUMENT_MATERIALIZATIONS.get(), before);
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 1),
            Ok((true, Vec::new())));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_package_symbolic_projection_pairs_one_exact_axis_with_one_disjoint_axis() {
        let app = source("src/main.w", "collect-one-projected-pair-axis");
        let prepare = |body: String| {
            let text = format!(r#"%%start
                unit(u8, i8) leaf = fn(left, right) {{
                    core.require(left < 5, {OUTPUT});
                    core.require(right < 0, {OUTPUT});
                    core.narrow(left != 6, {OUTPUT});
                    core.narrow(right != 2, {OUTPUT});
                }};
                unit(u8, i8) wrapper = fn(x, y) {{ {body} }};
                leaf(1, -1);
            %%end"#);
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, prepared)
        };
        let read = |text: &str, prepared: &PreparedCheckerProject| {
            let (active, reports) = collect_ordered_package_failures(prepared, ScalarCheckerScope::CurrentPackage, 1)
                .expect("complete typed two-axis reports");
            assert!(active);
            reports.iter().map(|report| {
                let ProjectCheck::Failed { condition, state, axes, .. } = report else { panic!("expected report"); };
                assert_eq!(state.row_count(), 1);
                assert_eq!(axes.len(), 2);
                assert_eq!((&axes[0].1, &axes[1].1), (&ScalarType::U8, &ScalarType::I8));
                let domains = [0, 1].map(|axis| state.intervals_at(0, axis).unwrap()
                    .map(|(lo, hi)| (lo.clone(), hi.clone())).collect::<Vec<_>>());
                (text[condition.start as usize..condition.end as usize].to_owned(), domains)
            }).collect::<Vec<_>>()
        };
        let limits = |pairs: &[(i32, i32)]| pairs.iter()
            .map(|(lo, hi)| (BigInt::from(*lo), BigInt::from(*hi))).collect::<Vec<_>>();
        use super::super::checker_flow::PROJECTED_ARGUMENT_MATERIALIZATIONS;
        let before = PROJECTED_ARGUMENT_MATERIALIZATIONS.get();
        let (text, prepared) = prepare(format!(r#"
            if (x < 4) {{ core.narrow(y == -5, {OUTPUT}); }} else {{ core.narrow(y == 0, {OUTPUT}); }};
            leaf(x, -1);
            if (y == 0) {{ core.require(x < 5, {OUTPUT}); }};
        "#));
        assert_eq!(read(&text, &prepared), [
            ("left < 5".into(), [limits(&[(5, 255)]), limits(&[(-1, -1)])]),
            ("x < 5".into(), [limits(&[(5, 5), (7, 255)]), limits(&[(0, 0)])]),
        ]);

        assert_eq!(PROJECTED_ARGUMENT_MATERIALIZATIONS.get(), before + 1);
        let (text, prepared) = prepare(format!(r#"
            if (y < 0) {{ core.narrow(x == 1, {OUTPUT}); }} else {{ core.narrow(x == 2, {OUTPUT}); }};
            leaf(3, y);
            if (x == 2) {{ core.require(y < 1, {OUTPUT}); }};
        "#));
        assert_eq!(read(&text, &prepared), [
            ("right < 0".into(), [limits(&[(3, 3)]), limits(&[(0, 127)])]),
            ("y < 1".into(), [limits(&[(2, 2)]), limits(&[(1, 1), (3, 127)])]),
        ]);

        let (text, prepared) = prepare(format!(r#"
            core.narrow(x == 3, {OUTPUT});
            leaf(x, y);
            core.require(y < 1, {OUTPUT});
        "#));
        assert_eq!(read(&text, &prepared), [
            ("right < 0".into(), [limits(&[(3, 3)]), limits(&[(0, 127)])]),
            ("y < 1".into(), [limits(&[(3, 3)]), limits(&[(1, 1), (3, 127)])]),
        ]);

        let (_, prepared) = prepare("leaf(x, y);".into());
        let before = PROJECTED_ARGUMENT_MATERIALIZATIONS.get();
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 1),
            Err(ProjectCheck::Unsupported));
        assert_eq!(PROJECTED_ARGUMENT_MATERIALIZATIONS.get(), before);
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::MainFlowOnly, 1),
            Ok((true, Vec::new())));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn startup_failure_collection_preserves_selected_order_and_stops_an_empty_lane() {
        let app = source("src/main.w", "collect-startup-failures");
        let prepare = |text: &str| prepare_checker_project(
            vec![(declarations(&app, text), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let text = format!(r#"%%start
            bool enabled = true;
            bool disabled = false;
            core.require(false, {OUTPUT});
            core.narrow(enabled, {OUTPUT});
            core.require(disabled, {OUTPUT});
            if (false) {{ core.require(false, {OUTPUT}); }} else {{
                core.require(false, {OUTPUT});
                core.narrow(false, {OUTPUT});
                core.require(false, {OUTPUT});
            }};
            core.require(false, {OUTPUT});
        %%end"#);
        let prepared = prepare(&text);
        let failures = collect_package_startup_failures(&prepared).unwrap();
        let authored = failures.iter().map(|failure| {
            let ProjectCheck::Failed { source, operation, directive, condition, axes, state, .. } = failure else {
                panic!("expected an authored startup diagnostic");
            };
            assert_eq!(source, &app);
            assert!(axes.is_empty());
            assert_eq!(state.row_count(), 1);
            (*operation, directive.start as usize, &text[condition.start as usize..condition.end as usize])
        }).collect::<Vec<_>>();
        let offsets = text.match_indices("core.require(false").map(|(offset, _)| offset).collect::<Vec<_>>();
        assert_eq!(authored, [
            (CheckerOperation::Require, offsets[0], "false"),
            (CheckerOperation::Require, text.find("core.require(disabled").unwrap(), "disabled"),
            (CheckerOperation::Require, offsets[2], "false"),
            (CheckerOperation::Narrow, text.find("core.narrow(false").unwrap(), "false"),
        ]);
        assert_eq!(check_package_initialization(&prepared), Err(failures.into_iter().next().unwrap()));

        let incomplete = format!(r#"%%start
            unit() leaf = fn {{ leaf(); }};
            unit() helper = fn {{ core.require(false, {OUTPUT}); leaf(); }};
            core.require(false, {OUTPUT});
            helper();
        %%end"#);
        assert_eq!(collect_package_startup_failures(&prepare(&incomplete)), Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn startup_failure_collection_interleaves_exact_unit_calls_and_stops_after_empty_callee() {
        let app = source("src/main.w", "collect-exact-unit-failures");
        let text = format!(r#"%%start
            unit(u8) helper = fn(value) {{
                core.require(value < 2, {OUTPUT});
                core.narrow(value < 5, {OUTPUT});
                core.require(value < 4, {OUTPUT});
            }};
            core.require(false, {OUTPUT});
            helper(3);
            core.require(false, {OUTPUT});
            helper(4);
            if (true) {{
                helper(5);
                core.require(false, {OUTPUT});
            }};
            core.require(false, {OUTPUT});
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
        ).unwrap();
        let failures = collect_package_startup_failures(&prepared).unwrap();
        let rows = failures.iter().map(|failure| {
            let ProjectCheck::Failed { source, operation, directive, condition, axes, state, .. } = failure else {
                panic!("expected authored failure");
            };
            assert_eq!(source, &app);
            assert_eq!(state.row_count(), 1);
            let value = if axes.is_empty() {
                None
            } else {
                assert_eq!(axes.len(), 1);
                let limits = state.intervals_at(0, 0).unwrap().collect::<Vec<_>>();
                let [(first, last)] = limits.as_slice() else { panic!("expected exact callee state"); };
                assert_eq!(first, last);
                Some((*first).clone())
            };
            (*operation, directive.start as usize, &text[condition.start as usize..condition.end as usize], value)
        }).collect::<Vec<_>>();
        let top = text.match_indices("core.require(false").map(|(offset, _)| offset).collect::<Vec<_>>();
        let first_callee = text.find("core.require(value < 2").unwrap();
        let second_callee = text.find("core.require(value < 4").unwrap();
        let narrowed = text.find("core.narrow(value < 5").unwrap();
        assert_eq!(rows, [
            (CheckerOperation::Require, top[0], "false", None),
            (CheckerOperation::Require, first_callee, "value < 2", Some(3.into())),
            (CheckerOperation::Require, top[1], "false", None),
            (CheckerOperation::Require, first_callee, "value < 2", Some(4.into())),
            (CheckerOperation::Require, second_callee, "value < 4", Some(4.into())),
            (CheckerOperation::Require, first_callee, "value < 2", Some(5.into())),
            (CheckerOperation::Narrow, narrowed, "value < 5", Some(5.into())),
        ]);
        assert_eq!(check_package_initialization(&prepared), Err(failures.into_iter().next().unwrap()));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn startup_failure_collection_observes_exact_nested_calls_in_place_and_propagates_empty() {
        let app = source("src/main.w", "collect-nested-startup-calls");
        let prepare = |definitions: &str, startup: &str| {
            let text = format!("%%start\n{definitions}\n{startup}\n%%end");
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())], vec![app.clone()], inputs(),
            ).unwrap();
            (text, prepared)
        };
        let definitions = format!(r#"
            unit() mark = fn {{ core.require(false, {OUTPUT}); }};
            unit(u8) inner = fn(value) {{
                core.require(value < 2, {OUTPUT});
                core.narrow(value < 5, {OUTPUT});
            }};
            unit(u8) bridge = fn(value) {{
                core.require(value <= 3, {OUTPUT});
                inner(value);
                core.require(value != 4, {OUTPUT});
            }};
            unit(u8) relay = fn(value) {{ bridge(value); }};
            unit(u8) outer = fn(value) {{
                core.require(value < 3, {OUTPUT});
                mark();
                relay(3);
                relay(value);
                core.require(value < 4, {OUTPUT});
            }};
        "#);
        let (text, prepared) = prepare(&definitions, &format!("outer(4); outer(5); core.require(false, {OUTPUT});"));
        let failures = collect_package_startup_failures(&prepared).unwrap();
        let steps = failures.iter().map(|failure| {
            let ProjectCheck::Failed { condition, axes, state, operation, .. } = failure else { panic!("expected failure"); };
            let value = if axes.is_empty() { None } else {
                let mut intervals = state.intervals_at(0, 0).unwrap();
                let (lo, hi) = intervals.next().unwrap();
                assert_eq!(lo, hi);
                assert!(intervals.next().is_none());
                Some(lo.clone())
            };
            (&text[condition.start as usize..condition.end as usize], *operation, value)
        }).collect::<Vec<_>>();
        use CheckerOperation::{Narrow, Require};
        assert_eq!(steps, [
            ("value < 3", Require, Some(4.into())),
            ("false", Require, None),
            ("value < 2", Require, Some(3.into())),
            ("value <= 3", Require, Some(4.into())),
            ("value < 2", Require, Some(4.into())),
            ("value != 4", Require, Some(4.into())),
            ("value < 4", Require, Some(4.into())),
            ("value < 3", Require, Some(5.into())),
            ("false", Require, None),
            ("value < 2", Require, Some(3.into())),
            ("value <= 3", Require, Some(5.into())),
            ("value < 2", Require, Some(5.into())),
            ("value < 5", Narrow, Some(5.into())),
        ]);
        assert_eq!(check_package_initialization(&prepared), Err(failures.into_iter().next().unwrap()));

        let cycle = format!("unit() recurse = fn {{ core.require(false, {OUTPUT}); recurse(); }};");
        let (_, prepared) = prepare(&cycle, "recurse();");
        assert_eq!(collect_package_startup_failures(&prepared), Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn startup_failure_collection_preserves_two_checked_axes_across_exact_unit_calls() {
        let app = source("src/main.w", "collect-pair-root");
        let child = source("src/child.w", "collect-pair-child");
        let prepare = |startup: &str| {
            let app_text = format!(r#"%%start
                lib = namespace demo "src/child.w";
                unit(u8, i8) bridge = fn(x, y) {{
                    core.require(x < 3, {OUTPUT});
                    lib.leaf(x, y);
                    core.require(y > -8, {OUTPUT});
                }};
                unit(u8) literal = fn(x) {{ lib.leaf(x, -8); }};
                {startup}
            %%end"#);
            let child_text = format!(r#"%%start
                unit(u8, i8) leaf = fn(left, right) {{
                    core.require(left < 2, {OUTPUT});
                    core.require(right > -7, {OUTPUT});
                    core.narrow(right > -10, {OUTPUT});
                }};
            %%end"#);
            let start = app_text.find("lib =").unwrap();
            let end = start + app_text[start..].find(';').unwrap() + 1;
            let prepared = prepare_checker_project(
                vec![
                    (declarations(&app, &app_text), vec![ScalarNamespaceBinding {
                        binding: "lib".into(), target: child.clone(), span: ByteSpan::new(start as u32, end as u32),
                    }]),
                    (declarations(&child, &child_text), Vec::new()),
                ],
                vec![app.clone(), child.clone()],
                inputs(),
            ).unwrap();
            (prepared, app_text, child_text)
        };
        let (prepared, app_text, child_text) = prepare(&format!(r#"
            lib.leaf(4, -5);
            bridge(3, -8);
            literal(3);
            bridge(5, -10);
            core.require(false, {OUTPUT});
        "#));
        let reports = collect_package_startup_failures(&prepared).unwrap();
        let shape = reports.iter().map(|failure| {
            let ProjectCheck::Failed { source, operation, condition, axes, state, .. } = failure else { panic!("expected report"); };
            assert_eq!(state.row_count(), 1);
            assert_eq!(axes.len(), 2);
            assert_eq!((&axes[0].1, &axes[1].1), (&ScalarType::U8, &ScalarType::I8));
            assert!(axes.iter().all(|(binding, _)| &binding.source == source));
            let values = [0, 1].map(|axis| {
                let mut values = state.intervals_at(0, axis).unwrap();
                let (lo, hi) = values.next().unwrap();
                assert_eq!(lo, hi);
                assert!(values.next().is_none());
                lo.clone()
            });
            let text = if source == &app { &app_text } else {
                assert_eq!(source, &child);
                &child_text
            };
            (source, *operation, &text[condition.start as usize..condition.end as usize], values)
        }).collect::<Vec<_>>();
        let row = |x: i32, y: i32| [BigInt::from(x), BigInt::from(y)];
        use CheckerOperation::{Narrow, Require};
        assert_eq!(shape, [
            (&child, Require, "left < 2", row(4, -5)),
            (&app, Require, "x < 3", row(3, -8)),
            (&child, Require, "left < 2", row(3, -8)),
            (&child, Require, "right > -7", row(3, -8)),
            (&app, Require, "y > -8", row(3, -8)),
            (&child, Require, "left < 2", row(3, -8)),
            (&child, Require, "right > -7", row(3, -8)),
            (&app, Require, "x < 3", row(5, -10)),
            (&child, Require, "left < 2", row(5, -10)),
            (&child, Require, "right > -7", row(5, -10)),
            (&child, Narrow, "right > -10", row(5, -10)),
        ]);
        assert_eq!(check_package_initialization(&prepared), Err(ProjectCheck::Unsupported));

        let (nonuniform, _, _) = prepare("lib.leaf(1, -1);");
        assert_eq!(collect_ordered_package_failures(&nonuniform, ScalarCheckerScope::CurrentPackage, 2),
            Err(ProjectCheck::Unsupported));
        assert_eq!(collect_ordered_package_failures(&nonuniform, ScalarCheckerScope::MainFlowOnly, 2),
            Ok((true, Vec::new())));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn startup_failure_collection_uses_only_completed_same_package_namespace_calls() {
        let app = source("src/main.w", "collect-ready-package-root");
        let child = source("src/child.w", "collect-ready-package-child");
        let named_output = |code: u8| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let app_text = format!(r#"%%start
            core.require(false, {});
            lib = namespace demo "src/child.w";
            alias = namespace demo "src/child.w";
            unit(u8) outer = fn(value) {{
                lib.inner(value);
                alias.inner(9);
            }};
            lib.inner(7);
            outer(8);
            core.require(false, {});
        %%end"#, named_output(11), named_output(44));
        let child_text = format!(r#"%%start
            core.require(false, {});
            unit(u8) inner = fn(value) {{ core.require(value < 7, {}); }};
        %%end"#, named_output(22), named_output(33));
        let namespace = |name: &str| {
            let start = app_text.find(&format!("{name} =")).unwrap();
            let end = start + app_text[start..].find(';').unwrap() + 1;
            ScalarNamespaceBinding {
                binding: name.into(), target: child.clone(), span: ByteSpan::new(start as u32, end as u32),
            }
        };
        let prepared = prepare_checker_project(
            vec![
                (declarations(&app, &app_text), vec![namespace("lib"), namespace("alias")]),
                (declarations(&child, &child_text), Vec::new()),
            ],
            vec![app.clone(), child.clone()],
            inputs(),
        ).unwrap();
        let failures = collect_package_startup_failures(&prepared).unwrap();
        let records = failures.iter().map(|failure| {
            let ProjectCheck::Failed { source, output, axes, state, .. } = failure else { panic!("expected authored report"); };
            let value = if axes.is_empty() { None } else {
                assert_eq!(axes[0].0.source, *source);
                let mut values = state.intervals_at(0, 0).unwrap();
                let (lo, hi) = values.next().unwrap();
                assert_eq!(lo, hi);
                assert!(values.next().is_none());
                Some(lo.clone())
            };
            (source, output.code_value.clone(), value)
        }).collect::<Vec<_>>();
        assert_eq!(records, [
            (&app, 11.into(), None),
            (&child, 22.into(), None),
            (&child, 33.into(), Some(7.into())),
            (&child, 33.into(), Some(8.into())),
            (&child, 33.into(), Some(9.into())),
            (&app, 44.into(), None),
        ]);

        let (project, calls) = prepared.parts();
        let (_, ordinals) = package_initialization_order(project).unwrap();
        let (root, target) = (ordinals[&app], ordinals[&child]);
        let ModuleInitializationDetail::Occurrences(topology) = &calls.initialization_region(root).unwrap().detail else {
            panic!("expected checked initialization");
        };
        let at = app_text.find("outer(8)").unwrap() as u32;
        let outer_call = topology.structure.blocks[topology.structure.root.0].items.iter().find_map(|item| {
            let BlockItemKind::Evaluation { expression } = item.kind else { return None; };
            (topology.occurrences[expression.0].span.start == at).then_some(expression)
        }).expect("prepared outer call");
        for not_ready in [InitializationVisit::Unseen, InitializationVisit::Started] {
            let mut visits = vec![InitializationVisit::Unseen; project.modules.len()];
            visits[root] = InitializationVisit::Started;
            visits[target] = not_ready;
            let mut actual = CheckedCalls {
                project, calls, active: HashSet::new(), active_depth: 0, reached: HashSet::new(),
                had_obligations: false, observer: (), symbolic_module: 0,
            };
            let mut output = OrderedFailures::default();
            let result = check_initializer_expression::<true, false>(
                topology, &mut actual, &app, &HashMap::new(), outer_call, 0, &mut Some(&mut output), &visits,
            );
            assert!(matches!(result, Err(ProjectCheck::Unsupported)));
            assert!(output.records.is_empty());
            assert!(actual.active.is_empty());
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn startup_failure_collection_visits_namespaces_once_at_authored_dispatch() {
        let app = source("src/main.w", "collect-startup-root");
        let child = source("src/child.w", "collect-startup-child");
        let root_text = format!(r#"%%start
            core.require(false, {OUTPUT});
            child = namespace demo "src/child.w";
            alias = namespace demo "src/child.w";
            core.require(false, {OUTPUT});
        %%end"#);
        let namespace = |name: &str| {
            let start = root_text.find(&format!("{name} =")).unwrap();
            let end = start + root_text[start..].find(';').unwrap() + 1;
            ScalarNamespaceBinding {
                binding: name.into(), target: child.clone(), span: ByteSpan::new(start as u32, end as u32),
            }
        };
        let collect = |operation: &str| {
            let child_text = format!("%%start\ncore.{operation}(false, {OUTPUT});\ncore.require(false, {OUTPUT});\n%%end");
            let prepared = prepare_checker_project(
                vec![
                    (declarations(&app, &root_text), vec![namespace("child"), namespace("alias")]),
                    (declarations(&child, &child_text), Vec::new()),
                ],
                vec![app.clone(), child.clone()],
                inputs(),
            ).unwrap();
            collect_package_startup_failures(&prepared).unwrap().into_iter().map(|failure| {
                let ProjectCheck::Failed { source, operation, .. } = failure else { panic!("expected failure"); };
                (source, operation)
            }).collect::<Vec<_>>()
        };
        assert_eq!(collect("require"), [
            (app.clone(), CheckerOperation::Require),
            (child.clone(), CheckerOperation::Require),
            (child.clone(), CheckerOperation::Require),
            (app.clone(), CheckerOperation::Require),
        ]);
        assert_eq!(collect("narrow"), [
            (app.clone(), CheckerOperation::Require),
            (child.clone(), CheckerOperation::Narrow),
        ]);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn callable_failure_collection_preserves_unproved_rows_and_narrow_reachability() {
        use super::super::checker_flow::collect_integer_requirement_failures;

        let app = source("src/main.w", "collect-callable-failures");
        let text = format!(r#"%%start
            unit(u8) flow = fn(value) {{
                if (value > 3) {{
                    core.require(value < 9, {OUTPUT});
                    core.require(value < 10, {OUTPUT});
                    core.narrow(value < 4, {OUTPUT});
                    core.require(false, {OUTPUT});
                }} else {{
                    core.require(value == 0, {OUTPUT});
                    core.narrow(value < 2, {OUTPUT});
                    core.require(value < 2, {OUTPUT});
                }};
                core.require(value < 2, {OUTPUT});
            }};
            unit() helper = fn {{}};
            unit(u8) incomplete = fn(value) {{
                core.require(value == 0, {OUTPUT});
                helper();
            }};
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &text), Vec::new())], vec![app], inputs(),
        ).unwrap();
        let (_, calls) = prepared.parts();
        let body = calls.function_body(0, 0, None).unwrap();
        let (active, failures) = collect_integer_requirement_failures(body).unwrap();
        assert!(active);
        let receipts = failures.iter().map(|failure| {
            let directive = &body.structure.checker_directives[&failure.directive];
            let condition = &text[directive.condition.start as usize..directive.condition.end as usize];
            let intervals = failure.state.intervals_at(0, 0).unwrap()
                .map(|(first, last)| (first.clone(), last.clone())).collect::<Vec<_>>();
            assert_eq!(failure.state.row_count(), 1);
            (directive.operation, condition, intervals)
        }).collect::<Vec<_>>();
        let bounds = |start, end| vec![(BigInt::from(start), BigInt::from(end))];
        assert_eq!(receipts, [
            (CheckerOperation::Require, "value < 9", bounds(9, 255)),
            (CheckerOperation::Require, "value < 10", bounds(10, 255)),
            (CheckerOperation::Narrow, "value < 4", bounds(4, 255)),
            (CheckerOperation::Require, "value == 0", bounds(1, 3)),
        ]);
        assert_eq!(check_integer_requirement_activity(body), Some(Err(failures.into_iter().next().unwrap())));
        assert!(collect_integer_requirement_failures(calls.function_body(0, 2, None).unwrap()).is_none());
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn callable_failure_collection_continues_other_value_returning_arms_after_failed_narrow() {
        use super::super::checker_flow::collect_integer_requirement_failures;

        let app = source("src/main.w", "collect-value-arm-failures");
        let text = format!(r#"%%start
            u8(u8) choose = fn(value) {{
                if (value >= 8) {{
                    core.narrow(value < 8, {OUTPUT});
                    core.require(false, {OUTPUT});
                    value
                }} else {{
                    core.require(value == 0, {OUTPUT});
                    value
                }}
            }};
        %%end"#);
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &text), Vec::new())], vec![app], inputs(),
        ).unwrap();
        let (_, calls) = prepared.parts();
        let body = calls.function_body(0, 0, None).unwrap();
        let (active, failures) = collect_integer_requirement_failures(body).unwrap();
        assert!(active);
        let [first, second] = failures.as_slice() else { panic!("expected exactly two reached failures"); };
        let condition = |failure: &IntegerCheckFailure<'_>| {
            let directive = &body.structure.checker_directives[&failure.directive];
            (directive.operation, &text[directive.condition.start as usize..directive.condition.end as usize])
        };
        assert_eq!(condition(first), (CheckerOperation::Narrow, "value < 8"));
        assert_eq!(condition(second), (CheckerOperation::Require, "value == 0"));
        assert_eq!(check_integer_requirement_activity(body), Some(Err(failures.into_iter().next().unwrap())));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn strict_induction_premise_preserves_width_reachability_and_callee_preconditions() {
        let check = |ty: &str, filter: &str, base: &str, use_name: &str| {
            let file = source("src/main.w", "strict-premise");
            let text = format!(r#"%%start
                {ty}({ty}) descend = fn(value) {{ value }};
                proof rank for descend(entry) -> (out) where ({filter}) ensures (out == 0) decreases (entry) {{
                    if ({base}) {{ core.show(out == 0); }} else {{ core.use({use_name}(entry - 1)); }}
                }}
            %%end"#);
            let prepared = PreparedProofModule::prepare(file, text, inputs()).unwrap();
            assert!(matches!(prepared.plan_module_proof(&prepared.source, "rank"), Err(ProjectCheck::Unsupported)));
            prepared.audit_induction_premise(&prepared.source, "rank")
        };
        assert_eq!(check("u8", "entry <= 7", "entry == 0", "rank"), Ok(InductionPremise::Established));
        assert_eq!(check("u8", "true", "entry < 0", "rank"), Ok(InductionPremise::NotStrict));
        assert_eq!(check("u8", "entry == 0", "entry == 0", "rank"), Ok(InductionPremise::Unreachable));
        assert_eq!(check("u8", "entry > 0", "entry == 0", "rank"), Ok(InductionPremise::CalleeWhereUnproved));
        assert_eq!(check("i8", "entry <= -1", "entry == -128", "rank"), Ok(InductionPremise::Established));
        assert_eq!(check("i8", "true", "entry == 0", "rank"), Ok(InductionPremise::NotStrict));
        assert_eq!(check("u8", "true", "entry == 0", "peer"), Err(ProjectCheck::Unsupported));
        assert!(matches!(check("u8", "unknown < 7", "entry == 0", "rank"), Err(ProjectCheck::Invalid(_))));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn induction_return_witness_binds_the_real_base_and_exact_recursive_tail() {
        let check = |function: &str, show: u8, goal: u8, guard: &str, filter: &str| {
            let file = source("src/main.w", "checked-induction-return");
            let text = format!(r#"%%start
                u8(u8) other = fn(value) {{ value }};
                u8(u8) descend = fn(value) {{ {function} }};
                proof rank for descend(entry) -> (out) where ({filter}) ensures (out == {goal}) decreases (entry) {{
                    if ({guard}) {{ core.show(out == {show}); }} else {{ core.use(rank(entry - 1)); }}
                }}
            %%end"#);
            let prepared = PreparedProofModule::prepare(file, text, inputs()).unwrap();
            assert!(matches!(prepared.plan_module_proof(&prepared.source, "rank"), Err(ProjectCheck::Unsupported)));
            prepared.audit_induction_return_witness(&prepared.source, "rank")
        };
        let canonical = "if (value == 0) { 0 } else { descend(value - 1) }";
        assert_eq!(check(canonical, 0, 0, "entry == 0", "true"), Ok(InductionPremise::Established));
        assert_eq!(check(canonical, 1, 0, "entry == 0", "true"), Err(ProjectCheck::Refuted));
        assert_eq!(check(canonical, 0, 1, "entry == 0", "true"), Err(ProjectCheck::Refuted));
        assert_eq!(check(canonical, 0, 0, "entry == 1", "true"), Err(ProjectCheck::Unsupported));
        assert_eq!(check("if (value == 0) { 0 } else { descend(value - 2) }", 0, 0, "entry == 0", "true"), Err(ProjectCheck::Unsupported));
        assert_eq!(check("if (value == 0) { 0 } else { other(value - 1) }", 0, 0, "entry == 0", "true"), Err(ProjectCheck::Unsupported));
        assert_eq!(check("if (value == 0) { 0 } else { descend(value - 1) + 1 }", 0, 0, "entry == 0", "true"), Err(ProjectCheck::Unsupported));
        assert_eq!(check(canonical, 1, 0, "entry == 0", "false"), Ok(InductionPremise::Unreachable));
        assert_eq!(check("if (value == 0) { 0 } else { other(value - 1) }", 1, 0, "entry == 0", "true"), Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ranked_induction_refutation_keeps_exact_guarded_base_and_failed_source_step() {
        let file = source("src/main.w", "checked-induction-refutation");
        let check = |show, goal, filter: &str, expected_step: Option<&str>| {
            let text = format!(r#"%%start
                u8(u8) descend = fn(value) {{ if (value < 3) {{ 4 }} else {{ descend(value - 3) }} }};
                proof rank for descend(entry) -> (out) where ({filter}) ensures (out == {goal}) decreases (entry) {{
                    if (entry < 3) {{ core.show(out == {show}); }} else {{ core.use(rank(entry - 3)); }}
                }}
            %%end"#);
            let prepared = PreparedProofModule::prepare(file.clone(), text.clone(), inputs()).unwrap();
            let plan = prepared.plan_module_proof_in_scope::<true, false, false>(&prepared.source, "rank").unwrap();
            let rank = plan.typed.measure.unwrap();
            let outcome = plan.verify_induction_return_witness(rank).unwrap();
            let Some(expected_step) = expected_step else {
                assert!(matches!(outcome, RankedInductionOutcome::NoApplicable));
                return;
            };
            let RankedInductionOutcome::Refuted(witness) = outcome else {
                panic!("one-axis admitted base refutation");
            };
            assert_eq!(witness.entry_type, ScalarType::U8);
            assert_eq!(witness.entry, BigInt::from(2));
            assert_eq!(witness.returned, &BigInt::from(4));
            assert_eq!(witness.proof_source, &file);
            assert_eq!(witness.return_source, &file);
            let snippet = |span: ByteSpan| &text[span.start as usize..span.end as usize];
            assert_eq!(snippet(witness.return_span), "4");
            let actual_step = match witness.rejected {
                RankedBaseRejection::AuthoredShow(span) if show != 4 => snippet(span),
                RankedBaseRejection::Goal(span) if show == 4 => snippet(span),
                _ => panic!("wrong refutation source step"),
            };
            assert_eq!(actual_step, expected_step);
        };
        check(5, 4, "entry == 2", Some("core.show(out == 5)"));
        check(4, 5, "entry == 2", Some("out == 5"));
        check(5, 5, "entry == 2", Some("core.show(out == 5)"));
        check(5, 5, "false", None);
    }

    #[test]
    fn proof_decreases_resolves_the_declared_entry_binding_and_checked_width() {
        let file = source("src/main.w", "typed-decrease");
        let text = r#"%%start
            u16(u8, u16) choose = fn(first, second) { if (first == 0) {}; second };
            proof ranked for choose(other, measure) -> (out) ensures (out <= 10) decreases (measure) {}
        %%end"#;
        let prepared = PreparedProofModule::prepare(file, text.into(), inputs()).unwrap();
        let proof = prepared.select_resolved(&prepared.source, "ranked").unwrap();
        let (module, item, _) = proof.function;
        let body = prepared.prepared.parts().1.function_body(module, item, None).unwrap();
        let (slot, binding) = typed_proof_decreases(proof.node, &["other".into(), "measure".into()], body)
            .unwrap().expect("can derive the typed measure independently of trusting it").descending;
        assert_eq!(slot.ordinal, 1);
        assert_eq!(slot.ty, ScalarType::U16);
        assert_eq!(binding, &body.boundary.checker_parameter_bindings.as_deref().unwrap()[1]);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn non_reusable_entry_goal_does_not_replay_actual_initialization() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "no-replay");
        let text = format!(r#"%%start
            u8(u8) producer = fn(value) {{ value }};
            u8(u8) live = fn(value) {{ value }};
            u8 actual = live(7);
            unit() caller = fn {{
                u8 result = producer(7);
                core.require(result <= 7, {OUTPUT});
            }};
            proof entry for producer(input) -> (out) where (input <= 7) ensures (out == core.old(input)) {{}}
            proof literal for producer(input) -> (out) where (input <= 7) ensures (out <= 7) {{}}
        %%end"#);
        let run = |name| {
            let (_, catalog) = crate::parse_proof_source(file.clone(), text.clone(), &[]).ok().unwrap();
            check_loaded_scalar_module_proof_call(
                (&file, name), (&file, "caller"), Vec::new(), (catalog, Vec::new()),
                vec![file.clone()], inputs(),
            )
        };
        let _ = take_integer_observations();
        assert_eq!(run("entry"), ScalarProofCallOutcome::Unavailable);
        assert_eq!(take_integer_observations(), (0, 0));
        assert_eq!(run("literal"), ScalarProofCallOutcome::Established);
        assert_eq!(take_integer_observations(), (1, 1));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn proof_show_alternatives_keep_flat_clause_offsets_after_normalization() {
        let file = source("src/main.w", "flat-show-alternatives");
        let text = r#"%%start
            i16(i16) echo = fn(value) { value };
            proof composed for echo(input) -> (out) ensures (out <= 8) {
                core.show((out <= 7 && out <= 7 && out >= 5) || out == -8 || (out < 3 && out >= 1));
            }
        %%end"#;
        let project = PreparedProofModule::prepare(file.clone(), text.into(), inputs()).unwrap();
        let plan = project.plan_module_proof(&file, "composed").unwrap();
        let recipe = proof_show_plan::<false, true>(
            &plan.steps, &plan.typed.input_names, &plan.typed.output, plan.typed.result_type, plan.typed.body, None,
        ).unwrap_or_else(|outcome| panic!("expected exact show recipe: {outcome:?}"));
        let IntegerShowProof::Output(IntegerShowOutput::Alternatives { atoms, clause_ends }) = recipe
            else { panic!("expected flat alternatives"); };
        assert_eq!(clause_ends, [2, 3, 5]);
        assert_eq!(atoms.into_iter().map(|(_, bound)| bound).collect::<Vec<_>>(), [5, 7, -8, 1, 3].map(BigInt::from));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn proof_show_conjunction_normalizes_same_bound_once_in_owned_plan() {
        let relation = |operator| IntegerRelation::from_operator(&operator).unwrap();
        let lower = BigInt::from(-8);
        let upper = BigInt::from(8);
        let mut atoms = vec![
            (relation(BinaryOperator::LessEqual), upper.clone()),
            (relation(BinaryOperator::GreaterEqual), lower.clone()),
            (relation(BinaryOperator::LessEqual), upper.clone()),
            (relation(BinaryOperator::Less), lower.clone()),
        ];
        normalize_show_conjunction(&mut atoms, 0);
        assert_eq!(atoms.len(), 2);
        assert_eq!(&atoms[0].1, &lower);
        assert!(!atoms[0].0.accepts(std::cmp::Ordering::Less));
        assert!(!atoms[0].0.accepts(std::cmp::Ordering::Equal));
        assert!(!atoms[0].0.accepts(std::cmp::Ordering::Greater));
        assert_eq!(atoms[1], (relation(BinaryOperator::LessEqual), upper));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn selected_proof_catalog_indexes_only_repeated_distinct_names_per_source() {
        let app = source("src/main.w", "selected-catalog-root");
        let lib = source("src/lib.w", "selected-catalog-dependency");
        let mut app_text = String::from(r#"%%start
            lib = namespace demo "src/lib.w";
            u8(u8) fixed = fn(value) { if (value == 0) {}; 7 };
            proof wrong for fixed(input) -> (out) ensures (out == 256) {}
            proof duplicate for fixed(input) -> (out) ensures (out == 7) {}
            proof duplicate for fixed(input) -> (out) ensures (out == 7) {}
        "#);
        for index in 0..16 {
            app_text.push_str(&format!("proof p{index} for fixed(input) -> (out) ensures (out == 7) {{}}\n"));
        }
        app_text.push_str("%%end");
        let start = app_text.find("lib =").unwrap();
        let end = start + app_text[start..].find(';').unwrap() + 1;
        let lib_text = r#"%%start
            u8(u8) fixed = fn(value) { if (value == 0) {}; 8 };
            proof p0 for fixed(input) -> (out) ensures (out == 8) {}
        %%end"#;
        let project = PreparedProofModule::prepare_project(
            app.clone(),
            vec![
                ScalarProofModuleInput { source: app.clone(), text: app_text,
                    namespaces: vec![ScalarNamespaceBinding {
                        binding: "lib".into(), target: lib.clone(), span: ByteSpan::new(start as u32, end as u32),
                    }],
                },
                ScalarProofModuleInput { source: lib.clone(), text: lib_text.into(), namespaces: Vec::new() },
            ],
            vec![app.clone(), lib.clone()],
            inputs(),
        ).unwrap();
        let take_indexes = || PROOF_CATALOG_INDEX_BUILDS.with(|count| count.replace(0));
        let _ = take_indexes();
        assert_eq!(project.check_module_proof_selections(&[(&app, "p0"), (&lib, "p0"), (&app, "p0"), (&lib, "p0")]),
            vec![ScalarModuleProofOutcome::Proved; 4]);
        assert_eq!(take_indexes(), 0);

        let names = ["p15", "missing", "p0", "wrong", "duplicate", "p1", "p2", "p3", "p4", "p5", "p15"];
        let mut expected = vec![ScalarModuleProofOutcome::Proved; names.len()];
        expected[1] = ScalarModuleProofOutcome::Unsupported;
        expected[3] = ScalarModuleProofOutcome::Invalid(Vec::new());
        expected[4] = ScalarModuleProofOutcome::Unsupported;
        assert_eq!(project.check_module_proofs(&names), expected);
        assert_eq!(take_indexes(), 1);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn local_peer_graph_plans_transitive_citations_once_and_walks_one_leaf_domain() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "local-peer-graph");
        let mut text = String::from(r#"%%start
            u8(u8) fixed = fn(value) { if (value == 0) {}; 7 };
            proof p0 for fixed(input) -> (out) where (input <= 7) ensures (out == 7) {}
            proof unrelated for fixed(input) -> (out) ensures (out == 256) {}
        "#);
        for index in 1..=32 {
            text.push_str(&format!(
                "proof p{index} for fixed(arg) -> (out) where (arg <= 7) ensures (out == 7) {{ core.use(p{}(arg)); }}\n",
                index - 1,
            ));
        }
        text.push_str("%%end");
        let inequality = text.replace("out == 7", "out < 8");
        for text in [text, inequality] {
            let _ = take_integer_observations();
            let _ = UNSELECTED_PEER_PLANS.with(|count| count.replace(0));
            let _ = PROOF_CATALOG_INDEX_BUILDS.with(|count| count.replace(0));
            assert_eq!(check_scalar_module_proofs(
                file.clone(), &["p0", "p16", "p32"],
                vec![ScalarProofModuleInput { source: file.clone(), text, namespaces: Vec::new() }],
                vec![file.clone()], inputs(),
            ).unwrap(), [ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved]);
            assert_eq!(UNSELECTED_PEER_PLANS.with(|count| count.replace(0)), 30);
            assert_eq!(PROOF_CATALOG_INDEX_BUILDS.with(|count| count.replace(0)), 1);
            assert_eq!(take_integer_observations(), (1, 0));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn authored_show_root_literal_arithmetic_is_exact_lazy_and_proof_local() {
        use super::super::checker_flow::AUTHORED_RETURN_CALCULATIONS;
        use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Refuted, Unsupported};
        let file = source("src/main.w", "authored-root-literal-arithmetic");
        let check_typed = |ty: &str, body: &str, where_guard: &str, ensures: &str, startup: &str, names: &[&str]| {
            check_scalar_module_proofs(
                file.clone(), names,
                vec![ScalarProofModuleInput {
                    source: file.clone(),
                    text: format!(r#"%%start
                        {ty}({ty}) number = fn(value) {{ if (value == 0) {{}}; {body} }};
                        {startup}
                        proof leaf for number(arg) -> (out) where ({where_guard}) ensures ({ensures}) {{
                            core.show(out <= 7);
                        }}
                        proof another for number(arg) -> (out) where ({where_guard}) ensures (out == 7) {{
                            core.show(out >= 7);
                        }}
                        proof cite for number(arg) -> (out) where ({where_guard}) ensures (out <= 7) {{
                            core.use(leaf(arg));
                        }}
                        proof plain for number(arg) -> (out) where ({where_guard}) ensures (out <= 7) {{}}
                    %%end"#),
                    namespaces: Vec::new(),
                }],
                vec![file.clone()], inputs(),
            ).unwrap()
        };
        let check = |body: &str, where_guard: &str, ensures: &str, startup: &str, names: &[&str]| {
            check_typed("i8", body, where_guard, ensures, startup, names)
        };
        let count = || AUTHORED_RETURN_CALCULATIONS.with(|count| count.replace(0));
        let _ = count();
        assert_eq!(check("3 + 4", "true", "out <= 7", "", &["another", "leaf", "cite", "plain"]),
            [Proved, Proved, Proved, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("9 - (4 - 2)", "true", "out <= 7", "", &["another", "leaf", "cite", "plain"]),
            [Proved, Proved, Proved, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("(127 + 1) - 127", "true", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("(-128 - 1) + 127", "true", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("(-128 + 1) + 127", "true", "out == 0", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check_typed("u8", "(255 - 254) + 6", "true", "out == 7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check_typed("u8", "(0 - 1) + 2", "true", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        let check_i32 = |body: &str, where_guard: &str, ensures: &str, startup: &str, names: &[&str]| {
            check_typed("i32", body, where_guard, ensures, startup, names)
        };
        assert_eq!(check_i32("-(3 + 4)", "true", "out == -7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check_i32("1 + -(2 * 4)", "true", "out == -7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check_i32("-(-(3 + 4))", "true", "out == 7", "", &["leaf", "cite"]), [Proved, Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check_i32("-(2147483647 + 1)", "true", "out == -2147483648", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check_i32("(-(-2147483648)) * 0", "true", "out == 0", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check_i32("-(value + 1)", "true", "out == 0", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check_i32("~(1 + 1)", "true", "out == -3", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check_i32("-(2147483647 + 1)", "false", "out == -2147483648", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check_i32("-(3 + 4)", "true", "out == -7", "i32 live = number(0);", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("-7", "true", "out == -7", "", &["leaf", "cite", "plain"]),
            [Proved, Proved, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("-128", "true", "out == -128", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("-128", "false", "out == -128", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("-7", "true", "out == -7", "i8 live = number(0);", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        let literal_arm = "if (value == 0) { -128 } else { -7 }";
        assert_eq!(check(literal_arm, "true", "out <= -7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 2);
        assert_eq!(check(literal_arm, "arg == 0", "out == -128", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        let negated_arm = "if (value == 0) { -(3 + 4) } else { 0 - 7 }";
        assert_eq!(check_i32(negated_arm, "true", "out == -7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 2);
        for body in ["value + 1", "1 + value", "value - -1"] {
            assert_eq!(check(body, "arg == 6", "out == 7", "", &["another", "leaf", "cite", "plain"]),
                [Proved, Proved, Proved, Unsupported]);
            assert_eq!(count(), 1);
        }
        assert_eq!(check("value - 1", "arg == 6", "out == 5", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("value + 1", "arg >= 5 && arg <= 6", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("if (value == 0) {} else {}; value + 1", "arg >= 0 && arg <= 1", "out <= 7", "", &["leaf"]),
            [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("value + 1", "arg == 127", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check_typed("u8", "value - 1", "arg == 0", "out == 0", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("1 - value", "arg == 6", "out == -5", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8 local = 6; local + 1", "arg == 6", "out == 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        let exact_arm = "if (value == 6) { value + 1 } else { 6 }";
        assert_eq!(check(exact_arm, "true", "out <= 7", "", &["leaf", "another"]), [Proved, Refuted]);
        assert_eq!(count(), 1);
        assert_eq!(check(exact_arm, "arg == 5", "out == 6", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 0);
        assert_eq!(check("value + 1", "false", "out == 7", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("value + 1", "arg == 6", "out == 7", "i8 live = number(6);", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        let deep = format!("7{}", " + 0".repeat(32));
        assert_eq!(check(&deep, "true", "out <= 7", "", &["another", "leaf"]), [Proved, Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("3 + (2 * 2)", "true", "out == 7", "", &["leaf", "cite"]), [Proved, Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("-32 * 4", "true", "out == -128", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("(-64 * 2) * 0", "true", "out == 0", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("(-65 * 2) * 0", "true", "out == 0", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check_typed("u8", "(17 * 15) - 255", "true", "out == 0", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check_typed("u8", "(16 * 16) * 0", "true", "out == 0", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("3 + (8 / 2)", "true", "out == 7", "", &["leaf", "cite"]), [Proved, Proved]);
        assert_eq!(count(), 1);
        for (body, ensures) in [
            ("-9 / 2", "out == -4"),
            ("9 / -2", "out == -4"),
            ("-9 / -2", "out == 4"),
            ("-9 % 2", "out == -1"),
            ("9 % -2", "out == 1"),
            ("-9 % -2", "out == -1"),
            ("(-128 / 2) % 9", "out == -1"),
        ] {
            assert_eq!(check(body, "true", ensures, "", &["leaf"]), [Proved]);
            assert_eq!(count(), 1);
        }
        assert_eq!(check_typed("u8", "(255 / 2) % 10", "true", "out == 7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        for body in ["(9 / (2 - 2)) * 0", "(9 % (2 - 2)) * 0", "(-128 / -1) * 0", "(-128 % -1) * 0"] {
            assert_eq!(check(body, "true", "out == 0", "", &["leaf"]), [Unsupported]);
            assert_eq!(count(), 1);
        }
        assert_eq!(check("(-128 % -1) * 0", "false", "out == 0", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("3 + (2 + value)", "true", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("(127 + 1) - 127", "false", "out <= 7", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("9 - (4 - 2)", "true", "out <= 7", "i8 live = number(3);", &["leaf", "cite"]),
            [Unsupported, Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("9 - 2", "true", "out <= 7", "", &["cite"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("9 - -2", "true", "out <= 7", "", &["leaf", "cite"]), [Refuted, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("127 + 1", "true", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("127 + 1", "false", "out <= 7", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("3 + 4", "true", "out <= 6", "", &["leaf", "cite"]), [Refuted, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("3 + 4", "true", "out <= 7", "i8 live = number(3);", &["leaf", "cite"]),
            [Unsupported, Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("if (value == 0) { 3 + 4 } else { 7 }", "true", "out <= 7", "", &["leaf"]),
            [Proved]);
        assert_eq!(count(), 1);
        let branches = "if (value == 0) { 3 + 4 } else { 9 - (4 - 1) }";
        assert_eq!(check(branches, "true", "out <= 7", "", &["leaf", "another", "cite", "plain"]),
            [Proved, Refuted, Proved, Unsupported]);
        assert_eq!(count(), 2);
        let bad = "if (value == 0) { 3 + 4 } else { (127 + 1) * 0 }";
        assert_eq!(check(bad, "arg == 0", "out == 7", "", &["leaf"]), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check(bad, "true", "out <= 7", "", &["leaf", "cite"]), [Unsupported, Unsupported]);
        assert_eq!(count(), 2);
        assert_eq!(check(bad, "false", "out <= 7", "", &["leaf"]), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check(branches, "true", "out <= 7", "i8 live = number(0);", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
        let nested = "if (value == 0) { if (value == 0) { 3 + 4 } else { 7 } } else { 7 }";
        assert_eq!(check(nested, "true", "out <= 7", "", &["leaf"]), [Unsupported]);
        assert_eq!(count(), 0);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn authored_show_two_formal_arithmetic_requires_one_exact_joint_entry() {
        use super::super::checker_flow::AUTHORED_RETURN_CALCULATIONS;
        use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Unsupported};
        let file = source("src/main.w", "authored-two-formal-arithmetic");
        let check = |ty: &str, body: &str, guard: &str, ensures: &str, startup: &str, names: &[&str]| {
            check_scalar_module_proofs(
                file.clone(), names,
                vec![ScalarProofModuleInput {
                    source: file.clone(),
                    text: format!(r#"%%start
                        {ty}({ty}, {ty}) number = fn(left, right) {{ {body} }};
                        {startup}
                        proof proof_one for number(a, b) -> (out) where ({guard}) ensures ({ensures}) {{
                            core.show(out <= 7);
                        }}
                        proof plain for number(a, b) -> (out) where ({guard}) ensures ({ensures}) {{}}
                    %%end"#),
                    namespaces: Vec::new(),
                }],
                vec![file.clone()], inputs(),
            ).unwrap()
        };
        let count = || AUTHORED_RETURN_CALCULATIONS.with(|count| count.replace(0));
        let _ = count();
        let both = ["proof_one", "plain"];
        let only = ["proof_one"];
        let guard = "a == 6 && b == 1";
        assert_eq!(check("i8", "left + right", guard, "out == 7", "", &both), [Proved, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", "left - right", guard, "out == 5", "", &only), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", "right - left", guard, "out == -5", "", &only), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", "left + right", "a == 6 && b <= 1", "out <= 7", "", &only), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8", "left + right", "a == 127 && b == 1", "out <= 7", "", &only), [Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("u8", "left - right", "a == 0 && b == 1", "out <= 7", "", &only), [Unsupported]);
        assert_eq!(count(), 1);
        let self_subtract = "if (right == 0) {}; left - left";
        assert_eq!(check("i8", self_subtract, "true", "out == 0", "", &both), [Proved, Unsupported]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", self_subtract, "a != 0", "out == 0", "", &only), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("u8", self_subtract, "true", "out == 0", "", &only), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", "if (right == 0) { left - left } else { 0 }", "true", "out == 0", "", &only), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", self_subtract, "false", "out == 0", "", &only), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8", self_subtract, guard, "out == 0", "i8 live = number(6, 1);", &only), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8", "if (right == 0) {}; left + left", guard, "out == 12", "", &only), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8", "left - right", "a >= 0 && b >= 0", "out == 0", "", &only), [Unsupported]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8", "if (left == 0) {}; i8 local = 6; local + right", guard, "out == 7", "", &only), [Unsupported]);
        assert_eq!(count(), 0);
        let branch = "if (left == 6) { left + right } else { 5 }";
        assert_eq!(check("i8", branch, "b == 1", "out <= 7", "", &only), [Proved]);
        assert_eq!(count(), 1);
        assert_eq!(check("i8", "left + right", "false", "out <= 7", "", &only), [NoApplicableInputs]);
        assert_eq!(count(), 0);
        assert_eq!(check("i8", "left + right", guard, "out == 7", "i8 live = number(6, 1);", &only), [Unsupported]);
        assert_eq!(count(), 0);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn authored_show_implicit_singleton_keeps_identity_and_result_inline() {
        let file = source("src/main.w", "implicit-show-singleton");
        let mut slots = ImplicitAuthoredShows::default();
        let (first, check) = slots.cite((&file, "leaf"), || Some("prepared"));
        assert!(matches!(first, Some(AuthoredShowDestination::Implicit(0))));
        assert_eq!(check, Some("prepared"));
        *slots.outcome_mut(0) = true;
        let (again, check) = slots.cite::<()>((&file, "leaf"), || panic!("reprepared leaf"));
        assert!(matches!(again, Some(AuthoredShowDestination::Implicit(0))));
        assert!(check.is_none() && slots.proved(0));
        assert_eq!(slots.index.capacity(), 0);
        assert_eq!(slots.additional.capacity(), 0);
        let (second, _) = slots.cite((&file, "other"), || Some(()));
        assert!(matches!(second, Some(AuthoredShowDestination::Implicit(1))));
        assert!(slots.proved(0) && !slots.proved(1));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn authored_show_groups_share_only_exact_return_observations() {
        use super::super::checker_flow::take_integer_observations;
        use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
        let file = source("src/main.w", "group-authored-shows");
        let text = r#"%%start
            u8(u8) echo = fn(arg) { arg };
            proof passes for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) { core.show(out < 8); }
            proof fails for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) {
                core.show(out < 8); core.show(out <= 6);
            }
            proof other for echo(entry) -> (out) where (entry < 7) ensures (out <= 6) { core.show(out < 7); }
            proof plain for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) {}
            proof empty_first for echo(entry) -> (out) where (false) ensures (out <= 7) { core.show(out < 8); }
            proof empty_second for echo(entry) -> (out) where (false) ensures (out <= 7) {
                core.show(out < 8); core.show(out <= 6);
            }
            proof invalid for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) { core.show(out < 256); }
            proof cited for echo(entry) -> (out) where (entry < 8) ensures (out < 8) { core.use(passes(entry)); }
            proof failed_citation for echo(entry) -> (out) where (entry < 8) ensures (out < 8) { core.use(fails(entry)); }
            proof other_citation for echo(entry) -> (out) where (entry < 7) ensures (out < 7) { core.use(other(entry)); }
            proof stronger_citation for echo(entry) -> (out) where (entry < 8) ensures (out <= 6) { core.use(passes(entry)); }
            proof cycle_left for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) { core.use(cycle_right(entry)); }
            proof cycle_right for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) { core.use(cycle_left(entry)); }
        %%end"#;
        let run = |names: &[&str]| check_scalar_module_proofs(
            file.clone(), names,
            vec![ScalarProofModuleInput { source: file.clone(), text: text.into(), namespaces: Vec::new() }],
            vec![file.clone()], inputs(),
        ).unwrap();
        let _ = take_integer_observations();
        assert_eq!(run(&["fails", "passes", "cited"]), [Refuted, Proved, Proved]);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(run(&["cited", "failed_citation"]), [Proved, Unsupported]);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(run(&["passes", "failed_citation"]), [Proved, Unsupported]);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(run(&["cited", "other_citation"]), [Proved, Proved]);
        assert_eq!(take_integer_observations(), (2, 0));
        assert_eq!(run(&["stronger_citation", "cycle_left"]), [Unsupported, Unsupported]);
        assert_eq!(take_integer_observations(), (0, 0));
        assert_eq!(run(&["passes", "other"]), [Proved, Proved]);
        assert_eq!(take_integer_observations(), (2, 0));
        assert_eq!(run(&["passes", "plain"]), [Proved, Proved]);
        assert_eq!(take_integer_observations(), (2, 0));
        assert_eq!(run(&["empty_second", "empty_first"]), [NoApplicableInputs, NoApplicableInputs]);
        assert_eq!(take_integer_observations(), (1, 0));
        assert!(matches!(run(&["invalid"]).as_slice(), [Invalid(_)]));
        assert_eq!(take_integer_observations(), (0, 0));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn authored_show_peer_shares_verification_between_selected_and_cited() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "authored-show-peer");
        let text = r#"%%start
            u8(u8) echo = fn(value) { value };
            proof leaf for echo(input) -> (out) where (input < 8) ensures (out < 8) {
                core.show(out <= 7); core.show(out == core.old(input));
            }
            proof first for echo(entry) -> (out) where (entry < 8) ensures (out <= 7) { core.use(leaf(entry)); }
            proof second for echo(entry) -> (out) where (entry < 8) ensures (out < 8) { core.use(leaf(entry)); }
            proof stronger for echo(entry) -> (out) where (entry < 8) ensures (out <= 6) { core.use(leaf(entry)); }
            proof unrelated for echo(input) -> (out) where (input < 8) ensures (out < 8) {
                core.show(out <= 6);
            }
        %%end"#;
        let _ = take_integer_observations();
        let _ = UNSELECTED_PEER_PLANS.with(|count| count.replace(0));
        for (names, implicit) in [
            (&["first", "leaf", "second"][..], 0),
            (&["first", "second"][..], 1),
        ] {
            assert_eq!(check_scalar_module_proofs(
                file.clone(), names,
                vec![ScalarProofModuleInput { source: file.clone(), text: text.into(), namespaces: Vec::new() }],
                vec![file.clone()], inputs(),
            ).unwrap(), vec![ScalarModuleProofOutcome::Proved; names.len()]);
            assert_eq!(take_integer_observations(), (1, 0));
            assert_eq!(UNSELECTED_PEER_PLANS.with(|count| count.replace(0)), implicit);
        }
        for (names, selected) in [(&["stronger"][..], false), (&["stronger", "leaf"][..], true)] {
            let mut expected = vec![ScalarModuleProofOutcome::Unsupported];
            if selected { expected.push(ScalarModuleProofOutcome::Proved); }
            assert_eq!(check_scalar_module_proofs(
                file.clone(), names,
                vec![ScalarProofModuleInput { source: file.clone(), text: text.into(), namespaces: Vec::new() }],
                vec![file.clone()], inputs(),
            ).unwrap(), expected);
            assert_eq!(take_integer_observations(), (usize::from(selected), 0));
            assert_eq!(UNSELECTED_PEER_PLANS.with(|count| count.replace(0)), usize::from(!selected));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn strict_peer_non_equality_checks_implication_before_return_membership() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "strict-peer-predicate");
        let text = r#"%%start
            u8(u8) echo = fn(value) { value };
            proof leaf for echo(input) -> (out) where (input < 8) ensures (out < 8) {}
            proof narrower for echo(entry) -> (out) where (entry == 6) ensures (out == 6) { core.use(leaf(entry)); }
            proof weaker for echo(entry) -> (out) where (entry == 6) ensures (out <= 7) { core.use(leaf(entry)); }
            proof empty_leaf for echo(input) -> (out) where (false) ensures (out < 8) {}
            proof empty_exact for echo(entry) -> (out) where (false) ensures (out == 6) { core.use(empty_leaf(entry)); }
        %%end"#;
        let _ = take_integer_observations();
        let _ = STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0));
        let run = |names: &[&str]| check_scalar_module_proofs(
            file.clone(), names,
            vec![ScalarProofModuleInput { source: file.clone(), text: text.into(), namespaces: Vec::new() }],
            vec![file.clone()], inputs(),
        ).unwrap();
        assert_eq!(run(&["narrower"]), [ScalarModuleProofOutcome::Unsupported]);
        assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 0);
        assert_eq!(take_integer_observations(), (0, 0));
        assert_eq!(run(&["narrower", "weaker"]), [
            ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved,
        ]);
        assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 1);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(run(&["empty_exact"]), [ScalarModuleProofOutcome::NoApplicableInputs]);
        assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 0);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(run(&["narrower", "leaf"]), [
            ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved,
        ]);
        assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 0);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(run(&["narrower", "weaker", "leaf"]), [
            ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved,
        ]);
        assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 1);
        assert_eq!(take_integer_observations(), (1, 0));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn strict_peer_singleton_return_membership_is_shared_within_one_unused_group() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "strict-peer-singleton");
        let text = r#"%%start
            u8(u8) fixed = fn(value) { if (value > 7) { 7 } else { 7 } };
            proof leaf for fixed(input) -> (out) where (input <= 7) ensures (out == 7) {}
            proof first for fixed(entry) -> (out) where (entry == 6) ensures (out <= 7) { core.use(leaf(entry)); }
            proof again for fixed(entry) -> (out) where (entry == 6) ensures (out >= 7) { core.use(leaf(entry)); }
            proof distinct for fixed(entry) -> (out) where (entry == 5) ensures (out == 7) { core.use(leaf(entry)); }
            proof interval for fixed(entry) -> (out) where (entry >= 2 && entry <= 4) ensures (out <= 7) { core.use(leaf(entry)); }
            proof interval_again for fixed(entry) -> (out) where (entry <= 4 && entry >= 2) ensures (out >= 7) { core.use(leaf(entry)); }
            proof unrelated for fixed(input) -> (out) ensures (out == 256) {}
            proof distinct_again for fixed(entry) -> (out) where (entry == 5) ensures (out == 7) { core.use(leaf(entry)); }
        %%end"#;
        let _ = take_integer_observations();
        let _ = STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0));
        let _ = UNSELECTED_PEER_PLANS.with(|count| count.replace(0));
        let run = |names: &[&str]| {
            check_scalar_module_proofs(
                file.clone(), names,
                vec![ScalarProofModuleInput { source: file.clone(), text: text.into(), namespaces: Vec::new() }],
                vec![file.clone()], inputs(),
            ).unwrap()
        };
        let selected = ["first", "leaf", "again", "distinct", "distinct_again", "interval", "interval_again"];
        let cited = ["first", "again", "distinct", "distinct_again", "interval", "interval_again"];
        for (names, plans) in [(selected.as_slice(), 0), (cited.as_slice(), 1)] {
            assert_eq!(run(names), vec![ScalarModuleProofOutcome::Proved; names.len()]);
            assert_eq!(take_integer_observations(), (1, 0));
            assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 3);
            assert_eq!(UNSELECTED_PEER_PLANS.with(|count| count.replace(0)), plans);
        }
        assert_eq!(run(&["leaf"]), [ScalarModuleProofOutcome::Proved]);
        assert_eq!(take_integer_observations(), (1, 0));
        assert_eq!(STRICT_PEER_ENTRY_CHECKS.with(|count| count.replace(0)), 0);
        assert_eq!(UNSELECTED_PEER_PLANS.with(|count| count.replace(0)), 0);
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn ordered_proof_union_reuse_invokes_issuer_only_after_exact_caller_admission() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "union-issuer");
        let text = format!(r#"%%start
            u8(u8) producer = fn(value) {{ value }};
            u8(u8) live = fn(value) {{ value }};
            u8 actual = live(7);
            unit() accepted = fn {{
                u8 result = producer(7);
                core.require(result <= 7, {OUTPUT});
            }};
            unit() hole = fn {{
                u8 result = producer(4);
                core.require(result <= 7, {OUTPUT});
            }};
            unit(u8) broad = fn(value) {{
                u8 result = producer(value);
                core.require(result <= 7, {OUTPUT});
            }};
            unit() wrong = fn {{
                u8 result = live(7);
                core.require(result <= 7, {OUTPUT});
            }};
            proof domain for producer(input) -> (out) where (input == 1 || input == 7) ensures (out <= 7) {{}}
            proof output_union for producer(input) -> (out)
                where (input == 1 || input == 7) ensures (out == 1 || out == 7) {{}}
            proof refuted_union for producer(input) -> (out)
                where (input == 1 || input == 7) ensures (out == 1 || out == 8) {{}}
            proof no_domain for producer(input) -> (out)
                where (input == 1) ensures (out == 1 || out == 7) {{}}
            proof false_goal for producer(input) -> (out) where (input == 1 || input == 7) ensures (out <= 1) {{}}
            proof full for producer(input) -> (out) where (input < 8 || input >= 8) ensures (out <= 255) {{}}
            unit(u8) complete = fn(value) {{
                u8 result = producer(value);
                core.require(result <= 255, {OUTPUT});
            }};
        %%end"#);
        let run = |proof, caller| {
            let (_, catalog) = crate::parse_proof_source(file.clone(), text.clone(), &[]).ok().unwrap();
            check_loaded_scalar_module_proof_call(
                (&file, proof), (&file, caller), Vec::new(), (catalog, Vec::new()),
                vec![file.clone()], inputs(),
            )
        };
        let _ = take_integer_observations();
        let take_normalizations = || OUTPUT_UNION_NORMALIZATIONS.with(|count| count.replace(0));
        let _ = take_normalizations();
        for caller in ["hole", "broad", "wrong"] {
            assert_eq!(run("domain", caller), ScalarProofCallOutcome::Unavailable);
            assert_eq!(take_integer_observations(), (0, 0));
            assert_eq!(run("output_union", caller), ScalarProofCallOutcome::Unavailable);
            assert_eq!(take_integer_observations(), (0, 0));
            assert_eq!(take_normalizations(), 0);
        }
        assert_eq!(run("no_domain", "accepted"), ScalarProofCallOutcome::Unavailable);
        assert_eq!(take_integer_observations(), (0, 0));
        assert_eq!(take_normalizations(), 0);
        assert_eq!(run("output_union", "accepted"), ScalarProofCallOutcome::Established);
        assert_eq!(take_integer_observations(), (1, 1));
        assert_eq!(take_normalizations(), 1);
        assert_eq!(run("refuted_union", "accepted"), ScalarProofCallOutcome::Unavailable);
        assert_eq!(take_integer_observations(), (1, 1));
        assert_eq!(take_normalizations(), 1);
        assert_eq!(run("domain", "accepted"), ScalarProofCallOutcome::Established);
        assert_eq!(take_integer_observations(), (1, 1));
        assert_eq!(run("full", "complete"), ScalarProofCallOutcome::Established);
        assert_eq!(take_integer_observations(), (1, 1));
        assert_eq!(run("false_goal", "accepted"), ScalarProofCallOutcome::Unavailable);
        assert_eq!(take_integer_observations(), (1, 1));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn named_proof_batch_observes_each_exact_typed_arbitrary_entry_domain_once() {
        use super::super::checker_flow::take_integer_observations;
        let file = source("src/main.w", "shared-symbolic-batch");
        let text = r#"%%start
            u8(u8) dormant = fn(value) { value };
            u8(u8) live = fn(value) { value };
            u8 actual = live(7);
            proof cap for dormant(x) -> (out) where (x <= 10) ensures (out <= 10) {}
            proof too_low for dormant(y) -> (out) where (y <= 10) ensures (out < 10) {}
            proof tighter for dormant(z) -> (out) where (z <= 5) ensures (out <= 5) {}
            proof real for live(x) -> (out) ensures (out <= 7) {}
            proof real_fails for live(x) -> (out) ensures (out < 7) {}
        %%end"#;
        let _ = take_integer_observations();
        let outcomes = check_scalar_module_proofs(
            file.clone(),
            &["cap", "real", "tighter", "too_low", "real_fails"],
            vec![ScalarProofModuleInput { source: file.clone(), text: text.into(), namespaces: Vec::new() }],
            vec![file],
            inputs(),
        ).unwrap();
        assert_eq!(take_integer_observations(), (2, 1));
        assert_eq!(outcomes, [
            ScalarModuleProofOutcome::Proved,
            ScalarModuleProofOutcome::Proved,
            ScalarModuleProofOutcome::Proved,
            ScalarModuleProofOutcome::Refuted,
            ScalarModuleProofOutcome::Refuted,
        ]);
    }

    fn source(path: &str, revision: &str) -> SourceIdentity {
        SourceIdentity::new("local".into(), "demo".into(), path.into(), revision.into())
    }

    fn declarations(source: &SourceIdentity, text: &str) -> ScalarSourceDeclarations {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    }

    fn inputs() -> ScalarPreparationInputs {
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 }
    }

    fn script(body: &str) -> String {
        format!("%%start\nunit() proof = fn {{ {body} }};\n%%end")
    }

    fn check(source: &SourceIdentity, text: &str, name: &str) -> ProjectCheck {
        check_closed_project_callable(
            vec![(declarations(source, text), Vec::new())],
            vec![source.clone()],
            inputs(),
            source,
            name,
        )
    }

    #[test]
    fn checks_the_selected_project_callable_and_preserves_its_failure_spans() {
        let app = source("src/main.w", "app");
        let other = source("src/other.w", "dependency");
        let passing = script(&format!("u8 local = 7; if (local == 7) {{}}; core.require(local == 7, {OUTPUT});"));
        let failure_output = r#"{ .stdout = "saída\n"; .stderr = "erro\t☕"; .code = 340282366920938463463374607431768211456; .files = []; }"#;
        let failing = script(&format!("u8 local = 7; if (local == 7) {{}}; core.require(local == 8, {failure_output});"));
        for (source, expected_proof) in [(&app, false), (&other, true)] {
            let result = check_closed_project_callable(
                vec![
                    (declarations(&other, &passing), Vec::new()),
                    (declarations(&app, &failing), Vec::new()),
                ],
                vec![other.clone(), app.clone()],
                inputs(),
                source,
                "proof",
            );
            if expected_proof {
                assert_eq!(result, ProjectCheck::Proved);
            } else {
                let ProjectCheck::Failed { source, operation, directive, condition, state, axes, output } = result else {
                    panic!("expected exact failure, found {result:?}");
                };
                assert_eq!(source, app);
                assert_eq!(operation, CheckerOperation::Require);
                assert_eq!(&failing[condition.start as usize..condition.end as usize], "local == 8");
                assert_eq!(
                    &failing[directive.start as usize..directive.end as usize],
                    format!("core.require(local == 8, {failure_output})")
                );
                let authored = |span: ByteSpan| &failing[span.start as usize..span.end as usize];
                assert_eq!(output.stdout_bytes.as_ref(), "saída\n".as_bytes());
                assert_eq!(output.stderr_bytes.as_ref(), "erro\t☕".as_bytes());
                assert_eq!(output.code_value, BigInt::from(u128::MAX) + 1);
                assert_eq!(authored(output.stdout), r#""saída\n""#);
                assert_eq!(authored(output.stderr), r#""erro\t☕""#);
                assert_eq!(authored(output.code), "340282366920938463463374607431768211456");
                assert_eq!(authored(output.files), "[]");
                assert_eq!(axes.len(), 1);
                assert!(state.includes(&[7]));
                assert!(!state.includes(&[8]));
            }
        }

        let ordinary = prepare_scalar_project(
            vec![(declarations(&app, &failing), Vec::new())],
            vec![app],
            inputs(),
        );
        assert!(ordinary.is_err());
    }

    #[test]
    fn distinguishes_missing_invalid_unsupported_and_vacuous_obligations() {
        let app = source("src/main.w", "app");
        assert_eq!(check(&app, &script(""), "missing"), ProjectCheck::MissingRoot);
        assert_eq!(check(&app, &script(""), "proof"), ProjectCheck::NoChecks);
        let unsupported = "%%start\nunit() effect = fn {};\nunit() proof = fn { effect(); };\n%%end";
        assert_eq!(check(&app, unsupported, "proof"), ProjectCheck::Unsupported);
        let invalid = script("core.require(true, 0);");
        assert!(matches!(check(&app, &invalid, "proof"), ProjectCheck::Invalid(_)));
        let dead = script(&format!("if (false) {{ core.require(false, {OUTPUT}); }};"));
        assert_eq!(check(&app, &dead, "proof"), ProjectCheck::Proved);
        assert_eq!(
            check_closed_project_callable(
                vec![(declarations(&app, &dead), Vec::new())],
                Vec::new(),
                inputs(),
                &app,
                "proof",
            ),
            ProjectCheck::RootSourceOutsideInitOrder
        );
        assert_eq!(
            check_closed_project_callable(
                vec![(declarations(&app, &dead), Vec::new())],
                vec![app.clone()],
                inputs(),
                &source("src/main.w", "wrong-revision"),
                "proof",
            ),
            ProjectCheck::MissingRoot
        );
    }
    #[test]
    fn canonical_module_and_explicit_fragment_entries_keep_distinct_proof_envelopes() {
        use wosy_syntax::{parse_proof_fragment, SyntaxKind};

        let identity = source("src/main.w", "proof-fragment");
        let fragment = "proof clamp_bounded for clamp(x) -> (out) where (x <= 10) ensures (out <= 10) {}";
        let fragment_parse = parse_proof_fragment(identity.clone(), fragment.to_owned(), &[]);
        let parsed = fragment_parse.as_result();
        assert!(parsed.is_valid(), "{:?}", parsed.errors);
        assert_eq!(parsed.canonical_cst().source, identity);
        assert_eq!(parsed.reconstruct(), fragment);
        let declaration = parsed
            .canonical_cst()
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::ProofDecl)
            .expect("canonical fragment proof");
        assert!(declaration.children().any(|node| node.kind() == SyntaxKind::ProofWhere));
        assert!(declaration.children().any(|node| node.kind() == SyntaxKind::ProofEnsures));

        let module = format!("%%start\n{fragment}\n%%end");
        let ordinary = crate::parse_source(identity.clone(), module.clone(), &[]);
        assert!(ordinary.diagnostics.is_empty(), "{:?}", ordinary.diagnostics);
        let module_proof = ordinary.result.canonical_cst().root.descendants()
            .find(|node| node.kind() == SyntaxKind::ProofDecl)
            .expect("canonical named module proof");
        assert_eq!(module_proof.text().to_string(), fragment);
        assert!(!crate::parse_source(identity.clone(), fragment.into(), &[]).diagnostics.is_empty());
        assert!(!parse_proof_fragment(identity, module, &[]).as_result().is_valid());
    }

    fn prove(source: &SourceIdentity, callable: &str, proof: &str) -> ProjectCheck {
        let proof_source = SourceIdentity::new(
            source.project.clone(), source.package.clone(), "fragment.w".into(), "fragment".into(),
        );
        let fragment = wosy_syntax::parse_proof_fragment(proof_source, proof.to_owned(), &[]);
        match prepare_proof_target(source, callable) {
            Ok(prepared) => check_proof_fragment(&prepared, source, &fragment),
            Err(error) => error,
        }
    }

    fn prepare_proof_target(
        source: &SourceIdentity,
        callable: &str,
    ) -> Result<PreparedCheckerProject, ProjectCheck> {
        prepare_checker_project(
            vec![(declarations(source, callable), Vec::new())],
            vec![source.clone()],
            inputs(),
        )
    }

    #[test]
    fn selected_theorems_share_one_canonically_prepared_target() {
        let app = source("src/main.w", "shared-clamp");
        let callable = "%%start\nu8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };\n%%end";
        let prepared = prepare_proof_target(&app, callable).unwrap();
        let proved_source = source("proof/proved.fragment.w", "proved-r1");
        let proved = wosy_syntax::parse_proof_fragment(
            proved_source.clone(),
            "proof bounded for clamp(x) -> (out) ensures (out <= 10) {}".into(),
            &[],
        );
        let refuted = wosy_syntax::parse_proof_fragment(
            source("proof/refuted.fragment.w", "refuted-r1"),
            "proof smaller for clamp(input) -> (result) ensures (result <= 9) {}".into(),
            &[],
        );
        let (outcome, artifact) = check_proof_fragment_impl(&prepared, &app, &proved, true).unwrap();
        assert_eq!(outcome, ProjectCheck::Proved);
        assert_eq!(artifact.unwrap().proof_source, proved_source);
        let (outcome, artifact) = check_proof_fragment_impl(&prepared, &app, &refuted, true).unwrap();
        assert_eq!(outcome, ProjectCheck::Refuted);
        assert_eq!(artifact, None);
    }

    #[test]
    fn canonical_module_selects_proofs_in_their_actual_source_at_most_once() {
        let file = source("src/main.w", "mixed-proofs");
        let first = "proof bounded for clamp(x) -> (out) ensures (out <= 10) {}";
        let second = "proof smaller for clamp(x) -> (out) ensures (out <= 9) {}";
        let third = "proof not_here for elsewhere(x) -> (out) ensures (out <= 10) {}";
        let text = format!("%%start\n{first}\nu8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};\n{second}\n{third}\n%%end");
        let module = PreparedProofModule::prepare(file.clone(), text.clone(), inputs()).unwrap();
        assert_eq!(module.source, file);
        assert_eq!(module.prepared.parts().0.modules[0].items.len(), 1);
        for (name, proof, expected) in [
            ("bounded", first, ProjectCheck::Proved),
            ("smaller", second, ProjectCheck::Refuted),
        ] {
            let start = text.find(proof).unwrap() as u32;
            assert_eq!(module.check(name), Ok((ByteSpan::new(start, start + proof.len() as u32), expected)));
        }
        assert_eq!(module.check("not_here"), Err(ProjectCheck::MissingRoot));
        assert_eq!(module.check("absent"), Err(ProjectCheck::MissingRoot));
        let duplicate = text.replace("proof smaller", "proof bounded");
        let module = PreparedProofModule::prepare(source("src/main.w", "duplicate"), duplicate, inputs()).unwrap();
        assert_eq!(module.check("bounded"), Err(ProjectCheck::AmbiguousRoot));
    }

    #[test]
    fn invalid_module_cannot_offer_recovered_or_partially_valid_proofs() {
        let declaration = "proof bounded for identity(x) -> (out) ensures (out <= 10) {}";
        let text = format!("%%start\n{declaration}\nu8(u8) identity = fn(value) {{ nonsense nonsense; {declaration} value }};\n%%end");
        let result = PreparedProofModule::prepare(source("src/main.w", "invalid-proofs"), text, inputs());
        assert!(matches!(result, Err(ProjectCheck::InvalidProof)));
    }

    #[test]
    fn verified_module_bound_preserves_actual_proof_and_callable_sites() {
        let file = source("src/main.w", "module-proof-r1");
        let proof = "proof bounded for clamp(x) -> (out) where (x <= 10) ensures (out <= 10) {}";
        let false_proof = "proof smaller for clamp(x) -> (out) ensures (out <= 9) {}";
        let empty = "proof empty for clamp(x) -> (out) where (false) ensures (out <= 10) {}";
        let text = format!("%%start\nu8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};\n{proof}\n{false_proof}\n{empty}\n%%end");
        let module = PreparedProofModule::prepare(file.clone(), text.clone(), inputs()).unwrap();
        let artifact = module.verify("bounded").unwrap().bound;
        let proof_start = text.find(proof).unwrap() as u32;
        let target_start = text.find("clamp").unwrap() as u32;
        assert_eq!(artifact.proof_venue, ProofVenue::ModuleDeclaration);
        assert_eq!(artifact.proof_source, file);
        assert_eq!(artifact.proof_span, ByteSpan::new(proof_start, proof_start + proof.len() as u32));
        assert_eq!(artifact.proof_name, "bounded");
        assert_eq!(artifact.target, ResolvedCallTarget::ModuleCallable {
            source: file.clone(),
            declaration_span: ByteSpan::new(target_start, target_start + "clamp".len() as u32),
            concrete: ConcreteCallSelection::Declaration,
        });
        assert_eq!(artifact.target_layout, ScalarTargetLayout::WASM32);
        assert_eq!(artifact.parameters, [ScalarType::U8]);
        assert_eq!(artifact.output_slot, 0);
        assert_eq!(artifact.output_type, ScalarType::U8);
        assert_eq!(artifact.guards, [VerifiedEntryGuard::Integer {
            slot: 0,
            ty: ScalarType::U8,
            operator: BinaryOperator::LessEqual,
            bound: BigInt::from(10),
        }]);
        for name in ["smaller", "empty", "absent"] {
            assert!(module.verify(name).is_none());
        }
        let detached = wosy_syntax::parse_proof_fragment(file.clone(), proof.into(), &[]);
        assert_eq!(verify_return_bound(&module.prepared, &file, &detached), None);
        let duplicate = text.replace("proof smaller", "proof bounded");
        let module = PreparedProofModule::prepare(source("src/main.w", "duplicate-proof"), duplicate, inputs()).unwrap();
        assert!(module.verify("bounded").is_none());
    }

    #[test]
    fn module_consumes_only_its_own_verified_bound_at_a_supported_guarded_call() {
        let file = source("src/main.w", "module-bridge-r1");
        let text = format!(r#"%%start
            u8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};
            proof bounded for clamp(x) -> (out) where (x <= 10) ensures (out <= 10) {{}}
            proof smaller for clamp(x) -> (out) ensures (out <= 9) {{}}
            u8() inside = fn {{ u8 input = 7; u8 result = clamp(input); core.require(result <= 10, {OUTPUT}); result }};
            u8() outside = fn {{ u8 input = 11; u8 result = clamp(input); core.require(result <= 10, {OUTPUT}); result }};
            u8() too_narrow = fn {{ u8 result = clamp(7); core.require(result <= 8, {OUTPUT}); result }};
            u8() unsupported = fn {{ u8 result = clamp(6 + 1); core.require(result <= 10, {OUTPUT}); result }};
        %%end"#);
        let module = PreparedProofModule::prepare(file.clone(), text.clone(), inputs()).unwrap();
        let verified = module.verify("bounded").unwrap();
        assert_eq!(module.apply("inside", "bounded", &verified), Some(ImportedRequirement::Proved));
        assert_eq!(module.apply("too_narrow", "bounded", &verified), Some(ImportedRequirement::NotEstablished));
        for name in ["outside", "unsupported", "clamp"] {
            assert_eq!(module.apply(name, "bounded", &verified), None);
        }
        assert!(module.verify("smaller").is_none());
        assert_eq!(module.apply("inside", "smaller", &verified), None);
        let same_source = PreparedProofModule::prepare(file.clone(), text.clone(), inputs()).unwrap();
        assert_eq!(same_source.apply("inside", "bounded", &verified), None);
        let stale_source = source("src/main.w", "module-bridge-r2");
        let changed = PreparedProofModule::prepare(stale_source, text, inputs()).unwrap();
        assert_eq!(changed.apply("inside", "bounded", &verified), None);
        let detached = wosy_syntax::parse_proof_fragment(
            source("proof/detached.w", "detached-r1"),
            "proof bound for clamp(x) -> (out) where (x <= 10) ensures (out <= 10) {}".into(),
            &[],
        );
        let fragment_bound = verify_return_bound(&module.prepared, &file, &detached).unwrap();
        let identity = (&fragment_bound.proof_source, fragment_bound.proof_span, "bound");
        assert_eq!(
            use_verified_return_bound(&module.prepared, &file, "inside", identity, &fragment_bound),
            Some(ImportedRequirement::Proved),
        );
        let identity = (&verified.bound.proof_source, verified.bound.proof_span, "bounded");
        assert_eq!(use_verified_return_bound(&module.prepared, &file, "inside", identity, &verified.bound), None);
    }

    #[test]
    fn canonical_proof_project_retains_sparse_module_census_and_ordinary_dependencies() {
        let app = source("src/main.w", "proof-root");
        let lib = source("src/lib.w", "proof-lib");
        let empty = source("src/empty.w", "proof-free");
        let app_proof = "proof bound for value(x) -> (out) ensures (out <= 7) {}";
        let lib_proof = "proof bound for value(x) -> (out) ensures (out <= 8) {}";
        let imported = "proof imported for lib.value(x) -> (out) where (x <= 10) ensures (out <= 9) {}";
        let tighter = "proof tighter for lib.value(x) -> (out) ensures (out <= 7) {}";
        let private = "proof inaccessible for lib._hidden(x) -> (out) ensures (out <= 7) {}";
        let unknown = "proof missing_namespace for absent.value(x) -> (out) ensures (out <= 7) {}";
        let empty_guard = "proof empty_guard for lib.value(x) -> (out) where (false) ensures (out <= 9) {}";
        let app_text = format!("%%start\nlib = namespace demo \"src/lib.w\";\nfree = namespace demo \"src/empty.w\";\nu8(u8) value = fn(x) {{ if (x == 0) {{}}; 7 }};\nu8() use_lib = fn {{ u8 arg = 7; u8 result = lib.value(arg); core.require(result <= 9, {OUTPUT}); result }};\nu8() outside = fn {{ u8 result = lib.value(11); core.require(result <= 9, {OUTPUT}); result }};\nu8() too_narrow = fn {{ u8 result = lib.value(7); core.require(result <= 8, {OUTPUT}); result }};\nu8() wrong_local = fn {{ u8 result = value(7); core.require(result <= 9, {OUTPUT}); result }};\nu8() wrong_import = fn {{ u8 result = free.value(7); core.require(result <= 9, {OUTPUT}); result }};\n{app_proof}\n{imported}\n{tighter}\n{private}\n{unknown}\n{empty_guard}\n%%end");
        let lib_text = format!("%%start\nu8(u8) value = fn(x) {{ if (x == 0) {{}}; 9 }};\nu8(u8) _hidden = fn(x) {{ x }};\n{lib_proof}\n%%end");
        let namespace = |name: &str, target: &SourceIdentity| {
            let declaration = format!("{name} =");
            let start = app_text.find(&declaration).unwrap();
            let end = start + app_text[start..].find(';').unwrap() + 1;
            ScalarNamespaceBinding {
                binding: name.into(),
                target: target.clone(),
                span: ByteSpan::new(start as u32, end as u32),
            }
        };
        let modules = |lib: &SourceIdentity| vec![
            ScalarProofModuleInput {
                source: app.clone(),
                namespaces: vec![namespace("lib", lib), namespace("free", &empty)],
                text: app_text.clone(),
            },
            ScalarProofModuleInput { source: lib.clone(), namespaces: Vec::new(), text: lib_text.clone() },
            ScalarProofModuleInput { source: empty.clone(), namespaces: Vec::new(), text: "%%start\nu8(u8) value = fn(x) { x };\n%%end".into() },
        ];
        let order = vec![empty.clone(), lib.clone(), app.clone()];
        let project = PreparedProofModule::prepare_project(app.clone(), modules(&lib), order.clone(), inputs()).unwrap();
        let (prepared, _) = project.prepared.parts();
        assert_eq!(prepared.initialization_order, order);
        assert_eq!(prepared.modules.len(), 3);
        assert_eq!(project.proofs.len(), 2);
        assert!(!project.proofs.contains_key(&empty));
        for (file, text, proof, outcome) in [
            (&app, &app_text, app_proof, ProjectCheck::Proved),
            (&lib, &lib_text, lib_proof, ProjectCheck::Refuted),
        ] {
            let start = text.find(proof).unwrap() as u32;
            assert_eq!(project.check_at(file, "bound"), Ok((ByteSpan::new(start, start + proof.len() as u32), outcome)));
        }
        assert_eq!(project.check_at(&empty, "bound"), Err(ProjectCheck::MissingRoot));
        for (name, proof, expected) in [("imported", imported, ProjectCheck::Proved), ("tighter", tighter, ProjectCheck::Refuted)] {
            let start = app_text.find(proof).unwrap() as u32;
            assert_eq!(project.check_at(&app, name), Ok((ByteSpan::new(start, start + proof.len() as u32), expected)));
        }
        for name in ["inaccessible", "missing_namespace"] {
            assert_eq!(project.check_at(&app, name), Err(ProjectCheck::MissingRoot));
        }
        assert_eq!(project.check_at(&source("src/main.w", "stale"), "imported"), Err(ProjectCheck::MissingRoot));
        let artifact = project.verify("imported").unwrap();
        let bound = &artifact.bound;
        let proof_start = app_text.find(imported).unwrap() as u32;
        let target_start = lib_text.find("value").unwrap() as u32;
        assert_eq!(bound.proof_venue, ProofVenue::ModuleDeclaration);
        assert_eq!(bound.proof_source, app);
        assert_eq!(bound.proof_span, ByteSpan::new(proof_start, proof_start + imported.len() as u32));
        assert_eq!(bound.proof_name, "imported");
        assert_eq!(bound.target, ResolvedCallTarget::ModuleCallable {
            source: lib.clone(),
            declaration_span: ByteSpan::new(target_start, target_start + "value".len() as u32),
            concrete: ConcreteCallSelection::Declaration,
        });
        assert_eq!(project.apply("use_lib", "imported", &artifact), Some(ImportedRequirement::Proved));
        assert_eq!(project.apply("too_narrow", "imported", &artifact), Some(ImportedRequirement::NotEstablished));
        for caller in ["outside", "wrong_local", "wrong_import"] {
            assert_eq!(project.apply(caller, "imported", &artifact), None);
        }
        let fresh = PreparedProofModule::prepare_project(app.clone(), modules(&lib), order, inputs()).unwrap();
        assert_eq!(fresh.apply("use_lib", "imported", &artifact), None);
        let stale_lib = source("src/lib.w", "proof-lib-r2");
        let changed = PreparedProofModule::prepare_project(
            app.clone(), modules(&stale_lib), vec![empty, stale_lib, app.clone()], inputs(),
        ).unwrap();
        assert_eq!(changed.apply("use_lib", "imported", &artifact), None);
        for name in ["tighter", "inaccessible", "missing_namespace", "empty_guard", "missing"] {
            assert!(project.verify(name).is_none());
        }
    }

    #[test]
    fn proof_census_rejects_a_parsed_revision_outside_the_graph_order() {
        let graph_source = source("src/main.w", "graph-revision");
        let parsed_source = source("src/main.w", "other-revision");
        let result = PreparedProofModule::prepare_project(
            graph_source.clone(),
            vec![ScalarProofModuleInput {
                source: parsed_source,
                text: "%%start\nu8() answer = fn { 42 };\n%%end".into(),
                namespaces: Vec::new(),
            }],
            vec![graph_source],
            inputs(),
        );
        assert!(matches!(result, Err(ProjectCheck::RootSourceOutsideInitOrder)));
    }

    #[test]
    fn checker_target_queries_share_canonical_ordinary_namespace_resolution() {
        let app = source("src/main.w", "query-app");
        let lib = source("src/lib.w", "query-lib");
        let stale = source("src/main.w", "stale-app");
        let app_text = "%%start\nlib = namespace demo \"src/lib.w\";\nu8() value = fn { 7 };\nu8() caller = fn { lib.value() };\n%%end";
        let lib_text = "%%start\nu8() value = fn { 9 };\nu8() _hidden = fn { 11 };\n%%end";
        let namespace_start = app_text.find("lib =").unwrap() as u32;
        let namespace_end = app_text.find(';').unwrap() as u32 + 1;
        let query = |source, receiver, name| CheckerTargetQuery { source, receiver, name };
        let prepared = prepared::prepare_checker_project_with_queries(
            vec![
                (declarations(&app, app_text), vec![ScalarNamespaceBinding {
                    binding: "lib".into(), target: lib.clone(),
                    span: ByteSpan::new(namespace_start, namespace_end),
                }]),
                (declarations(&lib, lib_text), Vec::new()),
            ],
            vec![lib.clone(), app.clone()],
            inputs(),
            &[
                query(&app, None, "value"),
                query(&app, Some("lib"), "value"),
                query(&app, Some("lib"), "_hidden"),
                query(&app, Some("unknown"), "value"),
                query(&stale, Some("lib"), "value"),
            ],
        ).unwrap();
        let (project, calls) = prepared.parts();
        let [Some(local), Some(imported), None, None, None] = calls.checker_targets.as_slice() else {
            panic!("expected exact local and imported callable only");
        };
        for (row, source) in [(local, &app), (imported, &lib)] {
            let (module, item, function) = select_checked_root(project, source, "value").unwrap();
            assert_eq!(row.declaration, (module, item));
            assert_eq!(row.target, ResolvedCallTarget::ModuleCallable {
                source: source.clone(), declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Declaration,
            });
        }
        let (module, item, _) = select_checked_root(project, &app, "caller").unwrap();
        let caller = calls.function_body(module, item, None).unwrap();
        let occurrence = &caller.call_occurrences[0];
        let resolved = caller.resolved_call_site(occurrence.ordinal).unwrap();
        assert_eq!(resolved.target, imported.target);
    }

    #[test]
    fn package_initializer_uses_canonical_calls_with_actual_integer_entry_arguments() {
        let file = source("src/main.w", "initializer");
        let text = |argument: &str| format!(r#"%%start
            unit(u8) need_small = fn(value) {{ if (value == 0) {{}}; core.require(value <= 10, {OUTPUT}); }};
            unit() noop = fn {{}};
            noop();
            need_small({argument});
        %%end"#);
        let package = prepare_proof_target(&file, &text("7")).unwrap();
        assert_eq!(check_package_initialization(&package).map(|result| result.reached), Ok(HashSet::from([(0, 0), (0, 1)])));
        let (_, calls) = package.parts();
        assert!(matches!(check_integer_requirements(calls.function_body(0, 0, None).unwrap()), Some(Err(_))));
        let failure = prepare_proof_target(&source("src/main.w", "initializer-refuted"), &text("11")).unwrap();
        assert!(matches!(check_package_initialization(&failure), Err(ProjectCheck::Failed { .. })));
        let unsupported = prepare_proof_target(&source("src/main.w", "initializer-unsupported"), &text("3 + 4")).unwrap();
        assert_eq!(check_package_initialization(&unsupported), Err(ProjectCheck::Unsupported));
    }

    #[test]
    fn package_initializer_dispatches_at_authored_namespace_steps_from_the_root() {
        let app = source("src/main.w", "initializer-root");
        let lib = source("src/lib.w", "initializer-lib");
        let disconnected = source("src/disconnected.w", "initializer-disconnected");
        let prepare = |root_argument: u8, library_argument: u8| {
            let app_text = format!(r#"%%start
                unit(u8) root_check = fn(value) {{ if (value == 0) {{}}; core.require(value <= 10, {OUTPUT}); }};
                root_check({root_argument});
                lib = namespace demo "src/lib.w";
                duplicate = namespace demo "src/lib.w";
                duplicate.check(7);
            %%end"#);
            let lib_text = format!(r#"%%start
                unit(u8) check = fn(value) {{ if (value == 0) {{}}; core.require(value <= 10, {OUTPUT}); }};
                check({library_argument});
            %%end"#);
            let disconnected_text = "%%start\nu8 poison = 1;\n%%end";
            let namespace = |name: &str| {
                let start = app_text.find(&format!("{name} =")).unwrap();
                let end = start + app_text[start..].find(';').unwrap() + 1;
                ScalarNamespaceBinding {
                    binding: name.into(), target: lib.clone(), span: ByteSpan::new(start as u32, end as u32),
                }
            };
            prepare_checker_project(
                vec![
                    (declarations(&app, &app_text), vec![namespace("lib"), namespace("duplicate")]),
                    (declarations(&lib, &lib_text), Vec::new()),
                    (declarations(&disconnected, disconnected_text), Vec::new()),
                ],
                vec![app.clone(), lib.clone(), disconnected.clone()],
                inputs(),
            ).unwrap()
        };
        for (root, library, first_failure) in [(11, 11, &app), (7, 11, &lib)] {
            let prepared = prepare(root, library);
            let Err(ProjectCheck::Failed { source, .. }) = check_package_initialization(&prepared) else {
                panic!("expected authored initializer failure");
            };
            assert_eq!(&source, first_failure);
        }
        let prepared = prepare(7, 7);
        assert_eq!(check_package_initialization(&prepared).map(|result| result.reached), Ok(HashSet::from([(0, 0), (1, 0)])));
    }

    #[test]
    fn unused_symbolic_roots_follow_exact_initialization_and_exclude_dependencies() {
        let app = source("src/main.w", "unused-root");
        let lib = source("src/local.w", "unused-local");
        let dependency = SourceIdentity::new("local".into(), "dependency".into(), "src/lib.w".into(), "unused-dep".into());
        let prepare = |argument: &str, private_limit: u8, dead_limit: u8| {
            let app_text = format!(r#"%%start
                local = namespace demo "src/local.w";
                dep = namespace dependency "src/lib.w";
                unit(u8) actual = fn(value) {{
                    if (value == 0) {{ local.unreachable_to_caller(value + 1); }};
                    core.require(value <= 10, {OUTPUT});
                }};
                actual({argument});
            %%end"#);
            let lib_text = format!(r#"%%start
                unit(u8) _unused = fn(value) {{ if (value == 0) {{}}; core.require(value <= {private_limit}, {OUTPUT}); }};
                unit(u8) unreachable_to_caller = fn(value) {{ if (value == 0) {{}}; core.require(value <= {dead_limit}, {OUTPUT}); }};
            %%end"#);
            let dependency_text = format!(r#"%%start
                unit(u8) not_an_own_root = fn(value) {{ if (value == 0) {{}}; core.require(value <= 10, {OUTPUT}); }};
            %%end"#);
            let namespace = |name: &str, target: &SourceIdentity| {
                let start = app_text.find(&format!("{name} =")).unwrap();
                let end = start + app_text[start..].find(';').unwrap() + 1;
                ScalarNamespaceBinding { binding: name.into(), target: target.clone(), span: ByteSpan::new(start as u32, end as u32) }
            };
            prepare_checker_project(
                vec![
                    (declarations(&app, &app_text), vec![namespace("local", &lib), namespace("dep", &dependency)]),
                    (declarations(&lib, &lib_text), Vec::new()),
                    (declarations(&dependency, &dependency_text), Vec::new()),
                ],
                vec![app.clone(), lib.clone(), dependency.clone()],
                inputs(),
            ).unwrap()
        };
        let supported = prepare("7", 255, 255);
        assert_eq!(check_package_roots(&supported), Ok(()));
        let unused_failure = prepare("7", 10, 255);
        let Err(ProjectCheck::Failed { source, .. }) = check_package_roots(&unused_failure) else {
            panic!("expected unused same-package root to fail");
        };
        assert_eq!(source, lib);
        let dead_callee = prepare("7", 255, 10);
        let Err(ProjectCheck::Failed { source, .. }) = check_package_roots(&dead_callee) else {
            panic!("a syntactic dead call must not exclude its symbolic root");
        };
        assert_eq!(source, lib);
        for argument in ["0", "3 + 4"] {
            let unsupported = prepare(argument, 10, 255);
            assert_eq!(check_package_roots(&unsupported), Err(ProjectCheck::Unsupported));
        }
    }

    #[test]
    fn actual_nested_calls_use_each_reaching_literal_context_and_callee_failure() {
        let app = source("src/main.w", "actual-nested");
        let dependency = source("src/lib.w", "actual-callee");
        let prepare = |second: &str| {
            let text = format!(r#"%%start
                lib = namespace demo "src/lib.w";
                unit() enter = fn {{ lib.gate(7); lib.gate({second}); }};
                enter();
            %%end"#);
            let library = format!(r#"%%start
                unit(u8) gate = fn(value) {{ if (value == 0) {{}}; core.require(value <= 10, {OUTPUT}); }};
            %%end"#);
            let start = text.find("lib =").unwrap();
            let end = start + text[start..].find(';').unwrap() + 1;
            prepare_checker_project(
                vec![
                    (declarations(&app, &text), vec![ScalarNamespaceBinding {
                        binding: "lib".into(), target: dependency.clone(),
                        span: ByteSpan::new(start as u32, end as u32),
                    }]),
                    (declarations(&dependency, &library), Vec::new()),
                ],
                vec![app.clone(), dependency.clone()],
                inputs(),
            ).unwrap()
        };
        let actual = prepare("9");
        assert_eq!(check_package_roots(&actual), Ok(()));
        let failure = prepare("11");
        let Err(ProjectCheck::Failed { source, state, .. }) = check_package_roots(&failure) else {
            panic!("expected the second nested call's authored failure");
        };
        assert_eq!(source, dependency);
        assert!(state.includes(&[11]));
        assert!(!state.includes(&[7]));
        let unsupported = prepare("3 + 4");
        assert_eq!(check_package_roots(&unsupported), Err(ProjectCheck::Unsupported));
    }

    #[test]
    fn reachable_nested_recursion_is_not_a_completed_package() {
        let file = source("src/main.w", "actual-cycle");
        let text = "%%start\nunit() cycle = fn { cycle(); };\ncycle();\n%%end";
        let prepared = prepare_proof_target(&file, text).unwrap();
        assert_eq!(check_package_initialization(&prepared).map(drop), Err(ProjectCheck::Unsupported));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_actual_unit_self_calls_descend_without_admitting_cycles() {
        let text = |base_limit: u8, next: &str, tail: &str| format!(r#"%%start
            unit(u8) descend = fn(value) {{
                if (value == 2) {{ descend({next}); }};
                if (value == 1) {{ descend(0); }};
                core.require(value >= {base_limit}, {OUTPUT});
            }};
            descend(2);
            {tail}
        %%end"#);
        let run = |revision, text: &str, scope| {
            let prepared = prepare_proof_target(&source("src/main.w", revision), text).unwrap();
            run_closed_without_named_proofs(&prepared, scope, 1)
        };
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let pass = run("exact-self-pass", &text(0, "1", "descend(2);"), scope).unwrap();
            assert!(pass.active && pass.directives.is_empty());
            let computed = text(0, "value - 1", "descend(2);").replace("if (value == 1) { descend(0); };", "if (value == 1) { descend(value - 1); };");
            let pass = run("exact-self-decrement", &computed, scope).unwrap();
            assert!(pass.active && pass.directives.is_empty());
            let late = OUTPUT.replace(".code = 1", ".code = 5");
            let failure = run("exact-self-failure",
                &text(3, "value - 1", &format!("core.require(false, {late});"))
                    .replace("if (value == 1) { descend(0); };", "if (value == 1) { descend(value - 1); };"),
                scope).unwrap();
            let [base, inner, outer, startup] = failure.directives.as_slice()
                else { panic!("expected the three reached frames in return order before the startup directive"); };
            for (failed, value) in [base, inner, outer].into_iter().zip(0..3) {
                assert_eq!(failed.code(), &BigInt::from(1));
                let value = BigInt::from(value);
                assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&value, &value)]));
            }
            assert_eq!(startup.code(), &BigInt::from(5));
            for (revision, argument) in [("same-entry", "2"), ("increased-entry", "3"), ("zero-decrement", "value - 0"), ("underflow", "value - 3"), ("arithmetic-increase", "value + 1")] {
                assert!(matches!(run(revision, &text(0, argument, ""), scope), Err(ProjectCheck::Unsupported)), "{revision}");
            }
            let duplicate = text(0, "1", "").replace("descend(1);", "descend(1); descend(0);");
            assert!(matches!(run("duplicate-reached-self-edge", &duplicate, scope), Err(ProjectCheck::Unsupported)));
            let mutual = format!(r#"%%start
                unit(u8) first = fn(n) {{ if (n == 2) {{ second(1); }}; core.require(n >= 0, {OUTPUT}); }};
                unit(u8) second = fn(n) {{ if (n == 1) {{ first(0); }}; }};
                first(2);
            %%end"#);
            assert!(matches!(run("mutual-cycle", &mutual, scope), Err(ProjectCheck::Unsupported)));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn projected_conditional_local_write_preserves_both_preimages() {
        let identity = OUTPUT.replace(".code = 1", ".code = 89");
        let constant = OUTPUT.replace(".code = 1", ".code = 90");
        let program = format!(r#"%%start
            i8(i8) transfer = fn(entry) {{
                i8 changed = entry;
                i8 previous = changed;
                changed = if (changed < 0) {{ 13 }} else {{ previous }};
                changed
            }};
            unit(i8) check = fn(input) {{
                i8 output = transfer(input);
                if (output == 0) {{ core.require(input != 0, {identity}); }};
                if (output == 13) {{ core.require(input >= 0, {constant}); }};
            }};
        %%end"#);
        let root = source("src/main.w", "projected-conditional-local-write-preimages");
        let prepared = prepare_proof_target(&root, &program).unwrap();
        let receipt = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        let [identity, constant] = receipt.directives.as_slice() else { panic!("both authored preimages must reach"); };
        assert_eq!(identity.code(), &BigInt::from(89));
        assert_eq!(constant.code(), &BigInt::from(90));
        let zero = BigInt::from(0);
        assert!(identity.witness.intervals_at(0, 0).unwrap().eq([(&zero, &zero)]));
        assert_eq!(identity.witness.row_equal_axes(0), Some(&[(1, 0)][..]));
        let (min, max) = (BigInt::from(-128), BigInt::from(-1));
        assert!(constant.witness.intervals_at(0, 0).unwrap().eq([(&min, &max)]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn projected_returning_local_copy_keeps_its_entry_relation_and_the_old_alias() {
        let failure_output = OUTPUT.replace(".code = 1", ".code = 88");
        let program = format!(r#"%%start
            i8(i8) transfer = fn(entry) {{
                i8 changed = 2;
                i8 previous = changed;
                changed = entry;
                if (previous == 2) {{ changed }} else {{ 0 }}
            }};
            unit(i8) check = fn(input) {{
                i8 output = transfer(input);
                if (output == 3) {{
                    core.require(input == 3, {OUTPUT});
                    core.require(input != 3, {failure_output});
                }};
            }};
        %%end"#);
        let root = source("src/main.w", "projected-local-copy-relation");
        let prepared = prepare_proof_target(&root, &program).unwrap();
        let report = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        let [failure] = report.directives.as_slice() else { panic!("reaching output == 3 must identify only the third input"); };
        assert_eq!(failure.code(), &BigInt::from(88));
        let three = BigInt::from(3);
        for axis in [0, 1] {
            assert!(failure.witness.intervals_at(0, axis).unwrap().eq([(&three, &three)]));
        }
        assert_eq!(failure.witness.row_equal_axes(0), Some(&[(1, 0)][..]));
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn projected_narrowed_integer_return_reaches_only_the_authored_caller_branch() {
        let reached = OUTPUT.replace(".code = 1", ".code = 86");
        let sibling = OUTPUT.replace(".code = 1", ".code = 87");
        let program = format!(r#"%%start
            i8(i8) retain = fn(entry) {{
                core.narrow(entry <= 0, {reached});
                entry
            }};
            unit(i8) check = fn(input) {{
                if (input > 0) {{
                    i8 output = retain(input);
                    core.require(output > 127, {OUTPUT});
                }} else {{
                    core.require(input > 0, {sibling});
                }};
            }};
        %%end"#);
        let root = source("src/main.w", "projected-narrowed-return-reached-branch");
        let prepared = prepare_proof_target(&root, &program).unwrap();
        let main = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::MainFlowOnly, 1).unwrap();
        assert!(main.directives.is_empty());
        let package = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::CurrentPackage, 1).unwrap();
        let [reached, sibling] = package.directives.as_slice() else { panic!("empty call result and sibling only"); };
        assert_eq!(reached.code(), &BigInt::from(86));
        assert_eq!(sibling.code(), &BigInt::from(87));
        for (failure, first, last) in [(reached, 1, 127), (sibling, -128, 0)] {
            assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&BigInt::from(first), &BigInt::from(last))]));
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_integer_returns_transfer_local_overwrites_without_mutating_original_aliases() {
        let original = OUTPUT.replace(".code = 1", ".code = 84");
        let returned = OUTPUT.replace(".code = 1", ".code = 85");
        let text = format!(r#"%%start
            i8(i8) revise = fn(entry) {{
                i8 current = 2;
                i8 old = current;
                current = entry;
                core.require(old != 2, {original});
                current
            }};
            i8 saved = revise(7);
            core.require(saved != 7, {returned});
        %%end"#);
        let source = source("src/main.w", "exact-scalar-return-local-overwrite");
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            let prepared = prepare_proof_target(&source, &text).unwrap();
            let receipt = run_closed_without_named_proofs(&prepared, scope, 1).unwrap();
            let [original, returned] = receipt.directives.as_slice() else { panic!("expected original alias and written return"); };
            assert_eq!(original.code(), &BigInt::from(84));
            assert_eq!(returned.code(), &BigInt::from(85));
            for (axis, value) in [7, 7, 2].map(BigInt::from).iter().enumerate() {
                assert!(original.witness.intervals_at(0, axis).unwrap().eq([(value, value)]));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_scalar_narrow_runs_only_real_surviving_return_continuations() {
        let narrow = OUTPUT.replace(".code = 1", ".code = 81");
        let after = OUTPUT.replace(".code = 1", ".code = 82");
        let startup_after = OUTPUT.replace(".code = 1", ".code = 83");
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            for input in [1i16, 3] {
                let text = format!(r#"%%start
                    i8(i8) restrict = fn(entry) {{
                        core.narrow(entry >= 2, {narrow});
                        entry
                    }};
                    i8(i8) forward = fn(entry) {{
                        i8 result = restrict(entry);
                        core.require(result > 127, {after});
                        result
                    }};
                    i8 saved = forward({input});
                    core.require(saved > 127, {startup_after});
                %%end"#);
                let root = source("src/main.w", &format!("exact-scalar-narrow-{input}"));
                let prepared = prepare_proof_target(&root, &text).unwrap();
                let receipt = run_closed_without_named_proofs(&prepared, scope, 1).unwrap();
                if input == 1 {
                    let [failure] = receipt.directives.as_slice() else { panic!("only the failed narrow may produce output"); };
                    assert_eq!(failure.code(), &BigInt::from(81));
                    let one = BigInt::from(1);
                    assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&one, &one)]));
                } else {
                    assert!(receipt.directives.iter().map(|failure| failure.code().clone())
                        .eq([82, 83].map(BigInt::from)));
                    let three = BigInt::from(3);
                    assert!(receipt.directives[0].witness.intervals_at(0, 1).unwrap().eq([(&three, &three)]));
                }
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_ranked_integer_returns_resume_the_parent_and_keep_failure_order() {
        let base = OUTPUT.replace(".code = 1", ".code = 71");
        let unwind = OUTPUT.replace(".code = 1", ".code = 72");
        let final_receipt = OUTPUT.replace(".code = 1", ".code = 73");
        let text = format!(r#"%%start
            u8(u8) descend = fn(value) {{
                if (value > 0) {{
                    u8 result = descend(value - 1);
                    core.require(result == 7, {OUTPUT});
                    core.require(value >= 3, {unwind});
                    result
                }} else {{
                    core.require(value != 0, {base});
                    7
                }}
            }};
            unit() startup = fn {{
                u8 final = descend(2);
                core.require(final == 7, {OUTPUT});
                core.require(final != 7, {final_receipt});
            }};
            startup();
        %%end"#);
        let root = source("src/main.w", "ranked-exact-int-return");
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            for code in ["value - 1", "value", "value - 0", "value - 3", "value + 1"] {
                let candidate = text.replace("value - 1", code);
                let prepared = prepare_proof_target(&root, &candidate).unwrap();
                if code == "value - 1" {
                    let (project, calls) = prepared.parts();
                    let (module, item, _) = select_checked_root(project, &root, "descend").unwrap();
                    let body = calls.function_body(module, item, None).unwrap();
                    assert_eq!(body.structure.checker_integer_call_subtractions.len(), 1);
                    assert!(body.structure.checker_integer_call_locals.values().any(|origin|
                        matches!(origin.arguments.as_slice(),
                            [(_, super::super::reference::CheckerIntegerCallArgument::RankedSelfDescent)])));
                }
                let result = run_closed_without_named_proofs(&prepared, scope, 1);
                if code != "value - 1" {
                    assert!(matches!(result, Err(ProjectCheck::Unsupported)), "{code}");
                    continue;
                }
                let receipt = result.unwrap();
                assert!(receipt.directives.iter().map(|failure| failure.code().clone())
                    .eq([71, 72, 72, 73].map(BigInt::from)));
                for (failure, exact) in receipt.directives.iter().zip([0, 1, 2, 7].map(BigInt::from)) {
                    assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&exact, &exact)]));
                }
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_actual_unit_self_calls_count_all_active_frames() {
        let program = |wrappers: usize| {
            let mut text = format!("%%start\nunit(u8) descend = fn(n) {{ if (n == 1) {{ descend(0); }}; core.require(n <= 1, {OUTPUT}); }};\n");
            for index in 0..wrappers {
                let body = if index == 0 { "descend(1);".to_owned() } else { format!("f{}();", index - 1) };
                text.push_str(&format!("unit() f{index} = fn {{ {body} }};\n"));
            }
            text.push_str(&format!("f{}();\n%%end", wrappers - 1));
            text
        };
        for (frames, admitted) in [(MAX_ACTUAL_CALL_DEPTH, true), (MAX_ACTUAL_CALL_DEPTH + 1, false)] {
            let file = source("src/main.w", &format!("exact-self-frame-{frames}"));
            let prepared = prepare_proof_target(&file, &program(frames - 2)).unwrap();
            let result = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::MainFlowOnly, 1);
            if admitted {
                let result = result.unwrap();
                assert!(result.active && result.directives.is_empty());
            } else {
                assert!(matches!(result, Err(ProjectCheck::Unsupported)));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_actual_unit_self_calls_preserve_narrowing_and_order() {
        let output = |code| OUTPUT.replace(".code = 1", &format!(".code = {code}"));
        let input = |start| format!(r#"%%start
            unit(u8) descend = fn(n) {{
                if (n == 1) {{ descend(n - 1); core.require(false, {}); }}
                else {{ core.require(false, {}); }};
                core.narrow(n != 0, {});
                core.require(false, {});
            }};
            core.require(false, {});
            descend({start});
            core.require(false, {});
        %%end"#, output(2), output(3), output(4), output(5), output(6), output(7));
        for scope in [ScalarCheckerScope::MainFlowOnly, ScalarCheckerScope::CurrentPackage] {
            for (start, expected) in [(1, &[6, 3, 4][..]), (2, &[6, 3, 5, 7][..])] {
                let file = source("src/main.w", &format!("exact-narrowed-cursor-{start}"));
                let prepared = prepare_proof_target(&file, &input(start)).unwrap();
                let result = run_closed_without_named_proofs(&prepared, scope, 1).unwrap();
                assert!(result.active);
                assert!(result.directives.iter().map(|failure| failure.code())
                    .eq(expected.iter().map(|code| BigInt::from(*code)).collect::<Vec<_>>().iter()));
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_actual_unit_self_calls_count_branched_frames() {
        let literal_branches = (1..=MAX_ACTUAL_CALL_DEPTH)
            .map(|n| format!("if (n == {n}) {{ descend({}); }};", n - 1))
            .collect::<Vec<_>>()
            .join("\n");
        for (kind, branches) in [
            ("literal", literal_branches.as_str()),
            ("computed", "if (n > 0) { descend(n - 1); };"),
        ] {
            for (frames, admitted) in [(MAX_ACTUAL_CALL_DEPTH, true), (MAX_ACTUAL_CALL_DEPTH + 1, false)] {
                let file = source("src/main.w", &format!("exact-{kind}-self-frame-{frames}"));
                let program = format!(r#"%%start
                    unit(u8) descend = fn(n) {{ {branches} core.require(n <= 64, {OUTPUT}); }};
                    descend({});
                %%end"#, frames - 1);
                let prepared = prepare_proof_target(&file, &program).unwrap();
                let result = run_closed_without_named_proofs(&prepared, ScalarCheckerScope::MainFlowOnly, 1);
                if admitted {
                    let result = result.unwrap();
                    assert!(result.active && result.directives.is_empty(), "{kind}");
                } else {
                    assert!(matches!(result, Err(ProjectCheck::Unsupported)), "{kind}");
                }
            }
        }
    }

    #[cfg(feature = "internal-checker")]
    #[test]
    fn exact_actual_unit_self_calls_do_not_expand_named_proof_reuse() {
        let file = source("src/main.w", "exact-self-named-proof-isolation");
        let run = |start| {
            let text = format!(r#"%%start
                unit(u8) descend = fn(n) {{ if (n == 1) {{ descend(0); }}; core.require(n <= 1, {OUTPUT}); }};
                descend({start});
                u8(u8) producer = fn(value) {{ value }};
                unit() caller = fn {{
                    u8 result = producer(7);
                    core.require(result <= 7, {OUTPUT});
                }};
                proof literal for producer(input) -> (out) where (input <= 7) ensures (out <= 7) {{}}
            %%end"#);
            let (_, catalog) = crate::parse_proof_source(file.clone(), text, &[]).ok().unwrap();
            check_loaded_scalar_module_proof_call(
                (&file, "literal"), (&file, "caller"), Vec::new(), (catalog, Vec::new()),
                vec![file.clone()], inputs(),
            )
        };
        assert_eq!(run(0), ScalarProofCallOutcome::Established);
        assert_eq!(run(1), ScalarProofCallOutcome::Unavailable);
    }

    #[test]
    fn actual_nested_calls_use_exact_lexical_values_at_the_call_site() {
        let file = source("src/main.w", "actual-lexical");
        let text = |replacement: u8| format!(r#"%%start
            unit(u8) gate = fn(value) {{ if (value == 0) {{}}; core.require(value <= 10, {OUTPUT}); }};
            unit(u8) enter = fn(input) {{
                gate(input);
                u8 local = 7;
                gate(local);
                input = {replacement};
                gate(input);
            }};
            enter(9);
        %%end"#);
        let prepared = prepare_proof_target(&file, &text(10)).unwrap();
        assert_eq!(check_package_roots(&prepared), Ok(()));
        let failed = prepare_proof_target(&source("src/main.w", "actual-lexical-overwrite"), &text(11)).unwrap();
        let Err(ProjectCheck::Failed { state, .. }) = check_package_roots(&failed) else {
            panic!("expected overwritten singleton at actual nested call");
        };
        assert!(state.includes(&[11]));
        assert!(!state.includes(&[9]));
    }

    #[test]
    fn actual_nested_acyclic_depth_is_bounded_privately() {
        fn chain(count: usize) -> String {
            let mut text = "%%start\nunit() f0 = fn {};\n".to_owned();
            for index in 1..count {
                text.push_str(&format!("unit() f{index} = fn {{ f{}(); }};\n", index - 1));
            }
            text.push_str(&format!("f{}();\n%%end", count - 1));
            text
        }
        for (depth, expected) in [(MAX_ACTUAL_CALL_DEPTH, Ok(())), (MAX_ACTUAL_CALL_DEPTH + 1, Err(ProjectCheck::Unsupported))] {
            let source = source("src/main.w", &format!("depth-{depth}"));
            let prepared = prepare_proof_target(&source, &chain(depth)).unwrap();
            assert_eq!(check_package_initialization(&prepared).map(drop), expected);
            #[cfg(feature = "internal-checker")]
            assert_eq!(collect_package_startup_failures(&prepared).map(drop), expected);
        }
    }

    #[test]
    fn actual_scalar_call_and_proof_use_the_same_exact_return_observer() {
        let file = source("src/main.w", "actual-return");
        let prepare = |text_revision: &str, limit: u8, value: &str| {
            let text = format!(r#"%%start
                u8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};
                proof bounded for clamp(x) -> (out) ensures (out <= 10) {{}}
                unit() enter = fn {{
                    u8 result = clamp({value});
                    if (result == 0) {{}};
                    core.require(result <= {limit}, {OUTPUT});
                }};
                enter();
            %%end"#);
            let identity = source("src/main.w", text_revision);
            PreparedProofModule::prepare(identity, text, inputs()).unwrap()
        };
        let prepared = prepare(&file.revision, 7, "7");
        assert!(matches!(prepared.check("bounded"), Ok((_, ProjectCheck::Proved))));
        assert_eq!(check_package_roots(&prepared.prepared), Ok(()));
        let failed = prepare("actual-return-fail", 6, "7");
        let Err(ProjectCheck::Failed { source, state, .. }) = check_package_roots(&failed.prepared) else {
            panic!("expected an authored downstream failure from actual returned value");
        };
        assert_eq!(source, failed.source);
        assert!(state.includes(&[7]));
        assert!(!state.includes(&[10]));
        let unsupported = prepare("actual-return-unsupported", 10, "3 + 4");
        assert_eq!(check_package_roots(&unsupported.prepared), Err(ProjectCheck::Unsupported));
    }

    #[test]
    fn parsed_where_types_in_isolated_proof_scope() {
        for condition in ["true", "false", "x <= 10 && x >= 3", "x", "out <= 10", "unknown <= 10", "identity(x) <= 10"] {
            let text = format!(r#"%%start
                u8(u8) identity = fn(value) {{ value }};
                proof bounded for identity(x) -> (out) where ({condition}) ensures (out <= 10) {{}}
            %%end"#);
            let revision = source("src/main.w", condition);
            let prepared = PreparedProofModule::prepare(revision.clone(), text.clone(), inputs()).unwrap();
            match condition {
                "true" => {
                    assert_eq!(prepared.type_where("bounded"), Ok(()));
                    assert!(matches!(prepared.check("bounded"), Ok((_, ProjectCheck::Refuted))));
                }
                "false" => {
                    assert_eq!(prepared.type_where("bounded"), Ok(()));
                    assert!(matches!(prepared.check("bounded"), Ok((_, ProjectCheck::NoApplicableInputs))));
                }
                "x <= 10 && x >= 3" => {
                    assert_eq!(prepared.type_where("bounded"), Ok(()));
                    assert!(matches!(prepared.check("bounded"), Ok((_, ProjectCheck::Proved))));
                    assert_eq!(prepared.verify("bounded").unwrap().bound.guards.len(), 2);
                }
                "identity(x) <= 10" => assert_eq!(prepared.type_where("bounded"), Err(ProjectCheck::Unsupported)),
                _ => {
                    let Err(ProjectCheck::Invalid(diagnostics)) = prepared.type_where("bounded") else {
                        panic!("expected ordinary typing diagnostics for {condition}");
                    };
                    let start = text.find(&format!("where ({condition})")).unwrap() + "where (".len();
                    assert!(diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels).any(|label| {
                        label.span.source == revision && label.span.range.start == start as u32
                    }));
                }
            }
        }
    }

    #[test]
    fn conjunctive_where_narrows_all_axes_before_issuing_a_summary() {
        for (guard, expected) in [
            ("lo <= 10 && hi >= 7 && hi <= 10", ProjectCheck::Proved),
            ("lo <= 10 && hi >= 7", ProjectCheck::Refuted),
            ("hi >= 7 && hi <= 10", ProjectCheck::Refuted),
            ("hi <= 10 && (hi >= 11 && lo <= 10)", ProjectCheck::NoApplicableInputs),
            ("false && hi >= 7", ProjectCheck::NoApplicableInputs),
            ("false && (hi >= 7 || hi <= 10)", ProjectCheck::Unsupported),
            ("hi >= lo && hi <= 10", ProjectCheck::Unsupported),
        ] {
            let text = format!(r#"%%start
                u8(u8, u8) second = fn(left, right) {{ if (left <= 10) {{ right }} else {{ left }} }};
                proof bounded for second(lo, hi) -> (out) where ({guard}) ensures (out <= 10) {{}}
            %%end"#);
            let revision = source("src/main.w", guard);
            let prepared = PreparedProofModule::prepare(revision, text, inputs()).unwrap();
            assert_eq!(prepared.type_where("bounded"), Ok(()));
            assert_eq!(prepared.check("bounded").map(|(_, outcome)| outcome).unwrap_or_else(|error| error), expected);
            let summary = prepared.verify("bounded");
            if expected == ProjectCheck::Proved {
                let guards = summary.unwrap().bound.guards;
                assert!(matches!(guards.as_slice(), [
                    VerifiedEntryGuard::Integer { slot: 0, .. },
                    VerifiedEntryGuard::Integer { slot: 1, .. },
                    VerifiedEntryGuard::Integer { slot: 1, .. },
                ]));
            } else {
                assert!(summary.is_none());
            }
        }
    }

    #[test]
    fn proof_where_accepts_only_negations_that_keep_an_exact_conjunctive_scope() {
        let app = source("src/main.w", "negated-input");
        let callable = "%%start\nu8(u8) identity = fn(value) { value };\n%%end";
        for (guard, bound, expected) in [
            ("!(x == 6)", "out != 6", ProjectCheck::Proved),
            ("!(x != 6)", "out == 6", ProjectCheck::Proved),
            ("!(x < 3)", "out >= 3", ProjectCheck::Proved),
            ("!(x <= 3)", "out > 3", ProjectCheck::Proved),
            ("!(x > 10)", "out <= 10", ProjectCheck::Proved),
            ("!(x >= 10)", "out < 10", ProjectCheck::Proved),
            ("!(x >= 10)", "out < 9", ProjectCheck::Refuted),
            ("!(x < 3) && !(x > 10)", "out <= 10", ProjectCheck::Proved),
            ("!(!(x >= 3 && x <= 10))", "out <= 10", ProjectCheck::Proved),
            ("!false && x <= 10", "out <= 10", ProjectCheck::Proved),
            ("!true && x <= 10", "out <= 10", ProjectCheck::NoApplicableInputs),
            ("!(x > 3 && x < 10)", "out <= 10", ProjectCheck::Unsupported),
            ("!false && !(x > 10 || x < 3)", "out <= 10", ProjectCheck::Unsupported),
            ("!true && !(x == x)", "out <= 10", ProjectCheck::Unsupported),
            ("!true && !(x + 1 == 4)", "out <= 10", ProjectCheck::InvalidProof),
            ("!x", "out <= 10", ProjectCheck::InvalidProof),
        ] {
            assert_eq!(
                prove(&app, callable, &format!("proof bounded for identity(x) -> (out) where ({guard}) ensures ({bound}) {{}}")),
                expected,
                "guard {guard}",
            );
        }
    }

    #[test]
    fn signed_where_and_ensures_use_the_actual_integer_width_and_boundaries() {
        let app = source("src/main.w", "signed-proof-input");
        for (ty, guard, result, expected) in [
            ("i8", "x >= -128 && x <= -5", "out <= -5", ProjectCheck::Proved),
            ("i8", "x >= -128 && x <= -5", "out < -5", ProjectCheck::Refuted),
            ("i8", "x <= -128 && x > -128", "out <= -5", ProjectCheck::NoApplicableInputs),
            ("i8", "x >= -129", "out <= -5", ProjectCheck::InvalidProof),
            ("i8", "x <= -5", "out >= -129", ProjectCheck::InvalidProof),
            ("u8", "x >= -1", "out <= 5", ProjectCheck::InvalidProof),
            ("i8", "!(x < -5) && x <= -2", "out >= -5", ProjectCheck::Proved),
            ("i8", "x >= -x", "out <= -5", ProjectCheck::Unsupported),
            ("i64", "x == -9223372036854775808", "out == -9223372036854775808", ProjectCheck::Proved),
            ("i128", "x == -170141183460469231731687303715884105728",
                "out == -170141183460469231731687303715884105728", ProjectCheck::Proved),
        ] {
            let callable = format!("%%start\n{ty}({ty}) identity = fn(value) {{ value }};\n%%end");
            let proof = format!("proof bounded for identity(x) -> (out) where ({guard}) ensures ({result}) {{}}");
            assert_eq!(prove(&app, &callable, &proof), expected, "{ty} {guard} {result}");
        }
    }

    #[test]
    fn canonical_empty_clamp_proof_binds_aliases_by_position() {
        let app = source("src/main.w", "clamp");
        let callable = "%%start\nu8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };\n%%end";
        let proof = |clause: &str| format!("proof clamp_bounded for clamp(x) -> (out) {clause} {{}}");
        assert_eq!(prove(&app, callable, &proof("ensures (out <= 10)")), ProjectCheck::Proved);
        assert_eq!(prove(&app, callable, &proof("ensures (out <= 9)")), ProjectCheck::Refuted);
        assert_eq!(prove(&app, callable, &proof("ensures (out <= 256)")), ProjectCheck::InvalidProof);
        assert_eq!(
            prove(&app, callable, "proof clamp_bounded for clamp(x) -> (out) ensures (out <= 10) { core.show(out == x); }"),
            ProjectCheck::Refuted,
        );
    }

    #[test]
    fn canonical_where_uses_input_aliases_and_an_empty_filter_supplies_no_fact() {
        let app = source("src/main.w", "identity");
        let callable = "%%start\nu8(u8) identity = fn(value) { value };\n%%end";
        let proof = |clause: &str| format!("proof bounded for identity(x) -> (out) {clause} {{}}");
        for (clause, expected) in [
            ("ensures (out <= 10)", ProjectCheck::Refuted),
            ("where (x <= 10) ensures (out <= 10)", ProjectCheck::Proved),
            ("where (x > 255) ensures (out <= 10)", ProjectCheck::NoApplicableInputs),
            ("where (false) ensures (out <= 10)", ProjectCheck::NoApplicableInputs),
            ("where (x <= 256) ensures (out <= 10)", ProjectCheck::InvalidProof),
            ("where (out <= 10) ensures (out <= 10)", ProjectCheck::InvalidProof),
            ("where (unknown <= 10) ensures (out <= 10)", ProjectCheck::InvalidProof),
        ] {
            assert_eq!(prove(&app, callable, &proof(clause)), expected);
        }
    }


    #[test]
    fn proof_input_aliases_use_exact_spelling_and_canonical_positions() {
        let app = source("src/main.w", "second");
        let callable = "%%start\nu8(u8, u8) second = fn(first, second_value) { if (first == 0) {}; second_value };\n%%end";
        let proof = |filter: &str| {
            format!("proof bounded for second(x, X) -> (out) where ({filter} <= 10) ensures (out <= 10) {{}}")
        };
        assert_eq!(prove(&app, callable, &proof("X")), ProjectCheck::Proved);
        assert_eq!(prove(&app, callable, &proof("x")), ProjectCheck::Refuted);
    }

    #[test]
    fn malformed_proof_aliases_and_wrong_source_do_not_select_another_callable() {
        let app = source("src/main.w", "identity");
        let wrong = source("src/main.w", "wrong-revision");
        let callable = "%%start\nu8(u8) identity = fn(value) { value };\n%%end";
        for fragment in [
            "proof invalid for identity(x, y) -> (out) ensures (out <= 10) {}",
            "proof invalid for identity(x) -> (x) ensures (x <= 10) {}",
            "proof invalid for identity(x) -> (out) ensures (unknown <= 10) {}",
        ] {
            assert_eq!(prove(&app, callable, fragment), ProjectCheck::InvalidProof);
        }
        let fragment = wosy_syntax::parse_proof_fragment(
            source("fragment.w", "proof-r1"),
            "proof bounded for identity(x) -> (out) ensures (out <= 10) {}".into(),
            &[],
        );
        assert_eq!(
            check_proof_fragment(&prepare_proof_target(&app, callable).unwrap(), &wrong, &fragment),
            ProjectCheck::MissingRoot,
        );
    }

    #[test]
    fn documented_clamp_proof_checks_shows_against_actual_return_partitions() {
        let app = source("src/main.w", "clamp");
        let callable = "%%start\nu8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };\n%%end";
        let proof = r#"
            proof clamp_bounded for clamp(x) -> (out) ensures (out <= 10) {
                if (x > 10) { core.show(out == 10); }
                else { core.show(out == x); }
            }
        "#;
        assert_eq!(prove(&app, callable, proof), ProjectCheck::Proved);
        assert_eq!(
            prove(&app, callable, &proof.replace("x > 10", "x > 11")),
            ProjectCheck::Refuted,
        );
        assert_eq!(
            prove(&app, callable, &proof.replace("out == 10", "out == 9")),
            ProjectCheck::Refuted,
        );
        assert_eq!(
            prove(&app, callable, &proof.replace("core.show(out == x);", "core.show(out == x); core.show(out == x);")),
            ProjectCheck::Unsupported,
        );
    }

    #[test]
    fn show_alias_proof_does_not_treat_an_overwritten_input_as_its_entry_value() {
        let app = source("src/main.w", "overwrite");
        let callable = "%%start\nu8(u8) overwrite = fn(value) { if (value > 0) {}; value = 0; value };\n%%end";
        assert_eq!(
            prove(&app, callable, "proof bounded for overwrite(x) -> (out) ensures (out <= 10) {}"),
            ProjectCheck::Proved,
        );
        let proof = r#"
            proof bounded for overwrite(x) -> (out) ensures (out <= 10) {
                if (x > 10) { core.show(out == 0); }
                else { core.show(out == x); }
            }
        "#;
        assert_eq!(prove(&app, callable, proof), ProjectCheck::Unsupported);
    }

    fn verified_summary(
        program_source: &SourceIdentity,
        callable: &str,
        target_source: &SourceIdentity,
        proof_source: &SourceIdentity,
        proof: &str,
    ) -> Option<VerifiedReturnBound> {
        let fragment = wosy_syntax::parse_proof_fragment(proof_source.clone(), proof.to_owned(), &[]);
        let prepared = prepare_proof_target(program_source, callable).ok()?;
        verify_return_bound(&prepared, target_source, &fragment)
    }

    #[test]
    fn verified_bound_preserves_exact_target_and_typed_entry_guard() {
        let app = source("src/main.w", "chosen");
        let callable = "%%start\nu16(u8, u16) choose = fn(first, value) { if (first == 0) {}; value };\n%%end";
        let proof_source = source("proof/choose.fragment.w", "proof-r1");
        let proof = "proof bounded for choose(a, b) -> (out) where (b <= 12) ensures (out <= 12) {}";
        let bound = verified_summary(&app, callable, &app, &proof_source, proof).unwrap();
        assert_eq!(bound.proof_source, proof_source);
        assert_eq!(bound.proof_name, "bounded");
        assert_eq!(&proof[bound.proof_span.start as usize..bound.proof_span.end as usize], proof);
        let name_start = callable.find("choose").unwrap() as u32;
        assert_eq!(bound.target, ResolvedCallTarget::ModuleCallable {
            source: app.clone(),
            declaration_span: ByteSpan::new(name_start, name_start + "choose".len() as u32),
            concrete: ConcreteCallSelection::Declaration,
        });
        assert_eq!(bound.target_layout, ScalarTargetLayout::WASM32);
        assert_eq!(bound.parameters, [ScalarType::U8, ScalarType::U16]);
        assert_eq!(bound.output_slot, 0);
        assert_eq!(bound.output_type, ScalarType::U16);
        assert_eq!(bound.operator, BinaryOperator::LessEqual);
        assert_eq!(bound.bound, BigInt::from(12));
        assert_eq!(bound.guards, [VerifiedEntryGuard::Integer {
            slot: 1,
            ty: ScalarType::U16,
            operator: BinaryOperator::LessEqual,
            bound: BigInt::from(12),
        }]);
        let second_source = source("proof/choose.fragment.w", "proof-r2");
        let second_proof = "proof tighter for choose(a, b) -> (out) where (b <= 11) ensures (out <= 11) {}";
        let second = verified_summary(&app, callable, &app, &second_source, second_proof).unwrap();
        assert_eq!(second.target, bound.target);
        assert_ne!(second.proof_source, bound.proof_source);
        assert_eq!(second.proof_source, second_source);
        assert_eq!(second.proof_name, "tighter");
        assert_eq!(
            &second_proof[second.proof_span.start as usize..second.proof_span.end as usize],
            second_proof,
        );

        let wrong = source("src/main.w", "other-revision");
        assert_eq!(verified_summary(&app, callable, &wrong, &proof_source, proof), None);
        assert_eq!(verified_summary(&app, callable, &app, &app, proof), None);
    }

    #[test]
    fn unproved_or_inapplicable_fragment_issues_no_bound() {
        let app = source("src/main.w", "identity");
        let proof_source = source("proof/identity.fragment.w", "proof-r1");
        let callable = "%%start\nu8(u8) identity = fn(value) { value };\n%%end";
        let checked_steps = "proof bounded for identity(x) -> (out) ensures (out <= 255) { core.show(out == x); }";
        assert_eq!(prove(&app, callable, checked_steps), ProjectCheck::Proved);
        for proof in [
            "proof bounded for identity(x) -> (out) ensures (out <= 10) {}",
            "proof bounded for identity(x) -> (out) where (x > 255) ensures (out <= 10) {}",
            "proof bounded for identity(x) -> (out) where (false) ensures (out <= 10) {}",
            "proof bounded for identity(x) -> (out) ensures (out <= 256) {}",
            checked_steps,
        ] {
            assert_eq!(verified_summary(&app, callable, &app, &proof_source, proof), None);
        }
    }

    #[test]
    fn proof_success_does_not_certify_a_callable_with_directives_writes_or_calls() {
        let app = source("src/main.w", "identity");
        let proof_source = source("proof/identity.fragment.w", "proof-r2");
        let proof = "proof bounded for identity(x) -> (out) ensures (out <= 255) {}";
        for body in [
            format!("core.narrow(value <= 10, {OUTPUT}); value"),
            format!("core.require(true, {OUTPUT}); value"),
            "if (value > 0) {}; value = 0; value".into(),
            "if (false) { other(); } value".into(),
        ] {
            let callable = format!("%%start\nu8(u8) identity = fn(value) {{ {body} }}; unit() other = fn {{}};\n%%end");
            assert_eq!(prove(&app, &callable, proof), ProjectCheck::Proved);
            assert_eq!(verified_summary(&app, &callable, &app, &proof_source, proof), None);
        }
    }

    #[test]
    fn canonical_scalar_call_binding_retains_typed_literal_and_exact_target() {
        let app = source("src/main.w", "literal-call");
        for (arg, caller_input) in [("7", ""), ("input", "u8")] {
            let params = if caller_input.is_empty() { "" } else { "(input)" };
            let text = format!(
                "%%start\nu8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};\n                 u8({caller_input}) caller = fn{params} {{ u8 result = clamp({arg});                  core.require(result <= 10, {OUTPUT}); result }};\n%%end",
            );
            let prepared = prepare_checker_project(
                vec![(declarations(&app, &text), Vec::new())],
                vec![app.clone()],
                inputs(),
            ).unwrap();
            let (project, calls) = prepared.parts();
            let (module, item, _) = select_checked_root(project, &app, "caller").unwrap();
            let body = calls.function_body(module, item, None).unwrap();
            let origin = body.structure.checker_integer_call_locals.get(&(body.structure.root, 0));
            if caller_input.is_empty() {
                let origin = origin.expect("canonical one-integer call");
                let site = body.resolved_call_site(origin.call).unwrap();
                let offset = text.find("clamp").unwrap() as u32;
                assert_eq!(site.target, ResolvedCallTarget::ModuleCallable {
                    source: app.clone(),
                    declaration_span: ByteSpan::new(offset, offset + "clamp".len() as u32),
                    concrete: ConcreteCallSelection::Declaration,
                });
                assert_eq!(site.source_span.source, app);
                assert_eq!(
                    &text[site.source_span.range.start as usize..site.source_span.range.end as usize],
                    "clamp(7)",
                );
                let signature = &body.checked_call_signatures[&origin.call];
                assert_eq!(signature.parameters, [ScalarType::U8]);
                assert_eq!(signature.outputs.outputs[0].ty, ScalarType::U8);
                let [(ordinal, argument)] = origin.arguments.as_slice() else { panic!("expected one canonical argument"); };
                assert_eq!(body.structure.checked_types.expressions[ordinal], ScalarType::U8);
                assert_eq!(*argument, super::super::reference::CheckerIntegerCallArgument::Literal(BigInt::from(7)));
                let directive = body.structure.checker_directives.values().next().unwrap();
                let Some(CheckerPredicate::Integer(requirement)) = &directive.predicate else {
                    panic!("expected a typed integer requirement");
                };
                assert_eq!(requirement.binding, origin.binding);
            } else {
                let origin = origin.expect("canonical plain-name argument");
                let [(_, argument)] = origin.arguments.as_slice() else { panic!("expected one canonical argument"); };
                assert_eq!(*argument, super::super::reference::CheckerIntegerCallArgument::PlainName);
            }
            assert_eq!(check_integer_requirements(body), None);
        }
    }

    #[test]
    fn compound_bound_requires_every_guard_at_the_actual_incoming_argument() {
        let app = source("src/main.w", "conjunctive-consumer");
        let proof_source = source("proof/identity.fragment.w", "compound-r1");
        let proof = "proof bounded for identity(x) -> (out) where (!(x > 10) && !(x < 3) && !(x == 6)) ensures (out <= 10) {}";
        let text = format!(r#"%%start
            u8(u8) identity = fn(value) {{ value }};
            u8() accepted = fn {{ u8 result = identity(7); core.require(result <= 10, {OUTPUT}); result }};
            u8() below = fn {{ u8 result = identity(2); core.require(result <= 10, {OUTPUT}); result }};
            u8() above = fn {{ u8 result = identity(11); core.require(result <= 10, {OUTPUT}); result }};
            u8() hole = fn {{ u8 incoming = 6; u8 result = identity(incoming); core.require(result <= 10, {OUTPUT}); result }};
            u8() accepted_local = fn {{ u8 incoming = 10; u8 result = identity(incoming); core.require(result <= 10, {OUTPUT}); result }};
            u8(u8) broad = fn(incoming) {{ u8 result = identity(incoming); core.require(result <= 10, {OUTPUT}); result }};
        %%end"#);
        let bound = verified_summary(&app, &text, &app, &proof_source, proof).unwrap();
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &text), Vec::new())],
            vec![app.clone()],
            inputs(),
        ).unwrap();
        let identity = (&proof_source, bound.proof_span, "bounded");
        for name in ["accepted", "accepted_local"] {
            assert_eq!(use_verified_return_bound(&prepared, &app, name, identity, &bound), Some(ImportedRequirement::Proved));
        }
        for name in ["below", "above", "hole", "broad"] {
            assert_eq!(use_verified_return_bound(&prepared, &app, name, identity, &bound), None);
        }
    }

    #[test]
    fn verified_bound_can_establish_a_later_requirement_but_not_an_authored_failure() {
        let app = source("src/main.w", "consumer");
        let proof_source = source("proof/clamp.fragment.w", "proof-r1");
        let proof = "proof bounded for clamp(x) -> (out) where (x <= 10) ensures (out <= 10) {}";
        let text = format!(r#"%%start
            u8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};
            u8(u8) other = fn(value) {{ value }};
            u8() accepted = fn {{ u8 result = clamp(7); core.require(result <= 10, {OUTPUT}); result }};
            u8() insufficient = fn {{ u8 result = clamp(7); core.require(result <= 8, {OUTPUT}); result }};
            u8() lexical_inside = fn {{ u8 input = 7; u8 result = clamp(input); core.require(result <= 10, {OUTPUT}); result }};
            u8() lexical_outside = fn {{ u8 input = 11; u8 result = clamp(input); core.require(result <= 10, {OUTPUT}); result }};
            u8() outside = fn {{ u8 result = clamp(11); core.require(result <= 10, {OUTPUT}); result }};
            u8() different = fn {{ u8 result = other(7); core.require(result <= 10, {OUTPUT}); result }};
            u8(u8) dynamic = fn(value) {{ u8 result = clamp(value); core.require(result <= 10, {OUTPUT}); result }};
            u8(u8) wrong_binding = fn(value) {{ u8 result = clamp(7); core.require(value <= 10, {OUTPUT}); result }};
        %%end"#);
        let verified = verified_summary(&app, &text, &app, &proof_source, proof).unwrap();
        let prepared = prepare_checker_project(
            vec![(declarations(&app, &text), Vec::new())],
            vec![app.clone()],
            inputs(),
        ).unwrap();
        let identity = (&proof_source, verified.proof_span, "bounded");
        let use_at = |name| use_verified_return_bound(&prepared, &app, name, identity, &verified);
        assert_eq!(use_at("accepted"), Some(ImportedRequirement::Proved));
        assert_eq!(use_at("lexical_inside"), Some(ImportedRequirement::Proved));
        let wide_proof = "proof wide for clamp(x) -> (out) where (x <= 255) ensures (out <= 10) {}";
        let wide_source = source("proof/clamp.fragment.w", "proof-wide-r1");
        let wide = verified_summary(&app, &text, &app, &wide_source, wide_proof).unwrap();
        assert_eq!(
            use_verified_return_bound(&prepared, &app, "dynamic", (&wide_source, wide.proof_span, "wide"), &wide),
            Some(ImportedRequirement::Proved),
        );
        assert_eq!(use_at("insufficient"), Some(ImportedRequirement::NotEstablished));
        for name in ["outside", "lexical_outside", "different", "dynamic", "wrong_binding", "clamp", "nonexistent"] {
            assert_eq!(use_at(name), None);
        }
        let different_source = source("proof/clamp.fragment.w", "proof-r2");
        for wrong in [
            (&different_source, verified.proof_span, "bounded"),
            (&proof_source, ByteSpan::new(0, 1), "bounded"),
            (&proof_source, verified.proof_span, "different"),
        ] {
            assert_eq!(use_verified_return_bound(&prepared, &app, "accepted", wrong, &verified), None);
        }
        let changed = source("src/main.w", "consumer-r2");
        let changed_prepared = prepare_checker_project(
            vec![(declarations(&changed, &text), Vec::new())],
            vec![changed.clone()],
            inputs(),
        ).unwrap();
        assert_eq!(
            use_verified_return_bound(&changed_prepared, &changed, "accepted", identity, &verified),
            None,
        );
    }

}
