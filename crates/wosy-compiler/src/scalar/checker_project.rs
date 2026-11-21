use std::collections::{HashMap, HashSet};

use num_bigint::BigInt;

use super::{
    checker_directive::{CheckedLiteralOutput, CheckerOperation},
    checker_flow::{
        check_integer_requirements, check_integer_requirements_at_entry, check_integer_return_bound, check_integer_return_bound_where,
        check_integer_return_bound_with_shows, exact_integer_call_argument, exact_integer_return_at_entry,
        proves_requirement_with_imported_root_local, ActualCallResult, ExactIntegerArgument,
        CheckerPredicate, ImportedLocal, IntegerBranchProof,
        IntegerCheckFailure, IntegerComparison, IntegerInputs, IntegerReturn, ProofCoverage,
    },
    control::{BlockItemKind, CheckedCallSignature, ModuleInitializationCallTarget, ModuleInitializationDetail, ModuleInitializationStepKind, NamespaceTarget},
    derive_block, derive_expression, project_from_declarations,
    reference::{CheckerTargetQuery, DirectDestinationIdentity, FunctionCallSites, OccurrenceEdge, OccurrenceNode, OccurrenceOrdinal, OccurrenceParent, OccurrenceStructure, ProjectCallSiteResults, ResolvedCheckerTarget},
    static_calls::prepare_project_occurrences,
    symbolic_integer::{IntegerRelation, IntegerSet},
    typecheck::{integer_literal_fits_type, is_integer_type, typecheck_proof_where},
    typecheck_scalar_project_in_mode, BinaryOperator, ByteSpan, ConcreteCallSelection,
    ReferenceBindingId, ResolvedCallTarget, ScalarBlock, ScalarBlockItem, ScalarExpression, ScalarFunction,
    ScalarItem, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarProject,
    ScalarSourceDeclarations, ScalarTargetLayout, ScalarType,
};
use crate::Diagnostic;
use wosy_syntax::{byte_span, CstNode, CstToken, ProofFragmentParse, SourceIdentity, SyntaxKind};

#[derive(Debug, PartialEq)]
enum ProjectCheck {
    Invalid(Vec<Diagnostic>),
    InvalidProof,
    MissingRoot,
    AmbiguousRoot,
    RootSourceOutsideInitOrder,
    MissingRootOccurrences,
    Unsupported,
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
        axes: Vec<ReferenceBindingId>,
        output: CheckedLiteralOutput,
    },
}

#[derive(Debug, PartialEq, Eq)]
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

#[derive(Debug, PartialEq, Eq)]
enum ProofVenue {
    DetachedFragment,
    ModuleDeclaration,
}

enum ProofSummaryOrigin<'a> {
    DetachedFragment(&'a SourceIdentity),
    ModuleDeclaration {
        source: &'a SourceIdentity,
        target: &'a ResolvedCallTarget,
    },
}

#[derive(Debug, PartialEq, Eq)]
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
}

fn check_package_roots(prepared: &PreparedCheckerProject) -> Result<(), ProjectCheck> {
    let (project, calls) = prepared.parts();
    let CheckedInitialization { visits, reached, ordinals } = check_package_initialization(prepared)?;
    let root = project.initialization_order.first().ok_or(ProjectCheck::MissingRoot)?;
    for source in &project.initialization_order {
        if source.project != root.project || source.package != root.package {
            continue;
        }
        let ordinal = *ordinals.get(source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
        if visits[ordinal] != InitializationVisit::Completed {
            continue;
        }
        let module = &project.modules[ordinal];
        for (item, declaration) in module.items.iter().enumerate() {
            let ScalarItem::Function(function) = declaration else {
                continue;
            };
            if reached.contains(&(ordinal, item)) {
                continue;
            }
            if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() {
                return Err(ProjectCheck::Unsupported);
            }
            let body = calls.function_body(ordinal, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
            if !matches!(body.boundary.results.as_slice(), [result]
                if result.ty == ScalarType::Unit || is_integer_type(&result.ty))
            {
                return Err(ProjectCheck::Unsupported);
            }
            match check_integer_requirements(body) {
                Some(Ok(())) => {}
                Some(Err(failure)) => return Err(project_failure(body, source, failure)),
                None => return Err(ProjectCheck::Unsupported),
            }
        }
    }
    Ok(())
}

fn check_package_initialization(
    prepared: &PreparedCheckerProject,
) -> Result<CheckedInitialization<'_>, ProjectCheck> {
    let (project, calls) = prepared.parts();
    let ordinals: HashMap<_, _> = project.modules.iter().enumerate()
        .map(|(ordinal, module)| (&module.source, ordinal)).collect();
    if ordinals.len() != project.modules.len() {
        return Err(ProjectCheck::AmbiguousRoot);
    }
    let root = project.initialization_order.first().ok_or(ProjectCheck::MissingRoot)?;
    if project.initialization_order.len() != ordinals.len()
        || !project.initialization_order.iter().all(|source| ordinals.contains_key(source))
    {
        return Err(ProjectCheck::RootSourceOutsideInitOrder);
    }
    let mut visits = vec![InitializationVisit::Unseen; project.modules.len()];
    let mut actual = ActualCalls { project, calls, active: HashSet::new(), reached: HashSet::new() };
    walk_package_initialization(project, calls, &ordinals, &mut visits, &mut actual, root)?;
    Ok(CheckedInitialization { visits, reached: actual.reached, ordinals })
}

fn walk_package_initialization(
    project: &ScalarProject,
    calls: &ProjectCallSiteResults,
    ordinals: &HashMap<&SourceIdentity, usize>,
    visits: &mut [InitializationVisit],
    actual: &mut ActualCalls<'_>,
    source: &SourceIdentity,
) -> Result<(), ProjectCheck> {
    let ordinal = *ordinals.get(source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
    match visits[ordinal] {
        InitializationVisit::Completed => return Ok(()),
        InitializationVisit::Started => return Err(ProjectCheck::Unsupported),
        InitializationVisit::Unseen => visits[ordinal] = InitializationVisit::Started,
    }
    let module = &project.modules[ordinal];
    let region = calls.initialization_region(ordinal).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
        return Err(ProjectCheck::MissingRootOccurrences);
    };
    for step in &region.steps {
        if let ModuleInitializationStepKind::NamespaceDispatch { target, .. } = &step.kind {
            let NamespaceTarget::Bound(target) = target else {
                return Err(ProjectCheck::Unsupported);
            };
            walk_package_initialization(project, calls, ordinals, visits, actual, target)?;
            continue;
        }
        if !matches!((&step.kind, module.items.get(step.item)),
            (ModuleInitializationStepKind::ExecutableItem, Some(ScalarItem::Executable(
                ScalarBlockItem::Expression(ScalarExpression::Call { .. }),
            )))
        ) {
            return Err(ProjectCheck::Unsupported);
        }
        let structure = &topology.structure;
        let Some(OccurrenceNode::Expression(call)) = structure.edges.get(&(
            OccurrenceParent::Block(structure.root),
            OccurrenceEdge::ExpressionItem { item: step.item },
        )) else {
            return Err(ProjectCheck::Unsupported);
        };
        let Some(ModuleInitializationCallTarget::Direct {
            declaration,
            concrete: ConcreteCallSelection::Declaration,
        }) = region.call_target(*call) else {
            return Err(ProjectCheck::Unsupported);
        };
        let signature = topology.checked_call_signatures.get(call).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let argument = exact_integer_call_argument(structure, *call, |_, _| None);
        actual.run(structure, *call, signature, argument, &declaration.source, declaration.range, &ScalarType::Unit)?;
    }
    visits[ordinal] = InitializationVisit::Completed;
    Ok(())
}

struct ActualCalls<'a> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    active: HashSet<(usize, usize)>,
    reached: HashSet<(usize, usize)>,
}

const MAX_ACTUAL_CALL_DEPTH: usize = 64;

impl ActualCalls<'_> {
    fn nested(
        &mut self,
        caller: &FunctionCallSites,
        call: OccurrenceOrdinal,
        argument: Option<ExactIntegerArgument<'_>>,
        expected: &ScalarType,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let site = caller.resolved_call_site(call).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let ResolvedCallTarget::ModuleCallable {
            source,
            declaration_span,
            concrete: ConcreteCallSelection::Declaration,
        } = &site.target else {
            return Err(ProjectCheck::Unsupported);
        };
        let signature = caller.checked_call_signatures.get(&call).ok_or(ProjectCheck::MissingRootOccurrences)?;
        self.run(&caller.structure, call, signature, argument, source, *declaration_span, expected)
    }

    fn run(
        &mut self,
        structure: &OccurrenceStructure,
        call: OccurrenceOrdinal,
        signature: &CheckedCallSignature,
        projected: Option<ExactIntegerArgument<'_>>,
        target_source: &SourceIdentity,
        target_span: ByteSpan,
        expected: &ScalarType,
    ) -> Result<ActualCallResult, ProjectCheck> {
        let &(target_module, target_item) = structure.checker_direct_targets.get(&call)
            .ok_or(ProjectCheck::MissingRootOccurrences)?;
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
        let exact: &[&BigInt] = match (signature.parameters.as_slice(), body.boundary.parameters.as_slice(), projected) {
            ([], [], None) => &[],
            ([parameter], [slot], Some((ty, value))) if *parameter == slot.ty && parameter == ty => {
                argument = [value];
                &argument
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
            let mut nested_error = None;
            let checked = check_integer_requirements_at_entry(body, exact, &mut |caller, occurrence, argument, expected| {
                match self.nested(caller, occurrence, argument, expected) {
                    Ok(result) => Some(result),
                    Err(error) => {
                        nested_error = Some(error);
                        None
                    }
                }
            });
            match checked {
                Some(Ok(())) => Ok(ActualCallResult::Unit),
                Some(Err(failure)) => Err(project_failure(body, &owner.source, failure)),
                None => Err(nested_error.unwrap_or(ProjectCheck::Unsupported)),
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
            Err(ProjectCheck::Unsupported)
        };
        self.active.remove(&target);
        if result.is_ok() {
            self.reached.insert(target);
        }
        result
    }
}

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
        prepare_checker_project_with_queries(declarations, initialization_order, inputs, &[])
    }

    pub(super) fn prepare_checker_project_with_queries(
        declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
        queries: &[CheckerTargetQuery<'_>],
    ) -> Result<PreparedCheckerProject, ProjectCheck> {
        let (project, calls) = project_from_declarations(declarations, initialization_order, inputs)
            .and_then(|project| prepare_project_occurrences(typecheck_scalar_project_in_mode(project, true), queries))
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
    let directive = &body.structure.checker_directives[&failure.directive];
    ProjectCheck::Failed {
        source: source.clone(),
        operation: directive.operation,
        directive: body.occurrences[failure.directive.0].span,
        condition: directive.condition,
        state: failure.state,
        axes: failure.axes.into_iter().cloned().collect(),
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

struct ProofModuleInput {
    source: SourceIdentity,
    text: String,
    namespaces: Vec<ScalarNamespaceBinding>,
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

struct VerifiedModuleReturnBound<'a> {
    issuer: &'a PreparedProofModule,
    proof: &'a CstNode,
    bound: VerifiedReturnBound,
}

impl PreparedProofModule {
    fn prepare(source: SourceIdentity, text: String, inputs: ScalarPreparationInputs) -> Result<Self, ProjectCheck> {
        Self::prepare_project(
            source.clone(),
            vec![ProofModuleInput { source: source.clone(), text, namespaces: Vec::new() }],
            vec![source],
            inputs,
        )
    }

    fn prepare_project(
        source: SourceIdentity,
        modules: Vec<ProofModuleInput>,
        initialization_order: Vec<SourceIdentity>,
        inputs: ScalarPreparationInputs,
    ) -> Result<Self, ProjectCheck> {
        let mut declarations = Vec::with_capacity(modules.len());
        let mut proofs = HashMap::new();
        let mut graph_sources: HashMap<_, _> = initialization_order.iter().map(|source| (source, false)).collect();
        if graph_sources.len() != initialization_order.len() {
            return Err(ProjectCheck::AmbiguousRoot);
        }
        if !graph_sources.contains_key(&source) {
            return Err(ProjectCheck::MissingRoot);
        }
        for module in modules {
            let parsed = wosy_syntax::parse_proof_module(module.source, module.text, &[]);
            if !parsed.is_valid() {
                return Err(ProjectCheck::InvalidProof);
            }
            let canonical = parsed.canonical_cst();
            let seen = graph_sources.get_mut(&canonical.source).ok_or(ProjectCheck::RootSourceOutsideInitOrder)?;
            if std::mem::replace(seen, true) {
                return Err(ProjectCheck::AmbiguousRoot);
            }
            if module.namespaces.iter().any(|edge| !graph_sources.contains_key(&edge.target)) {
                return Err(ProjectCheck::RootSourceOutsideInitOrder);
            }
            let mut module_proofs = Vec::new();
            let module_declarations = super::derive_scalar_declarations(canonical, Some(&mut module_proofs));
            if !module_proofs.is_empty() {
                proofs.insert(canonical.source.clone(), ModuleProofs { definitions: module_proofs, first_query: 0 });
            }
            declarations.push((module_declarations, module.namespaces));
        }
        if graph_sources.values().any(|seen| !seen) {
            return Err(ProjectCheck::MissingRoot);
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

    fn check(&self, proof_name: &str) -> Result<(ByteSpan, ProjectCheck), ProjectCheck> {
        self.check_at(&self.source, proof_name)
    }

    fn check_at(&self, source: &SourceIdentity, proof_name: &str) -> Result<(ByteSpan, ProjectCheck), ProjectCheck> {
        let proof = self.select_resolved(source, proof_name)?;
        let (outcome, _) = check_selected_proof(&self.prepared, proof.function, proof.node, source, None)?;
        Ok((byte_span(proof.node), outcome))
    }

    fn type_where(&self, proof_name: &str) -> Result<(), ProjectCheck> {
        let proof = self.select_resolved(&self.source, proof_name)?;
        let (module, item, _) = proof.function;
        let (project, calls) = self.prepared.parts();
        let body = calls.function_body(module, item, None).ok_or(ProjectCheck::MissingRootOccurrences)?;
        let target = child(proof.node, SyntaxKind::ProofTarget)?;
        let inputs = identifiers(&child(&target, SyntaxKind::Parameters)?).collect::<Vec<_>>();
        let layout = project.modules[module].target_layout;
        let _ = typed_proof_where(proof.node, &self.source, layout, &inputs, body)?;
        Ok(())
    }

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

fn check_proof_fragment(
    prepared: &PreparedCheckerProject,
    target_source: &SourceIdentity,
    fragment: &ProofFragmentParse,
) -> ProjectCheck {
    check_proof_fragment_impl(prepared, target_source, fragment, false)
        .map(|(outcome, _)| outcome)
        .unwrap_or_else(|error| error)
}

fn verify_return_bound(
    prepared: &PreparedCheckerProject,
    target_source: &SourceIdentity,
    fragment: &ProofFragmentParse,
) -> Option<VerifiedReturnBound> {
    check_proof_fragment_impl(prepared, target_source, fragment, true)
        .ok()
        .and_then(|(_, bound)| bound)
}

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

fn check_selected_proof(
    prepared: &PreparedCheckerProject,
    (module, item, function): (usize, usize, &ScalarFunction),
    declaration: &CstNode,
    proof_source: &SourceIdentity,
    summary_source: Option<ProofSummaryOrigin<'_>>,
) -> Result<(ProjectCheck, Option<VerifiedReturnBound>), ProjectCheck> {
    let steps = derive_block(&child(declaration, SyntaxKind::Block)?);
    let target = child(declaration, SyntaxKind::ProofTarget)?;
    let input_names = identifiers(&child(&target, SyntaxKind::Parameters)?).collect::<Vec<_>>();
    let outputs = child(declaration, SyntaxKind::ProofOutputs)?;
    let output_names = identifiers(&child(&outputs, SyntaxKind::Parameters)?).collect::<Vec<_>>();
    let mut unique = HashSet::new();
    if !input_names.iter().chain(&output_names).all(|name| unique.insert(name.as_str())) {
        return Err(ProjectCheck::InvalidProof);
    }
    let ensures = derive_expression(&child(
        &child(declaration, SyntaxKind::ProofEnsures)?,
        SyntaxKind::Expression,
    )?);
    let ensures = IntegerComparison::from_expression(&ensures).ok_or(ProjectCheck::Unsupported)?;

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
    let ([result], [output]) = (boundary.results.as_slice(), output_names.as_slice()) else {
        return Err(ProjectCheck::Unsupported);
    };
    if !is_integer_type(&result.ty) {
        return Err(ProjectCheck::Unsupported);
    }
    if ensures.name != output {
        return Err(if input_names.iter().any(|input| input == ensures.name) {
            ProjectCheck::Unsupported
        } else {
            ProjectCheck::InvalidProof
        });
    }
    if !integer_literal_fits_type(ensures.bound, &result.ty) {
        return Err(ProjectCheck::InvalidProof);
    }

    let where_clause = typed_proof_where(declaration, proof_source, target.target_layout, &input_names, body)
        .map_err(|error| match error {
            ProjectCheck::Invalid(_) => ProjectCheck::InvalidProof,
            error => error,
        })?;
    let filter = where_clause.as_ref().map(|expression| proof_entry_filters(expression, &input_names, body)).transpose()?;
    let checked = if !steps.items.is_empty() {
        let proof = proof_show_branch(&steps, &input_names, output, &result.ty, body)?;
        check_integer_return_bound_with_shows(body, ensures.operator, ensures.bound, filter.as_deref(), &proof)
    } else if let Some(filter) = filter.as_deref() {
        check_integer_return_bound_where(body, ensures.operator, ensures.bound, filter)
    } else {
        check_integer_return_bound(body, ensures.operator, ensures.bound)
            .map(|result| result.map(ProofCoverage::Applicable))
    };
    let outcome = match checked {
        None => ProjectCheck::Unsupported,
        Some(Ok(ProofCoverage::Applicable(true))) => ProjectCheck::Proved,
        Some(Ok(ProofCoverage::Applicable(false))) => ProjectCheck::Refuted,
        Some(Ok(ProofCoverage::NoApplicableInputs)) => ProjectCheck::NoApplicableInputs,
        Some(Err(failure)) => project_failure(body, &target.source, failure),
    };
    let verified = summary_source.filter(|_| outcome == ProjectCheck::Proved).and_then(|origin| {
        issue_return_bound(
            origin,
            declaration,
            &target.source,
            function,
            body,
            target.target_layout,
            &ensures,
            filter.as_deref(),
        )
    });
    Ok((outcome, verified))
}

fn issue_return_bound(
    origin: ProofSummaryOrigin<'_>,
    declaration: &CstNode,
    target_source: &SourceIdentity,
    function: &ScalarFunction,
    body: &FunctionCallSites,
    target_layout: ScalarTargetLayout,
    ensures: &IntegerComparison<'_>,
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
        operator: ensures.operator.clone(),
        bound: ensures.bound.clone(),
        guards,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ImportedRequirement {
    Proved,
    NotEstablished,
}

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
    let site = body.resolved_call_site(origin.call)?;
    let signature = body.checked_call_signatures.get(&origin.call)?;
    let [parameter] = signature.parameters.as_slice() else {
        return None;
    };
    let [output] = signature.outputs.outputs.as_slice() else {
        return None;
    };
    if directive.operation != CheckerOperation::Require
        || predicate.binding != origin.binding
        || site.target != verified.target
        || signature.parameters != verified.parameters
        || verified.output_slot != 0
        || output.ty != verified.output_type
        || structure.checked_types.expressions.get(&origin.argument_ordinal)? != parameter
    {
        return None;
    }
    let mut guards = Vec::new();
    for guard in &verified.guards {
        match guard {
            VerifiedEntryGuard::Boolean(true) => {}
            VerifiedEntryGuard::Integer { slot: 0, ty, operator, bound } if ty == parameter =>
                guards.push((IntegerRelation::from_operator(operator)?, bound)),
            _ => return None,
        }
    }
    let values = IntegerSet::from_type(&verified.output_type)?.keep(&verified.operator, &verified.bound)?;
    let imported = ImportedLocal { position: binding_item.position, values, guards };
    Some(if proves_requirement_with_imported_root_local(body, imported)? {
        ImportedRequirement::Proved
    } else {
        ImportedRequirement::NotEstablished
    })
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
    if input_names.len() != body.boundary.parameters.len()
        || input_names.iter().collect::<HashSet<_>>().len() != input_names.len()
    {
        return Err(ProjectCheck::InvalidProof);
    }
    let expression = derive_expression(&child(&clause, SyntaxKind::Expression)?);
    if !pure_proof_input_expression(&expression) {
        return Err(ProjectCheck::Unsupported);
    }
    let inputs = input_names.iter().zip(&body.boundary.parameters).map(|(name, parameter)| (name.clone(), parameter.ty.clone()));
    typecheck_proof_where(source, target_layout, &expression, inputs).map_err(ProjectCheck::Invalid)?;
    Ok(Some(expression))
}

fn proof_entry_filters(
    expression: &ScalarExpression,
    inputs: &[String],
    body: &FunctionCallSites,
) -> Result<Vec<CheckerPredicate>, ProjectCheck> {
    let mut filters = Vec::new();
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        if let ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. } = expression {
            pending.push(right);
            pending.push(left);
        } else {
            filters.push(proof_input_condition(expression, inputs, body)?);
        }
    }
    Ok(filters)
}

fn proof_input_condition(
    expression: &ScalarExpression,
    inputs: &[String],
    body: &FunctionCallSites,
) -> Result<CheckerPredicate, ProjectCheck> {
    if let ScalarExpression::Boolean { value, .. } = expression {
        return Ok(CheckerPredicate::Boolean(*value));
    }
    let condition = IntegerComparison::from_expression(expression).ok_or(ProjectCheck::Unsupported)?;
    let index = inputs.iter().position(|input| input == condition.name)
        .ok_or(ProjectCheck::InvalidProof)?;
    let ty = &body.boundary.parameters[index].ty;
    if !is_integer_type(ty) {
        return Err(ProjectCheck::Unsupported);
    }
    if !integer_literal_fits_type(condition.bound, ty) {
        return Err(ProjectCheck::InvalidProof);
    }
    let bindings = body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
    let predicate = condition.typed(ty, ty, bindings[index].clone())
        .ok_or(ProjectCheck::Unsupported)?;
    Ok(CheckerPredicate::Integer(predicate))
}

fn proof_show_branch<'a>(
    steps: &'a ScalarBlock,
    inputs: &[String],
    output: &str,
    ty: &ScalarType,
    body: &'a FunctionCallSites,
) -> Result<IntegerBranchProof<'a>, ProjectCheck> {
    let [ScalarBlockItem::Expression(ScalarExpression::If {
        condition, then_branch, else_branch, ..
    })] = steps.items.as_slice() else {
        return Err(ProjectCheck::Unsupported);
    };
    let bindings = body.boundary.checker_parameter_bindings.as_deref().ok_or(ProjectCheck::Unsupported)?;
    if body.assignments.values().flat_map(|group| group.destinations()).any(|destination| {
        match body.prepared_direct_destination(destination.destination) {
            Some(DirectDestinationIdentity::Existing(binding)) => bindings.contains(binding),
            Some(DirectDestinationIdentity::Module(_) | DirectDestinationIdentity::Introduce { .. }) => false,
            None => true,
        }
    }) {
        return Err(ProjectCheck::Unsupported);
    }
    let guard = proof_input_condition(condition, inputs, body)?;
    Ok(IntegerBranchProof {
        guard,
        then_claim: proof_show_claim(then_branch, inputs, output, ty, body, bindings)?,
        else_claim: proof_show_claim(else_branch, inputs, output, ty, body, bindings)?,
    })
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

mod tests {
    use super::*;
    use num_bigint::BigInt;
    use crate::{derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project, ScalarTargetLayout};

    const OUTPUT: &str = r#"{ .stdout = ""; .stderr = "unexpected state"; .code = 1; .files = []; }"#;

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
    fn compiler_test_dependency_accepts_only_the_explicit_proof_fragment_entry() {
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

        let ordinary = crate::parse_source(
            identity,
            format!("%%start\n{fragment}\n%%end"),
            &[],
        );
        assert!(!ordinary.diagnostics.is_empty());
        assert!(!ordinary
            .result
            .canonical_cst()
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::ProofDecl));
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
            ProofModuleInput {
                source: app.clone(),
                namespaces: vec![namespace("lib", lib), namespace("free", &empty)],
                text: app_text.clone(),
            },
            ProofModuleInput { source: lib.clone(), namespaces: Vec::new(), text: lib_text.clone() },
            ProofModuleInput { source: empty.clone(), namespaces: Vec::new(), text: "%%start\nu8(u8) value = fn(x) { x };\n%%end".into() },
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
            vec![ProofModuleInput {
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
        assert_eq!(check_package_roots(&prepared), Err(ProjectCheck::Unsupported));
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
            assert_eq!(check_package_roots(&prepared), expected);
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
    fn canonical_empty_clamp_proof_binds_aliases_by_position() {
        let app = source("src/main.w", "clamp");
        let callable = "%%start\nu8(u8) clamp = fn(value) { if (value > 10) { 10 } else { value } };\n%%end";
        let proof = |clause: &str| format!("proof clamp_bounded for clamp(x) -> (out) {clause} {{}}");
        assert_eq!(prove(&app, callable, &proof("ensures (out <= 10)")), ProjectCheck::Proved);
        assert_eq!(prove(&app, callable, &proof("ensures (out <= 9)")), ProjectCheck::Refuted);
        assert_eq!(prove(&app, callable, &proof("ensures (out <= 256)")), ProjectCheck::InvalidProof);
        assert_eq!(
            prove(&app, callable, "proof clamp_bounded for clamp(x) -> (out) ensures (out <= 10) { core.show(out == x); }"),
            ProjectCheck::Unsupported,
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
        for proof in [
            "proof bounded for identity(x) -> (out) ensures (out <= 10) {}",
            "proof bounded for identity(x) -> (out) where (x > 255) ensures (out <= 10) {}",
            "proof bounded for identity(x) -> (out) where (false) ensures (out <= 10) {}",
            "proof bounded for identity(x) -> (out) ensures (out <= 256) {}",
            "proof bounded for identity(x) -> (out) ensures (out <= 255) { core.show(out == x); }",
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
                assert_eq!(body.structure.checked_types.expressions[&origin.argument_ordinal], ScalarType::U8);
                assert_eq!(origin.argument, super::super::reference::CheckerIntegerCallArgument::Literal(BigInt::from(7)));
                let directive = body.structure.checker_directives.values().next().unwrap();
                let Some(CheckerPredicate::Integer(requirement)) = &directive.predicate else {
                    panic!("expected a typed integer requirement");
                };
                assert_eq!(requirement.binding, origin.binding);
            } else {
                let origin = origin.expect("canonical plain-name argument");
                assert_eq!(origin.argument, super::super::reference::CheckerIntegerCallArgument::PlainName);
            }
            assert_eq!(check_integer_requirements(body), None);
        }
    }

    #[test]
    fn compound_bound_requires_every_guard_at_the_actual_incoming_argument() {
        let app = source("src/main.w", "conjunctive-consumer");
        let proof_source = source("proof/identity.fragment.w", "compound-r1");
        let proof = "proof bounded for identity(x) -> (out) where (x <= 10 && x >= 3 && x != 6) ensures (out <= 10) {}";
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
