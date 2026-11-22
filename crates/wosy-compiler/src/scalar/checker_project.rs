use std::{borrow::Cow, cell::RefCell, rc::Rc, collections::{HashMap, HashSet}};

use num_bigint::BigInt;
#[cfg(feature = "internal-checker")]
use std::hash::{Hash, Hasher};

use super::{
    checker_directive::{CheckedLiteralOutput, CheckerOperation, InitializerIntegerOperand, InitializerPredicate, PreparedCheckerDirective},
    checker_flow::{
        check_integer_requirement_activity, check_integer_requirements_with_calls, collect_integer_requirement_failures, check_integer_return_bound_with_shows, collect_integer_requirement_failures_with_calls, concrete_integer_literal, exact_integer_call_argument, exact_integer_call_argument_at,
        exact_integer_return_at_entry, ActualCallResult, CheckerPredicate, ExactIntegerArgument, IntegerRequirementContinuation, IntegerRequirementEntry, NestedIntegerArgument, ProjectedIntegerArgument,
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
use super::{checker_flow::{holds_actual_mixed_entry, evaluate_borrowed_return_proof, evaluate_mixed_entry_output, observe_authored_show_returns, integer_predicate_implies, observe_arbitrary_integer_returns, observe_unused_module_proof_returns, ActualFlatShow, GuardedOutputClaims, ImmutableEntryRange, ImmutableProofOutputComparison, IntegerShowOutput}, symbolic_integer::{IntegerShowConstraints, OwnedIntegerUnion}, typecheck::integer_bounds};
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
    axes: Vec<(ReferenceBindingId, ScalarType)>,
    state: IntegerInputs,
}

#[cfg(feature = "internal-checker")]
impl ScalarCheckerIntegerWitness {
    pub fn axes(&self) -> impl ExactSizeIterator<Item = (&ReferenceBindingId, &ScalarType)> {
        self.axes.iter().map(|(binding, ty)| (binding, ty))
    }

    pub fn row_count(&self) -> usize {
        self.state.row_count()
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
        let root = &initialization_order[0];
        fn logical_source(source: &SourceIdentity) -> (&str, &str, &str) {
            (&source.project, &source.package, &source.path)
        }
        let mut startup_keys = HashSet::new();
        if !declarations.iter().all(|(source, _)| {
            raw_sources.contains(source.source()) && startup_keys.insert(logical_source(source.source()))
        }) {
            return Err(ProjectCheck::Unsupported);
        }
        let mut dormant_keys = HashSet::new();
        for source in &dormant {
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
        declarations.extend(dormant.into_iter().map(|source| (source, Vec::new())));
    }

    Ok((prepare_checker_project(declarations, initialization_order, inputs)?, startup_count))
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
        witness: ScalarCheckerIntegerWitness { axes, state },
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

fn check_package_roots_with_activity(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<bool, ProjectCheck> {
    check_package_roots_with_policy::<false>(prepared, scope, startup_count, None)
}

#[cfg(feature = "internal-checker")]
fn collect_ordered_checker_records(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<(bool, Vec<ScalarOrderedCheckerFailure>), ProjectCheck> {
    let mut failures = OrderedFailures::default();
    let active = check_package_roots_with_policy::<true>(prepared, scope, startup_count, Some(&mut failures))?;
    Ok((active, failures.records))
}

#[cfg(all(test, feature = "internal-checker"))]
fn collect_ordered_package_failures(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<(bool, Vec<ProjectCheck>), ProjectCheck> {
    let (active, failures) = collect_ordered_checker_records(prepared, scope, startup_count)?;
    Ok((active, failures.into_iter().map(ordered_failure_as_legacy).collect()))
}

fn check_package_roots_with_policy<const COLLECT: bool>(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
    mut failures: Option<&mut OrderedFailures>,
) -> Result<bool, ProjectCheck> {
    let (project, calls) = prepared.parts();
    let (startup, dormant) = project.modules.split_at(startup_count);
    let (root, ordinals) = package_startup_order(project, startup)?;
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
    if !potential_obligations {
        return Ok(false);
    }
    let (CheckedInitialization { visits, reached, ordinals, mut had_obligations }, ()) =
        check_package_initialization_observing_with_policy::<(), COLLECT>(
            prepared, root, ordinals, (), failures.as_deref_mut(),
        )?;
    if scope == ScalarCheckerScope::MainFlowOnly {
        return Ok(had_obligations);
    }
    let mut ordered_roots = if COLLECT {
        close_ordered_symbolic_roots(std::mem::take(&mut authored_roots), || {
            ordered_symbolic_callers(project, calls, root)
        })?
    } else {
        HashMap::new()
    };
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
            had_obligations |= check_unused_module::<COLLECT>(
                project, calls, module, ordinal, &reached, selected, failures.as_deref_mut(), Some(&visits),
            )?;
        }
    }
    for (index, module) in dormant.iter().enumerate() {
        let ordinal = startup_count + index;
        if let Some(selected) = module_roots(ordinal, module)? {
            had_obligations |= check_unused_module::<COLLECT>(
                project, calls, module, ordinal, &reached, selected, failures.as_deref_mut(), None,
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
) -> Result<HashMap<PreparedCallable, Vec<PreparedCallable>>, ProjectCheck> {
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

fn check_unused_module<const COLLECT: bool>(
    project: &ScalarProject,
    calls: &ProjectCallSiteResults,
    module: &ScalarModule,
    ordinal: usize,
    reached: &HashSet<(usize, usize)>,
    symbolic_roots: HashSet<usize>,
    mut failures: Option<&mut OrderedFailures>,
    completed_visits: Option<&[InitializationVisit]>,
) -> Result<bool, ProjectCheck> {
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
            if result.ty == ScalarType::Unit || is_integer_type(&result.ty))
        {
            return Err(ProjectCheck::Unsupported);
        }
        if body.structure.checker_direct_targets.is_empty() {
            if COLLECT {
                let (observed, selected) = collect_integer_requirement_failures(body).ok_or(ProjectCheck::Unsupported)?;
                active |= observed;
                let failures = failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?;
                for failure in selected {
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
                project, calls, active: HashSet::from([(ordinal, item)]), reached: HashSet::new(),
                had_obligations: false, observer: (), symbolic_module: ordinal,
            };
            if COLLECT {
                let sink = RefCell::new(failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?);
                let collection = OrderedCallCollection { failures: &sink, visits: completed_visits };
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
    check_package_initialization_observing_with_policy::<(), true>(
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
    check_package_initialization_observing_with_policy::<Observer, false>(prepared, root, ordinals, observer, None)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartupLane {
    Feasible,
    Empty,
}

fn check_package_initialization_observing_with_policy<'a, Observer: ActualIntegerObserver, const COLLECT: bool>(
    prepared: &'a PreparedCheckerProject,
    root: &'a SourceIdentity,
    ordinals: HashMap<&'a SourceIdentity, usize>,
    observer: Observer,
    mut failures: Option<&mut OrderedFailures>,
) -> Result<(CheckedInitialization<'a>, Observer), ProjectCheck> {
    let (project, calls) = prepared.parts();
    let mut visits = vec![InitializationVisit::Unseen; project.modules.len()];
    let mut actual = CheckedCalls {
        project, calls, active: HashSet::new(), reached: HashSet::new(),
        had_obligations: false, observer, symbolic_module: 0,
    };
    walk_package_initialization::<COLLECT>(project, calls, &ordinals, &mut visits, &mut actual, root, &mut failures)?;
    Ok((CheckedInitialization { visits, reached: actual.reached, ordinals, had_obligations: actual.had_obligations }, actual.observer))
}

fn walk_package_initialization<const COLLECT: bool>(
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
            if walk_package_initialization::<COLLECT>(project, calls, ordinals, visits, actual, target, failures)? == StartupLane::Empty {
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
                        let ActualCallResult::Integer(value) = run_initializer_call(
                            topology, actual, outputs.primary, ty,
                            |argument, argument_type| initialized_integer_argument(topology, &storage, argument, argument_type),
                            None,
                        )? else {
                            return Err(ProjectCheck::Unsupported);
                        };
                        Cow::Owned(value)
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
        if check_initializer_expression::<COLLECT>(topology, actual, source, &storage, *expression, 0, failures, visits)? == StartupLane::Empty {
            return Ok(StartupLane::Empty);
        }
    }
    visits[ordinal] = InitializationVisit::Completed;
    Ok(StartupLane::Feasible)
}

fn check_initializer_expression<const COLLECT: bool>(
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
                .push(source, directive, span, IntegerInputs::closed(), Vec::new());
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
                if check_initializer_expression::<COLLECT>(topology, actual, source, storage, expression, depth + 1, failures, visits)?
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
        Some((source, OrderedCallCollection { failures: &sink, visits: Some(visits) }))
    } else { None };
    match run_initializer_call(topology, actual, expression, &ScalarType::Unit, |argument, ty| {
        if COLLECT { initialized_integer_argument(topology, storage, argument, ty) } else { None }
    }, collection)? {
        ActualCallResult::Unit => Ok(StartupLane::Feasible),
        ActualCallResult::EmptyUnit if COLLECT => Ok(StartupLane::Empty),
        _ => Err(ProjectCheck::Unsupported),
    }
}

fn run_initializer_call<'a>(
    topology: &'a ModuleInitializationTopology,
    actual: &mut CheckedCalls<'_, impl ActualIntegerObserver>,
    occurrence: OccurrenceOrdinal,
    expected: &ScalarType,
    initialized_argument: impl Fn(OccurrenceOrdinal, &ScalarType) -> Option<&'a BigInt>,
    collection: Option<(&SourceIdentity, OrderedCallCollection<'_, '_>)>,
) -> Result<ActualCallResult, ProjectCheck> {
    let Some(ModuleInitializationCallTarget::Direct {
        declaration,
        concrete: ConcreteCallSelection::Declaration,
    }) = topology.call_targets.get(&occurrence) else {
        return Err(ProjectCheck::Unsupported);
    };
    let signature = topology.checked_call_signatures.get(&occurrence).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let arguments = if signature.parameters.len() == 2
        && (is_integer_type(expected) || (*expected == ScalarType::Unit && collection.is_some()))
    {
        let first = exact_integer_call_argument_at(&topology.structure, occurrence, 0, &initialized_argument)
            .ok_or(ProjectCheck::Unsupported)?;
        let second = exact_integer_call_argument_at(&topology.structure, occurrence, 1, &initialized_argument)
            .ok_or(ProjectCheck::Unsupported)?;
        ActualProjectedArguments::ExactPair(first, second)
    } else {
        let argument = exact_integer_call_argument(&topology.structure, occurrence, initialized_argument);
        if *expected != ScalarType::Unit && argument.is_none() {
            return Err(ProjectCheck::Unsupported);
        }
        ActualProjectedArguments::Single(argument)
    };
    actual.run(&topology.structure, occurrence, signature, arguments, &declaration.source, declaration.range, expected, collection)
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
}

enum ActualProjectedArguments<'a> {
    Single(Option<ExactIntegerArgument<'a>>),
    ExactPair(ExactIntegerArgument<'a>, ExactIntegerArgument<'a>),
    SymbolicAxis(ProjectedIntegerArgument<'a>),
}

struct CheckedCalls<'a, Observer, const ACTUAL_STARTUP: bool = true> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    active: HashSet<(usize, usize)>,
    reached: HashSet<(usize, usize)>,
    had_obligations: bool,
    observer: Observer,
    symbolic_module: usize,
}

trait ActualIntegerObserver {
    const ENABLED: bool;

    fn observe(&mut self, target: (usize, usize), arguments: &[&BigInt], value: &BigInt);

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

const MAX_ACTUAL_CALL_DEPTH: usize = 64;

impl<Observer: ActualIntegerObserver, const ACTUAL_STARTUP: bool> CheckedCalls<'_, Observer, ACTUAL_STARTUP> {
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
        if collection.is_some() && *expected != ScalarType::Unit {
            return Err(ProjectCheck::Unsupported);
        }
        let projected = match argument {
            NestedIntegerArgument::Single(exact) => ActualProjectedArguments::Single(exact),
            NestedIntegerArgument::Pair(first, second) => ActualProjectedArguments::ExactPair(first, second),
            NestedIntegerArgument::Projected(projection) => ActualProjectedArguments::SymbolicAxis(projection),
        };
        self.run(&caller.structure, call, signature, projected, source, *declaration_span, expected, collection)
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
            body, entry, handler, !ACTUAL_STARTUP && !terminal_projection, &mut sink,
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
    ) -> Result<ActualCallResult, ProjectCheck> {
        let &(target_module, target_item) = structure.checker_direct_targets.get(&call)
            .ok_or(ProjectCheck::MissingRootOccurrences)?;
        if !ACTUAL_STARTUP && (*expected != ScalarType::Unit
            || (collection.is_none() && target_module != self.symbolic_module))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let owner = self.project.modules.get(target_module).ok_or(ProjectCheck::MissingRoot)?;
        let Some(ScalarItem::Function(function)) = owner.items.get(target_item) else {
            return Err(ProjectCheck::MissingRoot);
        };
        if &owner.source != target_source || function.name_span != target_span {
            return Err(ProjectCheck::MissingRoot);
        }
        if !matches!(signature.outputs.outputs.as_slice(), [output] if &output.ty == expected) {
            return Err(ProjectCheck::Unsupported);
        }
        let body = self.calls.function_body(target_module, target_item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let collection = if let Some((caller_source, collection)) = collection {
            if caller_source != &owner.source
                && (*expected != ScalarType::Unit || matches!(&projected, ActualProjectedArguments::SymbolicAxis(..))
                    || caller_source.project != owner.source.project || caller_source.package != owner.source.package
                    || collection.visits.and_then(|visits| visits.get(target_module)) != Some(&InitializationVisit::Completed))
            {
                return Err(ProjectCheck::Unsupported);
            }
            Some(collection)
        } else {
            None
        };
        let target = (target_module, target_item);
        let projected = match projected {
            ActualProjectedArguments::SymbolicAxis(projection) => {
                let Some(collection) = collection else { return Err(ProjectCheck::Unsupported); };
                let ProjectedIntegerArgument { ty, slot, fixed, .. } = projection;
                let types_match = match (signature.parameters.as_slice(), body.boundary.parameters.as_slice(), fixed) {
                    ([parameter], [entry], None) => slot == 0 && parameter == ty && entry.ty == *ty,
                    (parameters @ [_, _], entries @ [_, _], Some((fixed_ty, _))) if slot < 2 => {
                        parameters[slot] == *ty && entries[slot].ty == *ty
                            && parameters[1 - slot] == *fixed_ty && entries[1 - slot].ty == *fixed_ty
                    }
                    _ => false,
                };
                if ACTUAL_STARTUP || *expected != ScalarType::Unit || !types_match
                    || !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
                    || !body.structure.checker_direct_targets.is_empty()
                    || !body.call_occurrences.is_empty() || !body.allocation_length_calls.is_empty()
                    || !body.assignments.is_empty() || !body.arms.is_empty()
                    || body.structure.checker_directives.values().any(|directive| directive.call_signature.is_some())
                    || self.active.len() == MAX_ACTUAL_CALL_DEPTH || self.active.contains(&target)
                {
                    return Err(ProjectCheck::Unsupported);
                }
                let values = projection.materialize().ok_or(ProjectCheck::Unsupported)?;
                let outcome = self.collect_requirements(
                    body, &owner.source, IntegerRequirementEntry::Projected {
                        slot, values: Rc::clone(&values), fixed: fixed.map(|(_, value)| value),
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
        if self.active.len() == MAX_ACTUAL_CALL_DEPTH {
            return Err(ProjectCheck::Unsupported);
        }
        if !self.active.insert(target) {
            return Err(ProjectCheck::Unsupported);
        }
        let result = if *expected == ScalarType::Unit {
            if let Some(collection) = collection {
                self.collect_requirements(body, &owner.source, IntegerRequirementEntry::Exact(exact), collection)
            } else {
                self.requirements(body, &owner.source, Some(exact)).map(|active| {
                    self.had_obligations |= active;
                    ActualCallResult::Unit
                })
            }
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
            let proved = if ACTUAL_STARTUP && Observer::ENABLED {
                self.observer.proved_recursive_return(target, exact, expected)
            } else {
                None
            };
            #[cfg(not(feature = "internal-checker"))]
            let proved = None;
            proved.map(ActualCallResult::Integer).ok_or(ProjectCheck::Unsupported)
        };
        self.active.remove(&target);
        if ACTUAL_STARTUP && Observer::ENABLED {
            if let Ok(ActualCallResult::Integer(value)) = &result {
                self.observer.observe(target, exact, value);
            }
        }
        if ACTUAL_STARTUP && result.is_ok() {
            self.reached.insert(target);
        }
        result
    }
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
    ) {
        self.records.push(ScalarOrderedCheckerFailure {
            source: source.clone(),
            operation: directive.operation.into(),
            directive: span,
            condition: directive.condition,
            witness: ScalarCheckerIntegerWitness { axes, state },
            output: self.payloads.shared(directive),
        });
    }

    fn push_callable(&mut self, body: &FunctionCallSites, source: &SourceIdentity, failure: IntegerCheckFailure<'_>) {
        self.push(
            source,
            &body.structure.checker_directives[&failure.directive],
            body.occurrences[failure.directive.0].span,
            failure.state,
            failure.axes.into_iter().map(|(binding, ty)| (binding.clone(), ty.clone())).collect(),
        );
    }
}

#[cfg(all(test, feature = "internal-checker"))]
fn ordered_failure_as_legacy(failure: ScalarOrderedCheckerFailure) -> ProjectCheck {
    let ScalarOrderedCheckerFailure {
        source, operation, directive, condition, witness: ScalarCheckerIntegerWitness { axes, state }, output,
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
        Self::prepare_declarations(source, declarations, proofs, initialization_order, inputs)
    }

    fn prepare_declarations(
        source: SourceIdentity,
        declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        catalogs: Vec<ModuleProofCatalog>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
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
        let mut proofs = HashMap::new();
        for catalog in catalogs {
            if !graph_sources.contains_key(&catalog.source) {
                return Err(ProjectCheck::RootSourceOutsideInitOrder);
            }
            if proofs.insert(catalog.source, ModuleProofs { definitions: catalog.definitions, first_query: 0 }).is_some() {
                return Err(ProjectCheck::AmbiguousRoot);
            }
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
    let prepared = prepare_loaded_proof_project(declarations, proofs, initialization_order, inputs)
        .map_err(|error| module_proof_outcome(Err(error)))?;
    Ok(prepared.check_module_proof_selections(selections))
}

#[cfg(feature = "internal-checker")]
fn prepare_loaded_proof_project(
    mut declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    proofs: Vec<(ScalarProofDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
) -> Result<PreparedProofModule, ProjectCheck> {
    let mut catalogs = Vec::with_capacity(proofs.len());
    declarations.reserve(proofs.len());
    for (proof, namespaces) in proofs {
        let ScalarProofDeclarations { declarations: proof_declarations, definitions } = proof;
        catalogs.push(ModuleProofCatalog { source: proof_declarations.source().clone(), definitions });
        declarations.push((proof_declarations, namespaces));
    }
    let root = initialization_order.first().cloned().ok_or(ProjectCheck::MissingRoot)?;
    PreparedProofModule::prepare_declarations(root, declarations, catalogs, initialization_order, inputs)
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
    let result = prepare_loaded_proof_project(declarations, vec![catalog], initialization_order, inputs)
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

    fn matching_call(
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
            ALLOW_DOMAIN && steps.items.is_empty() && typed.measure.is_none(),
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
        let filter = prepare_proof_entry_filter(
            proof.node, proof_source, typed.target.target_layout, &typed.input_names, typed.body,
        )?;
        let proof_case = prepare_decrement_proof_steps(proof_name, &steps, &typed, measure)?;
        check_decrement_induction_premise(measure, &proof_case.base_guard, filter.as_deref().unwrap_or_default())
    }

    #[cfg(test)]
    fn audit_induction_return_witness(
        &self, proof_source: &SourceIdentity, proof_name: &str,
    ) -> Result<InductionPremise, ProjectCheck> {
        let plan = self.plan_module_proof_in_scope::<true, false, false>(proof_source, proof_name)?;
        let measure = plan.typed.measure.ok_or(ProjectCheck::Unsupported)?;
        Ok(plan.verify_induction_return_witness(measure)?.unwrap_or(InductionPremise::Unreachable))
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
        if !pending_peers.is_empty() {
            for (_, plan, _, _) in &pending_peers {
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
                } else if claim.is_none() && immediate.steps.items.is_empty() {
                    let Some(entry) = plan.closed_entry_admitted_by_peer(immediate) else { continue; };
                    Some(entry)
                } else {
                    continue;
                };
                graph.activate(requested);
                admitted_roots.push((index, plan, requested, strict_entry, claim));
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
                let mixed_implies_claims = claim.map(|(show_relation, show_bound)| {
                    let TypedProofGoal::Literal { relation: requested_relation, bound: requested, .. } =
                        &plan.typed.goal else { return false; };
                    let show = (show_relation, show_bound.as_ref());
                    let ensures = (*requested_relation, requested);
                    let premise = (relation, bound);
                    cited_bounds.implies(plan.typed.result_type, premise, show)
                        && (show == ensures || cited_bounds.implies(plan.typed.result_type, premise, ensures))
                });
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
                                if let Some(implied) = mixed_implies_claims {
                                    if implied {
                                        outcomes[index] = ScalarModuleProofOutcome::Proved;
                                    }
                                    continue;
                                }
                                let TypedProofGoal::Literal { relation: requested_relation, bound: requested, .. } =
                                    &consumer.typed.goal else { continue; };
                                if relation != equality && !implication_checked {
                                    let premise = (relation, bound);
                                    let conclusion = (*requested_relation, requested);
                                    let implied = cited_bounds.implies(consumer.typed.result_type, premise, conclusion);
                                    if !implied {
                                        continue;
                                    }
                                }
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
                                outcomes[index] = if relation == equality && !requested_relation.accepts(bound.cmp(requested)) {
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
    ) -> Result<Option<InductionPremise>, ProjectCheck> {
        let proof_case = prepare_decrement_proof_steps(self.name, &self.steps, &self.typed, measure)?;
        let base_matches = check_canonical_induction_returns(
            self.function, self.canonical_target, &self.typed, measure, &proof_case,
        )?;
        let filter = self.filter.as_deref().unwrap_or_default();
        let (entries, minimum) = induction_entry_domain(measure, filter)?;
        if entries.is_empty() {
            return Ok(None);
        }
        if !base_matches && induction_base_reachable(&entries, &proof_case.base_guard)? {
            return Err(ProjectCheck::Refuted);
        }
        check_induction_entry_domain(entries, &minimum, &proof_case.base_guard, filter, measure.descending.1).map(Some)
    }

    fn verify_unused_induction(&self, measure: TypedInductionRank<'_>) -> Result<ProjectCheck, ProjectCheck> {
        match self.verify_induction_return_witness(measure) {
            Ok(None) => Ok(ProjectCheck::NoApplicableInputs),
            Ok(Some(InductionPremise::Established | InductionPremise::Unreachable)) => Ok(ProjectCheck::Proved),
            Ok(Some(InductionPremise::NotStrict | InductionPremise::CalleeWhereUnproved)) => Err(ProjectCheck::Unsupported),
            Err(ProjectCheck::Refuted) => Ok(ProjectCheck::Refuted),
            Err(error) => Err(error),
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

    fn citation_then_show(&self) -> Result<Option<(&str, (IntegerRelation, Cow<'_, BigInt>))>, ProjectCheck> {
        let [use_step, show_step] = self.steps.items.as_slice() else { return Ok(None); };
        if self.steps.terminated_items.as_slice() != [true, true] {
            return Ok(None);
        }
        let Some(peer) = self.peer_use_step(use_step) else { return Ok(None); };
        let claim = proof_show_expression(show_step, true)?;
        let comparison = IntegerComparison::from_expression(claim).ok_or(ProjectCheck::Unsupported)?;
        let (_, relation, bound) = checked_literal_output_goal_borrowed(
            comparison, &self.typed.output, &self.typed.input_names, self.typed.result_type,
        )?;
        Ok(Some((peer, (relation, bound))))
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
                where_guard: checked_integer_guard_slots::<false>(plan.typed.body, plan.filter.as_deref().unwrap_or_default())?,
                used: false,
            })
        })();
        (checked, token)
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
        let token = self.induction.as_mut()?;
        if token.target != target || token.result_type != expected
            || !token.where_guard.accepts(arguments)
        {
            return None;
        }
        token.used = true;
        Some(token.value.clone())
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
struct TypedInductionProofSteps<'a> {
    base_guard: CheckerPredicate,
    base_claim: IntegerReturn<'a>,
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
    let entry = &typed.input_names[rank.descending.0.ordinal];
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
    let argument = match (rank.earlier, arguments.as_slice()) {
        (None, [decrement]) => decrement,
        (Some((earlier, _)), [ScalarExpression::Name { name, .. }, decrement])
            if name == &typed.input_names[earlier.ordinal] => decrement,
        _ => return Err(ProjectCheck::Unsupported),
    };
    let ScalarExpression::Binary { operator: BinaryOperator::Subtract, left, right, .. } = argument else {
        return Err(ProjectCheck::Unsupported);
    };
    if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == entry)
        || !matches!(right.as_ref(), ScalarExpression::Integer { value, .. } if *value == BigInt::from(1))
    {
        return Err(ProjectCheck::Unsupported);
    }
    let base_bindings = typed.body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
    let base_claim = proof_show_claim(then_branch, &typed.input_names, &typed.output, typed.result_type, typed.body, base_bindings)?;
    let base_guard = proof_input_condition::<true>(condition, &typed.input_names, typed.body, false)?;
    if matches!(&base_guard, CheckerPredicate::Integer(predicate) if &predicate.binding != rank.descending.1) {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(TypedInductionProofSteps { base_guard, base_claim })
}

#[cfg(feature = "internal-checker")]
fn check_canonical_induction_returns(
    function: &ScalarFunction,
    target: &ResolvedCallTarget,
    typed: &TypedProofPostcondition<'_>,
    rank: TypedInductionRank<'_>,
    proof: &TypedInductionProofSteps<'_>,
) -> Result<bool, ProjectCheck> {
    use super::reference::{OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};
    use super::typecheck::expression_span;

    let (parameter, binding) = rank.descending;
    let body = typed.body;
    let structure = &body.structure;
    let TypedProofGoal::Literal { relation, bound, .. } = &typed.goal else {
        return Err(ProjectCheck::Unsupported);
    };
    let IntegerReturn::Const(claimed_base) = &proof.base_claim else {
        return Err(ProjectCheck::Unsupported);
    };
    if &parameter.ty != typed.result_type || !body.allocation_length_calls.is_empty() {
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

    let base = output_at(*then_block)
        .and_then(|occurrence| structure.checker_integer_literals.get(&occurrence))
        .ok_or(ProjectCheck::Unsupported)?;

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
        (Some((earlier, _)), [first, second]) => first == &earlier.ty && second == &parameter.ty,
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
    let (argument, earlier_argument) = match (rank.earlier, arguments.as_slice()) {
        (None, [decrement]) => (decrement, None),
        (Some(earlier), [first, decrement]) => (decrement, Some((first, earlier))),
        _ => return Err(ProjectCheck::Unsupported),
    };
    let ScalarExpression::Binary { operator: BinaryOperator::Subtract, left, right, .. }
        = argument else { return Err(ProjectCheck::Unsupported); };
    if source_outputs.next().is_some()
        || body.occurrences.get(tail.0).map(|node| node.span) != Some(source_tail.span)
        || body.occurrences.get(recursive.0).map(|node| node.span) != Some(expression_span(source_call))
        || !matches!(right.as_ref(), ScalarExpression::Integer { value, .. } if *value == BigInt::from(1))
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
    if let Some((expression, (earlier, expected))) = earlier_argument {
        let first = checked_operand(recursive, OccurrenceEdge::CallArgument(earlier.ordinal), expression, &earlier.ty)
            .ok_or(ProjectCheck::Unsupported)?;
        if !matches!(body.prepared_plain_name(first), Some(PlainNameIdentity::Lexical(entry)) if entry == expected) {
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
    Ok(base == *claimed_base && relation.accepts(base.cmp(bound)))
}

#[cfg(feature = "internal-checker")]
fn check_decrement_induction_premise(
    rank: TypedInductionRank<'_>,
    base_guard: &CheckerPredicate,
    entry_filter: &[CheckerPredicate],
) -> Result<InductionPremise, ProjectCheck> {
    let (entries, minimum) = induction_entry_domain(rank, entry_filter)?;
    check_induction_entry_domain(entries, &minimum, base_guard, entry_filter, rank.descending.1)
}

#[cfg(feature = "internal-checker")]
fn induction_entry_domain(
    rank: TypedInductionRank<'_>,
    entry_filter: &[CheckerPredicate],
) -> Result<(IntegerSet, BigInt), ProjectCheck> {
    let (parameter, binding) = rank.descending;
    let mut entries = IntegerSet::from_type(&parameter.ty).ok_or(ProjectCheck::Unsupported)?;
    let mut earlier = match rank.earlier {
        Some((slot, binding)) => Some((IntegerSet::from_type(&slot.ty).ok_or(ProjectCheck::Unsupported)?, binding)),
        None => None,
    };
    let minimum = entries.intervals().next().ok_or(ProjectCheck::Unsupported)?.0.clone();
    for guard in entry_filter {
        match guard {
            CheckerPredicate::Boolean(true) => {}
            CheckerPredicate::Boolean(false) => return Ok((IntegerSet::default(), minimum)),
            CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return Err(ProjectCheck::Unsupported),
            CheckerPredicate::Integer(predicate) if &predicate.binding == binding => {
                entries = entries.keep(&predicate.operator, &predicate.bound).ok_or(ProjectCheck::Unsupported)?;
                if entries.is_empty() {
                    break;
                }
            }
            CheckerPredicate::Integer(predicate) => {
                let (domain, earlier_binding) = earlier.as_mut().ok_or(ProjectCheck::Unsupported)?;
                if &predicate.binding != *earlier_binding {
                    return Err(ProjectCheck::Unsupported);
                }
                *domain = domain.keep(&predicate.operator, &predicate.bound).ok_or(ProjectCheck::Unsupported)?;
                if domain.is_empty() {
                    return Ok((IntegerSet::default(), minimum));
                }
            }
        }
    }
    Ok((entries, minimum))
}

#[cfg(feature = "internal-checker")]
fn induction_base_reachable(
    entries: &IntegerSet,
    guard: &CheckerPredicate,
) -> Result<bool, ProjectCheck> {
    if entries.is_empty() {
        return Ok(false);
    }
    match guard {
        CheckerPredicate::Boolean(value) => Ok(*value),
        CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => Err(ProjectCheck::Unsupported),
        CheckerPredicate::Integer(predicate) => {
            let complement = complement_integer_comparison(&predicate.operator)
                .and_then(IntegerRelation::from_operator).ok_or(ProjectCheck::Unsupported)?;
            Ok(!entries.every(complement, &predicate.bound))
        }
    }
}

#[cfg(feature = "internal-checker")]
fn check_induction_entry_domain(
    mut entries: IntegerSet,
    minimum: &BigInt,
    base_guard: &CheckerPredicate,
    entry_filter: &[CheckerPredicate],
    descending: &ReferenceBindingId,
) -> Result<InductionPremise, ProjectCheck> {
    match base_guard {
        CheckerPredicate::Boolean(true) => return Ok(InductionPremise::Unreachable),
        CheckerPredicate::Boolean(false) => {}
        CheckerPredicate::EntryDomain { .. } | CheckerPredicate::DirectiveDisjunction { .. } | CheckerPredicate::ProofReflexiveBranch(_) => return Err(ProjectCheck::Unsupported),
        CheckerPredicate::Integer(predicate) => {
            let operator = complement_integer_comparison(&predicate.operator).ok_or(ProjectCheck::Unsupported)?;
            entries = entries.keep(operator, &predicate.bound).ok_or(ProjectCheck::Unsupported)?;
        }
    }
    if entries.is_empty() {
        return Ok(InductionPremise::Unreachable);
    }
    let strict = IntegerRelation::from_operator(&BinaryOperator::Greater).expect("integer comparison");
    if !entries.every(strict, minimum) {
        return Ok(InductionPremise::NotStrict);
    }
    for guard in entry_filter {
        if let CheckerPredicate::Integer(predicate) = guard {
            if &predicate.binding != descending {
                continue;
            }
            let relation = IntegerRelation::from_operator(&predicate.operator).ok_or(ProjectCheck::Unsupported)?;
            if !entries.every(relation, &(&predicate.bound + 1)) {
                return Ok(InductionPremise::CalleeWhereUnproved);
            }
        }
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
                return Err(ProjectCheck::Unsupported);
            }
            if let Some(second) = foralls.next() {
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
    let [binding_item, requirement_item, tail @ ..] = &items[first_nonliteral..] else {
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
                project, calls, active: HashSet::new(), reached: HashSet::new(),
                had_obligations: false, observer: (), symbolic_module: 0,
            };
            let mut output = OrderedFailures::default();
            let result = check_initializer_expression::<true>(
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
