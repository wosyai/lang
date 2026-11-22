use std::{borrow::Cow, cell::RefCell, collections::{HashMap, HashSet}};

use num_bigint::BigInt;
#[cfg(feature = "internal-checker")]
use std::hash::{Hash, Hasher};

use super::{
    checker_directive::{CheckedLiteralOutput, CheckerOperation, InitializerIntegerOperand, InitializerPredicate, PreparedCheckerDirective},
    checker_flow::{
        check_integer_requirement_activity, check_integer_requirements_with_calls, collect_integer_requirement_failures, check_integer_return_bound_with_shows, collect_integer_requirement_failures_with_calls, concrete_integer_literal, exact_integer_call_argument, exact_integer_call_argument_at,
        exact_integer_return_at_entry, ActualCallResult, CheckerPredicate, ExactIntegerArgument,
        proves_requirement_with_imported_root_local, ImportedEntryGuard, ImportedLocal, IntegerShowProof, IntegerCheckFailure, IntegerComparison, IntegerInputs, IntegerReturn, ProofCoverage,
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
use super::checker_flow::observe_arbitrary_integer_returns;
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

#[cfg(feature = "internal-checker")]
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

#[cfg(feature = "internal-checker")]
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

#[cfg(feature = "internal-checker")]
#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub enum ScalarOrderedCheckerOutcome {
    Passed,
    NoChecks,
    Unsupported,
    Invalid(Vec<Diagnostic>),
    Failed(Vec<ScalarCheckerFailure>),
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
        .and_then(|(prepared, startup_count)| collect_ordered_package_failures(&prepared, scope, startup_count));
    match result {
        Ok((active, failures)) if failures.is_empty() => {
            if active { ScalarOrderedCheckerOutcome::Passed } else { ScalarOrderedCheckerOutcome::NoChecks }
        }
        Ok((_, failures)) => failures.into_iter().map(materialize_checker_failure).collect::<Option<Vec<_>>>()
            .map_or(ScalarOrderedCheckerOutcome::Unsupported, ScalarOrderedCheckerOutcome::Failed),
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
        operation: match operation {
            CheckerOperation::Require => ScalarCheckerOperation::Require,
            CheckerOperation::Narrow => ScalarCheckerOperation::Narrow,
        },
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
fn collect_ordered_package_failures(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
) -> Result<(bool, Vec<ProjectCheck>), ProjectCheck> {
    let mut failures = Vec::new();
    let active = check_package_roots_with_policy::<true>(prepared, scope, startup_count, Some(&mut failures))?;
    Ok((active, failures))
}

fn check_package_roots_with_policy<const COLLECT: bool>(
    prepared: &PreparedCheckerProject, scope: ScalarCheckerScope, startup_count: usize,
    mut failures: Option<&mut Vec<ProjectCheck>>,
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
    for source in &project.initialization_order {
        if source.project != root.project || source.package != root.package {
            continue;
        }
        let ordinal = *ordinals.get(source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
        if let Some(seeds) = authored_roots.remove(&ordinal) {
            let module = &project.modules[ordinal];
            if visits[ordinal] != InitializationVisit::Completed {
                if COLLECT && symbolic_obligation_roots(calls, module, ordinal, seeds)?
                    .into_iter().any(|item| !reached.contains(&(ordinal, item)))
                {
                    return Err(ProjectCheck::Unsupported);
                }
                continue;
            }
            had_obligations |= check_unused_module::<COLLECT>(
                project, calls, module, ordinal, &reached, seeds, failures.as_deref_mut(),
            )?;
        }
    }
    for (index, module) in dormant.iter().enumerate() {
        let ordinal = startup_count + index;
        if let Some(seeds) = authored_roots.remove(&ordinal) {
            had_obligations |= check_unused_module::<COLLECT>(
                project, calls, module, ordinal, &reached, seeds, failures.as_deref_mut(),
            )?;
        }
    }
    Ok(had_obligations)
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
    authored_roots: Vec<usize>,
    mut failures: Option<&mut Vec<ProjectCheck>>,
) -> Result<bool, ProjectCheck> {
    let symbolic_roots = symbolic_obligation_roots(calls, module, ordinal, authored_roots)?;
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
                failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?.extend(
                    selected.into_iter().map(|failure| project_failure(body, &module.source, failure)),
                );
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
                let collection = OrderedCallCollection { failures: &sink, nested_calls_remaining: 1 };
                symbolic.collect_requirements(body, &module.source, None, collection)?;
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

#[cfg(feature = "internal-checker")]
#[cfg_attr(not(test), allow(dead_code))]
fn collect_package_startup_failures(
    prepared: &PreparedCheckerProject,
) -> Result<Vec<ProjectCheck>, ProjectCheck> {
    let (root, ordinals) = package_initialization_order(prepared.parts().0)?;
    let mut failures = Vec::new();
    check_package_initialization_observing_with_policy::<(), true>(
        prepared, root, ordinals, (), Some(&mut failures),
    )?;
    Ok(failures)
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
    mut failures: Option<&mut Vec<ProjectCheck>>,
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
    failures: &mut Option<&mut Vec<ProjectCheck>>,
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
        if check_initializer_expression::<COLLECT>(topology, actual, source, &storage, *expression, 0, failures)? == StartupLane::Empty {
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
    failures: &mut Option<&mut Vec<ProjectCheck>>,
) -> Result<StartupLane, ProjectCheck> {
    let structure = &topology.structure;
    if let Some(directive) = structure.checker_directives.get(&expression) {
        let predicate = directive.initializer_predicate.as_ref().ok_or(ProjectCheck::Unsupported)?;
        let value = evaluate_initializer_predicate(predicate, storage).ok_or(ProjectCheck::Unsupported)?;
        actual.had_obligations = true;
        if directive.operation.rejects(value, !value) {
            let span = topology.occurrences.get(expression.0).ok_or(ProjectCheck::MissingRootOccurrences)?.span;
            let failure = selected_failure(source, directive, span, IntegerInputs::closed(), Vec::new());
            if !COLLECT {
                return Err(failure);
            }
            failures.as_deref_mut().ok_or(ProjectCheck::Unsupported)?.push(failure);
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
                if check_initializer_expression::<COLLECT>(topology, actual, source, storage, expression, depth + 1, failures)?
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
        Some(OrderedCallCollection { failures: &sink, nested_calls_remaining: 1 })
    } else { None };
    match run_initializer_call(topology, actual, expression, &ScalarType::Unit, |_, _| None, collection)? {
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
    collection: Option<OrderedCallCollection<'_>>,
) -> Result<ActualCallResult, ProjectCheck> {
    let Some(ModuleInitializationCallTarget::Direct {
        declaration,
        concrete: ConcreteCallSelection::Declaration,
    }) = topology.call_targets.get(&occurrence) else {
        return Err(ProjectCheck::Unsupported);
    };
    let signature = topology.checked_call_signatures.get(&occurrence).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let arguments = if is_integer_type(expected) && signature.parameters.len() == 2 {
        let first = exact_integer_call_argument_at(&topology.structure, occurrence, 0, &initialized_argument)
            .ok_or(ProjectCheck::Unsupported)?;
        let second = exact_integer_call_argument_at(&topology.structure, occurrence, 1, &initialized_argument)
            .ok_or(ProjectCheck::Unsupported)?;
        ActualProjectedArguments::InitializerPair(first, second)
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

struct OrderedCallCollection<'a> {
    failures: &'a RefCell<&'a mut Vec<ProjectCheck>>,
    nested_calls_remaining: u8,
}

enum ActualProjectedArguments<'a> {
    Single(Option<ExactIntegerArgument<'a>>),
    InitializerPair(ExactIntegerArgument<'a>, ExactIntegerArgument<'a>),
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
        argument: Option<ExactIntegerArgument<'_>>,
        expected: &ScalarType,
        collection: Option<(&SourceIdentity, OrderedCallCollection<'_>)>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let site = caller.resolved_call_site(call).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let ResolvedCallTarget::ModuleCallable {
            source,
            declaration_span,
            concrete: ConcreteCallSelection::Declaration,
        } = &site.target else {
            return Err(ProjectCheck::Unsupported);
        };
        let collection = match collection {
            Some((owner, collection)) if owner == source && *expected == ScalarType::Unit => Some(collection),
            Some(_) => return Err(ProjectCheck::Unsupported),
            None => None,
        };
        let signature = caller.checked_call_signatures.get(&call).ok_or(ProjectCheck::MissingRootOccurrences)?;
        self.run(&caller.structure, call, signature, ActualProjectedArguments::Single(argument), source, *declaration_span, expected, collection)
    }

    fn requirements(
        &mut self,
        body: &FunctionCallSites,
        source: &SourceIdentity,
        exact: Option<&[&BigInt]>,
    ) -> Result<bool, ProjectCheck> {
        let mut nested_error = None;
        let checked = check_integer_requirements_with_calls(body, exact, &mut |caller, occurrence, argument, expected| {
            match self.nested(caller, occurrence, argument, expected, None) {
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
        exact: Option<&[&BigInt]>,
        collection: OrderedCallCollection<'_>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let OrderedCallCollection { failures, nested_calls_remaining } = collection;
        let mut sink = |failure| failures.borrow_mut().push(project_failure(body, source, failure));
        let mut nested_error = None;
        let mut nested = |caller: &FunctionCallSites, occurrence, argument: Option<ExactIntegerArgument<'_>>, expected: &ScalarType| {
            let collection = OrderedCallCollection {
                failures, nested_calls_remaining: nested_calls_remaining - 1,
            };
            match self.nested(caller, occurrence, argument, expected, Some((source, collection))) {
                Ok(result) => Some(result),
                Err(error) => {
                    nested_error = Some(error);
                    None
                }
            }
        };
        let handler: Option<&mut super::checker_flow::ActualCallHandler<'_, '_>> =
            if nested_calls_remaining > 0 { Some(&mut nested) } else { None };
        let Some((active, continued)) = collect_integer_requirement_failures_with_calls(
            body, exact, handler, &mut sink,
        ) else {
            return Err(nested_error.unwrap_or(ProjectCheck::Unsupported));
        };
        self.had_obligations |= active;
        Ok(if continued { ActualCallResult::Unit } else { ActualCallResult::EmptyUnit })
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
        collection: Option<OrderedCallCollection<'_>>,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let &(target_module, target_item) = structure.checker_direct_targets.get(&call)
            .ok_or(ProjectCheck::MissingRootOccurrences)?;
        if !ACTUAL_STARTUP && (*expected != ScalarType::Unit || target_module != self.symbolic_module) {
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
                ActualProjectedArguments::InitializerPair((first_ty, first_value), (second_ty, second_value)))
                if is_integer_type(expected)
                    && *first == first_slot.ty && first == first_ty
                    && *second == second_slot.ty && second == second_ty =>
            {
                pair = [first_value, second_value];
                &pair
            }
            _ => return Err(ProjectCheck::Unsupported),
        };
        let target = (target_module, target_item);
        if self.active.len() == MAX_ACTUAL_CALL_DEPTH {
            return Err(ProjectCheck::Unsupported);
        }
        if !self.active.insert(target) {
            return Err(ProjectCheck::Unsupported);
        }
        let result = if *expected == ScalarType::Unit {
            if let Some(collection) = collection {
                self.collect_requirements(body, &owner.source, Some(exact), collection)
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
            proof.children_with_tokens()
                .filter_map(|element| element.into_token())
                .find(|token| token.kind() == SyntaxKind::Identifier)
                .is_some_and(|token| token.text() == proof_name)
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
        canonical_target: &'a ResolvedCallTarget,
        target_layout: ScalarTargetLayout,
        parameters: &'a [ParameterSlot],
        output: &'a ScalarType,
        guards: Vec<ImportedEntryGuard<'a>>,
        values: IntegerSet,
    }

    pub(super) fn apply(
        project: &PreparedProofModule,
        (source, name): (&SourceIdentity, &str),
        (caller_source, caller_name): (&SourceIdentity, &str),
    ) -> Option<ScalarProofCallOutcome> {
        let plan = project.plan_module_proof_in_scope::<true>(source, name).ok()?;
        let TypedProofGoal::Literal { operator, relation, bound } = &plan.typed.goal else { return None; };
        let (scalar, calls) = project.prepared.parts();
        let caller = select_checked_root(scalar, caller_source, caller_name).ok()?;
        let (root, ordinals) = package_initialization_order(scalar).ok()?;
        if plan.typed.measure.is_some()
            && (operator != &BinaryOperator::Equal || plan.proof_source != root
                || &plan.typed.target.source != root || caller_source != root)
        {
            return None;
        }
        let initialization = check_package_initialization_with_order(&project.prepared, root, ordinals).ok()?;
        let &venue = initialization.ordinals.get(plan.proof_source)?;
        if initialization.visits[venue] != InitializationVisit::Completed
            || initialization.visits[plan.target.0] != InitializationVisit::Completed
            || initialization.visits[caller.0] != InitializationVisit::Completed
            || initialization.reached.contains(&plan.target)
            || initialization.reached.contains(&(caller.0, caller.1))
            || (caller.0, caller.1) == plan.target
        {
            return None;
        }

        let body = calls.function_body(caller.0, caller.1, None)?;
        let (position, origin) = root_local_import_requirement(body)?;
        if !caller.2.generic_parameters.is_empty() || !caller.2.overload_arms.is_empty()
            || !body.allocation_length_calls.is_empty()
            || !matches!(body.call_occurrences.as_slice(), [call] if call.ordinal == origin.call)
            || body.structure.checker_directives.len() != 1
        {
            return None;
        }
        let site = body.resolved_call_site(origin.call)?;
        let signature = body.checked_call_signatures.get(&origin.call)?;
        let identity = (plan.proof_source, wosy_syntax::byte_span(plan.proof), name);
        let verified = VerifiedModuleBound::establish(project, &plan, identity, (operator, *relation, bound))?;
        verified.consume(project, identity, scalar.modules[caller.0].target_layout, body, position, origin, &site.target, signature)
    }

    impl<'a> VerifiedModuleBound<'a> {
        fn establish(
            issuer: &'a PreparedProofModule,
            plan: &'a PlannedModuleProof<'_>,
            proof_identity: (&'a SourceIdentity, ByteSpan, &'a str),
            (operator, relation, bound): (&BinaryOperator, IntegerRelation, &BigInt),
        ) -> Option<Self> {
            let body = plan.typed.body;
            let parameters = body.boundary.parameters.as_slice();
            if !matches!(parameters.len(), 1 | 2) {
                return None;
            }
            let [output] = body.boundary.results.as_slice() else { return None; };
            if output.ordinal != 0 {
                return None;
            }
            let guard = checked_integer_guard_slots(body, plan.filter.as_deref().unwrap_or_default())?;
            if guard.always_rejects {
                return None;
            }
            let values = if let Some(rank) = plan.typed.measure {
                if !matches!(plan.verify_unused_induction(rank), Ok(ProjectCheck::Proved)) {
                    return None;
                }
                IntegerSet::singleton(bound)
            } else {
                if !plan.steps.items.is_empty() || !valid_arbitrary_proof_body(body) {
                    return None;
                }
                let ProofCoverage::Applicable(paths) = observe_arbitrary_integer_returns(body, plan.filter.as_deref())?.ok()? else {
                    return None;
                };
                if !paths.iter().all(|path| path.output_satisfies(relation, bound) == Some(true)) {
                    return None;
                }
                IntegerSet::from_type(&output.ty)?.keep(operator, bound)?
            };
            Some(Self {
                issuer, proof_identity, canonical_target: plan.canonical_target,
                target_layout: plan.typed.target.target_layout,
                parameters, output: &output.ty, guards: guard.comparisons, values,
            })
        }

        fn consume(
            self,
            issuer: &PreparedProofModule,
            identity: (&SourceIdentity, ByteSpan, &str),
            caller_layout: ScalarTargetLayout,
            body: &FunctionCallSites,
            position: usize,
            origin: &CheckerIntegerCallOrigin,
            target: &ResolvedCallTarget,
            signature: &CheckedCallSignature,
        ) -> Option<ScalarProofCallOutcome> {
            let [output] = signature.outputs.outputs.as_slice() else { return None; };
            let arguments = origin.arguments.as_slice();
            if !std::ptr::eq(self.issuer, issuer) || self.proof_identity != identity
                || self.canonical_target != target || self.target_layout != caller_layout
                || self.output != &output.ty || self.parameters.len() != signature.parameters.len()
                || self.parameters.len() != arguments.len()
                || self.parameters.iter().zip(&signature.parameters).zip(arguments).any(|((producer, caller), (ordinal, _))| {
                    &producer.ty != caller || body.structure.checked_types.expressions.get(ordinal) != Some(caller)
                })
            {
                return None;
            }
            let fact = ImportedLocal { position, values: self.values, guards: self.guards };
            Some(if proves_requirement_with_imported_root_local(body, fact)? {
                ScalarProofCallOutcome::Established
            } else {
                ScalarProofCallOutcome::NotEstablished
            })
        }
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
impl PreparedProofModule {
    #[cfg(test)]
    fn plan_module_proof<'a>(&'a self, proof_source: &SourceIdentity, proof_name: &'a str) -> Result<PlannedModuleProof<'a>, ProjectCheck> {
        self.plan_module_proof_in_scope::<false>(proof_source, proof_name)
    }

    fn plan_module_proof_in_scope<'a, const ALLOW_INDUCTION: bool>(
        &'a self, proof_source: &SourceIdentity, proof_name: &'a str,
    ) -> Result<PlannedModuleProof<'a>, ProjectCheck> {
        let (proof_source, _) = self.proofs.get_key_value(proof_source).ok_or(ProjectCheck::MissingRoot)?;
        let proof = self.select_resolved(proof_source, proof_name)?;
        let (module, item, function) = proof.function;
        let steps = derive_block(&child(proof.node, SyntaxKind::Block)?);
        let typed = typed_proof_postcondition::<ALLOW_INDUCTION>(&self.prepared, proof.function, proof.node, &steps)?;
        let filter = prepare_proof_entry_filter(
            proof.node, proof_source, typed.target.target_layout, &typed.input_names, typed.body,
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
        let typed = typed_proof_postcondition::<true>(&self.prepared, proof.function, proof.node, &steps)?;
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
        let plan = self.plan_module_proof_in_scope::<true>(proof_source, proof_name)?;
        let measure = plan.typed.measure.ok_or(ProjectCheck::Unsupported)?;
        Ok(plan.verify_induction_return_witness(measure)?.unwrap_or(InductionPremise::Unreachable))
    }

    fn check_module_proofs(&self, proof_names: &[&str]) -> Vec<ScalarModuleProofOutcome> {
        let selections = proof_names.iter().map(|&name| (&self.source, name)).collect::<Vec<_>>();
        self.check_module_proof_selections(&selections)
    }

    fn check_module_proof_selections(&self, selections: &[(&SourceIdentity, &str)]) -> Vec<ScalarModuleProofOutcome> {
        let mut outcomes = vec![ScalarModuleProofOutcome::Unsupported; selections.len()];
        let mut selected = HashMap::new();
        let mut duplicate = Vec::new();
        let mut plans = Vec::new();
        for (index, &(proof_source, name)) in selections.iter().enumerate() {
            match selected.entry((proof_source, name)) {
                std::collections::hash_map::Entry::Occupied(first) => duplicate.push((index, *first.get())),
                std::collections::hash_map::Entry::Vacant(first) => {
                    first.insert(index);
                    match self.plan_module_proof_in_scope::<true>(proof_source, name) {
                        Ok(plan) => plans.push((index, plan)),
                        Err(error) => outcomes[index] = module_proof_outcome(Err(error)),
                    }
                }
            }
        }
        self.run_module_proof_plans(&plans, selected.len() == 1, &mut outcomes);
        for (index, first) in duplicate {
            outcomes[index] = outcomes[first].clone();
        }
        outcomes
    }

    fn run_module_proof_plans(
        &self,
        plans: &[(usize, PlannedModuleProof<'_>)],
        one_distinct_selection: bool,
        outcomes: &mut [ScalarModuleProofOutcome],
    ) {
        if plans.is_empty() {
            return;
        }
        let Ok((root, ordinals)) = package_initialization_order(self.prepared.parts().0) else { return; };
        let mut independently_checked = None;
        let mut induction = None;
        if let (true, [(_, plan)]) = (one_distinct_selection, plans) {
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
        let mut arbitrary_groups = HashMap::<_, Vec<_>>::new();
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
            } else if !matches!(plan.typed.goal, TypedProofGoal::Literal { .. } | TypedProofGoal::Conjunction(_) | TypedProofGoal::Universal { .. } | TypedProofGoal::Existential { .. }) {
                continue;
            } else if plan.uses_direct_goal() {
                let key = (plan.target, ExactGuardGroup(plan.filter.as_deref()));
                arbitrary_groups.entry(key).or_default().push((*index, plan));
            } else {
                outcomes[*index] = module_proof_outcome(plan.verify_arbitrary_with_steps());
            }
        }
        for ((_, guard), group) in arbitrary_groups {
            let body = group[0].1.typed.body;
            if !valid_arbitrary_proof_body(body) {
                continue;
            }
            match observe_arbitrary_integer_returns(body, guard.0) {
                Some(Ok(ProofCoverage::Applicable(paths))) => {
                    for (index, plan) in group {
                        let mut holds = Some(true);
                        for path in &paths {
                            holds = plan.typed.goal.holds_on_returned_path(path);
                            if holds != Some(true) {
                                break;
                            }
                        }
                        outcomes[index] = match holds {
                            Some(true) => ScalarModuleProofOutcome::Proved,
                            Some(false) => ScalarModuleProofOutcome::Refuted,
                            None => ScalarModuleProofOutcome::Unsupported,
                        };
                    }
                }
                Some(Ok(ProofCoverage::NoApplicableInputs)) if guard.0.is_some() => {
                    for (index, _) in group {
                        outcomes[index] = ScalarModuleProofOutcome::NoApplicableInputs;
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
        self.steps.items.is_empty() || matches!(self.typed.goal, TypedProofGoal::Existential { .. })
    }

    fn observer(&self) -> SelectedProofObserver<'_> {
        SelectedProofObserver {
            guard: if self.uses_direct_goal() {
                checked_integer_guard_slots(self.typed.body, self.filter.as_deref().unwrap_or_default())
            } else {
                None
            },
            goal: &self.typed.goal,
            coverage: ActualProofCoverage::Unseen,
        }
    }

    fn verify_arbitrary_with_steps(&self) -> Result<ProjectCheck, ProjectCheck> {
        let TypedProofPostcondition { target, body, result_type, input_names, output, goal, .. } = &self.typed;
        let TypedProofGoal::Literal { operator, bound, .. } = goal else {
            return Err(ProjectCheck::Unsupported);
        };
        if !valid_arbitrary_proof_body(body) {
            return Err(ProjectCheck::Unsupported);
        }
        let proof = proof_show_plan::<false>(&self.steps, input_names, output, result_type, body)?;
        let checked = check_integer_return_bound_with_shows(body, operator, bound, self.filter.as_deref(), &proof);
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
}

#[cfg(feature = "internal-checker")]
impl CheckedIntegerGuardSlots<'_> {
    fn accepts(&self, arguments: &[&BigInt]) -> bool {
        self.arity == arguments.len() && !self.always_rejects
            && self.comparisons.iter().all(|comparison| {
                arguments.get(comparison.slot).is_some_and(|argument| comparison.relation.accepts((*argument).cmp(comparison.bound)))
            })
    }
}

#[cfg(feature = "internal-checker")]
fn checked_integer_guard_slots<'a>(
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
    let mut guard = CheckedIntegerGuardSlots { arity: parameters.len(), always_rejects: false, comparisons: Vec::new() };
    for predicate in filter {
        match predicate {
            CheckerPredicate::Boolean(value) => guard.always_rejects |= !value,
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
struct SelectedProofObserver<'a> {
    guard: Option<CheckedIntegerGuardSlots<'a>>,
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
        match self.coverage {
            ActualProofCoverage::Proved => Ok(ProjectCheck::Proved),
            ActualProofCoverage::Refuted => Ok(ProjectCheck::Refuted),
            ActualProofCoverage::Inapplicable => Ok(ProjectCheck::NoApplicableInputs),
            ActualProofCoverage::Unsupported | ActualProofCoverage::Unseen => Err(ProjectCheck::Unsupported),
        }
    }

    fn record(&mut self, arguments: &[&BigInt], value: &BigInt) {
        let Some(guard) = self.guard.as_ref().filter(|guard| guard.arity == arguments.len()) else {
            self.coverage = ActualProofCoverage::Unsupported;
            return;
        };
        if matches!(self.coverage, ActualProofCoverage::Unsupported | ActualProofCoverage::Refuted) {
            return;
        }
        if guard.accepts(arguments) {
            let holds = match self.goal {
                TypedProofGoal::Literal { relation, bound, .. } | TypedProofGoal::Universal { relation, bound } =>
                    Some(relation.accepts(value.cmp(bound))),
                TypedProofGoal::Existential { relation, witness, witness_in_domain } =>
                    (*witness_in_domain && relation.accepts(value.cmp(witness))).then_some(true),
                TypedProofGoal::Conjunction(atoms) => Some(atoms.iter().all(|(relation, bound)| relation.accepts(value.cmp(bound)))),
                TypedProofGoal::EntryEqual { slot } => arguments.get(*slot).map(|entry| *entry == value),
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
                where_guard: checked_integer_guard_slots(plan.typed.body, plan.filter.as_deref().unwrap_or_default())?,
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

enum TypedProofGoal {
    Literal {
        operator: BinaryOperator,
        relation: IntegerRelation,
        bound: BigInt,
    },
    Conjunction(Vec<(IntegerRelation, BigInt)>),
    Universal { relation: IntegerRelation, bound: BigInt },
    Existential { relation: IntegerRelation, witness: BigInt, witness_in_domain: bool },
    EntryEqual { slot: usize },
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
            Self::Conjunction(atoms) => {
                for (relation, bound) in atoms {
                    if !path.output_satisfies(*relation, bound)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Self::EntryEqual { .. } => None,
        }
    }
}

fn checked_literal_output_goal(
    comparison: IntegerComparison<'_>, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<(BinaryOperator, IntegerRelation, BigInt), ProjectCheck> {
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
    Ok((comparison.operator.clone(), relation, comparison.bound.into_owned()))
}

fn typed_universal_output_goal(
    forall: &CstNode,
    ensures: &ScalarExpression,
    output: &str,
    inputs: &[String],
    output_type: &ScalarType,
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

fn typed_existential_output_goal(
    exists: &CstNode, steps: &ScalarBlock, output: &str, inputs: &[String], ty: &ScalarType,
) -> Result<TypedProofGoal, ProjectCheck> {
    let (ty, binder) = typed_quantifier_binder(exists, output, inputs, ty)?;
    let predicate = derive_expression(&child(exists, SyntaxKind::Expression)?);
    let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } = predicate else {
        return Err(ProjectCheck::Unsupported);
    };
    if !matches!(left.as_ref(), ScalarExpression::Name { name, .. } if name == output)
        || !matches!(right.as_ref(), ScalarExpression::Name { name, .. } if name == &binder)
    {
        return Err(ProjectCheck::Unsupported);
    }
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
    let witness = concrete_integer_literal(argument).ok_or(ProjectCheck::Unsupported)?;
    if !integer_literal_fits_type(witness.as_ref(), &ty) {
        return Err(ProjectCheck::InvalidProof);
    }
    let domain = typed_quantifier_domain(exists, &binder, inputs, &ty)?;
    let relation = IntegerRelation::from_operator(&BinaryOperator::Equal).expect("integer equality");
    let witness_in_domain = domain.contains(witness.as_ref());
    Ok(TypedProofGoal::Existential { relation, witness: witness.into_owned(), witness_in_domain })
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

fn typed_proof_postcondition<'a, const AUDIT_INDUCTION: bool>(
    prepared: &'a PreparedCheckerProject,
    (module, item, function): (usize, usize, &'a ScalarFunction),
    declaration: &CstNode,
    steps: &ScalarBlock,
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
        if foralls.next().is_some() || measure.is_some() {
            return Err(ProjectCheck::Unsupported);
        }
        typed_existential_output_goal(&exists, steps, output, &input_names, &result.ty)?
    } else {
        let ensures = derive_expression(&child(&ensures_node, SyntaxKind::Expression)?);
        if let Some(forall) = foralls.next() {
            if foralls.next().is_some() || measure.is_some() {
                return Err(ProjectCheck::Unsupported);
            }
            typed_universal_output_goal(&forall, &ensures, output, &input_names, &result.ty)?
        } else if matches!(&ensures, ScalarExpression::Binary { operator: BinaryOperator::And, .. }) {
            let mut pending = vec![&ensures];
            let mut atoms = Vec::new();
            while let Some(expression) = pending.pop() {
                if let ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } = expression {
                    pending.push(right);
                    pending.push(left);
                    continue;
                }
                let comparison = IntegerComparison::from_expression(expression).ok_or(ProjectCheck::Unsupported)?;
                let (_, relation, bound) = checked_literal_output_goal(comparison, output, &input_names, &result.ty)?;
                atoms.push((relation, bound));
            }
            TypedProofGoal::Conjunction(atoms)
        } else if let Some(ensures) = IntegerComparison::from_expression(&ensures) {
            let (operator, relation, bound) = checked_literal_output_goal(ensures, output, &input_names, &result.ty)?;
            TypedProofGoal::Literal { operator, relation, bound }
        } else {
            let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } = &ensures else {
                return Err(ProjectCheck::Unsupported);
            };
            let (ScalarExpression::Name { name: out, .. }, ScalarExpression::Call {
                receiver: Some(core), name: old, type_arguments, arguments, ..
            }) = (left.as_ref(), right.as_ref()) else {
                return Err(ProjectCheck::Unsupported);
            };
            if core != "core" || old != "old" || !type_arguments.is_empty() {
                return Err(ProjectCheck::Unsupported);
            }
            let [ScalarExpression::Name { name: input, .. }] = arguments.as_slice() else {
                return Err(ProjectCheck::Unsupported);
            };
            if out != output {
                return Err(ProjectCheck::InvalidProof);
            }
            let slot = input_names.iter().position(|name| name == input).ok_or(ProjectCheck::InvalidProof)?;
            if boundary.parameters.len() != 1 || boundary.parameters[slot].ty != result.ty {
                return Err(ProjectCheck::Unsupported);
            }
            TypedProofGoal::EntryEqual { slot }
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
        typed_proof_postcondition::<false>(prepared, (module, item, function), declaration, &steps)?;
    let TypedProofGoal::Literal { operator, bound, .. } = goal else {
        return Err(ProjectCheck::Unsupported);
    };

    let filter = prepare_proof_entry_filter(declaration, proof_source, target.target_layout, &input_names, body)
        .map_err(|error| match error {
            ProjectCheck::Invalid(_) => ProjectCheck::InvalidProof,
            error => error,
        })?;
    let checked = if !steps.items.is_empty() {
        let proof = proof_show_plan::<true>(&steps, &input_names, &output, result_type, body)?;
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
    let imported = ImportedLocal { position, values, guards };
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
    typed_proof_where(declaration, source, layout, inputs, body)?
        .as_ref().map(|expression| proof_entry_filters(expression, inputs, body)).transpose()
}

fn proof_entry_filters(
    expression: &ScalarExpression,
    inputs: &[String],
    body: &FunctionCallSites,
) -> Result<Vec<CheckerPredicate>, ProjectCheck> {
    let mut filters = Vec::new();
    let mut pending = vec![(expression, false)];
    while let Some((expression, negated)) = pending.pop() {
        match expression {
            ScalarExpression::Unary { operator: UnaryOperator::LogicalNot, operand, .. } =>
                pending.push((operand, !negated)),
            ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } if !negated => {
                pending.push((right, false));
                pending.push((left, false));
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

fn proof_show_plan<'a, const VERIFY_ENTRY_WRITES: bool>(
    steps: &'a ScalarBlock,
    inputs: &[String],
    output: &str,
    ty: &ScalarType,
    body: &'a FunctionCallSites,
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
        [ScalarBlockItem::Expression(ScalarExpression::Call { .. })] => Ok(IntegerShowProof::Equality(
            proof_show_claim(steps, inputs, output, ty, body, bindings)?,
        )),
        [ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. })] => {
            let guard = proof_input_condition::<true>(condition, inputs, body, false)?;
            Ok(IntegerShowProof::Conditional {
                guard,
                then_claim: proof_show_claim(then_branch, inputs, output, ty, body, bindings)?,
                else_claim: proof_show_claim(else_branch, inputs, output, ty, body, bindings)?,
            })
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
    let [ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver, name, arguments, type_arguments, ..
    })] = arm.items.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    if receiver.as_deref() != Some("core")
        || name != "show"
        || !type_arguments.is_empty()
        || arm.terminated_items.as_slice() != [true]
    {
        return Err(ProjectCheck::Unsupported);
    }
    let [ScalarExpression::Binary {
        operator: BinaryOperator::Equal, left, right, ..
    }] = arguments.as_slice() else {
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
                unit() leaf = fn {{}};
                unit() helper = fn {{ leaf(); }};
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
        let (_, prepared) = prepare("leaf(1);", "", "value");
        assert_eq!(collect_ordered_package_failures(&prepared, ScalarCheckerScope::CurrentPackage, 1),
            Err(ProjectCheck::Unsupported));
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
            unit() deeper = fn {{}};
            unit() leaf = fn {{ deeper(); }};
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
            unit(u8) outer = fn(value) {{
                core.require(value < 3, {OUTPUT});
                mark();
                inner(3);
                inner(value);
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
            ("value < 2", Require, Some(4.into())),
            ("value < 4", Require, Some(4.into())),
            ("value < 3", Require, Some(5.into())),
            ("false", Require, None),
            ("value < 2", Require, Some(3.into())),
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
