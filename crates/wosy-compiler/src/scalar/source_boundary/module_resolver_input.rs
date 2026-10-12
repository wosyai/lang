use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceModuleResolverInputRole {
    InitialModuleResolver,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceModuleResolverInputRef {
    pub owner: ProducerCfgOwnerId,
    pub role: SourceModuleResolverInputRole,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceModuleResolverInputState {
    PendingModuleInvocation,
    NotApplicable,
    OriginalModuleInvocation(Box<SourceModuleResolverInputEvidence>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct FunctionInput {
    name: String,
    name_span: ByteSpan,
    signature: ScalarType,
    parameters: Vec<String>,
    parameter_spans: Vec<ByteSpan>,
    body: ScalarBlock,
    span: ByteSpan,
    generic_parameters: Vec<ScalarGenericParameter>,
    overload_arms: Vec<FunctionInput>,
    owner: ProducerCfgOwnerId,
}

impl FunctionInput {
    fn freeze(function: &ScalarFunction) -> Self {
        Self {
            name: function.name.clone(), name_span: function.name_span,
            signature: function.signature.clone(), parameters: function.parameters.clone(),
            parameter_spans: function.parameter_spans.clone(), body: function.body.clone(),
            span: function.span, generic_parameters: function.generic_parameters.clone(),
            overload_arms: function.overload_arms.iter().map(Self::freeze).collect(),
            owner: function.reference_cfg.producer_owner.clone(),
        }
    }

    fn materialize(&self) -> ScalarFunction {
        ScalarFunction {
            name: self.name.clone(), name_span: self.name_span, signature: self.signature.clone(),
            parameters: self.parameters.clone(), parameter_spans: self.parameter_spans.clone(),
            body: self.body.clone(), span: self.span, generic_parameters: self.generic_parameters.clone(),
            overload_arms: self.overload_arms.iter().map(Self::materialize).collect(),
            reference_cfg: ScalarReferenceCfg::new(self.owner.clone()),
        }
    }

    fn validate(&self, source: &SourceIdentity) -> Result<(), ReferenceAnalysisError> {
        if !matches!(&self.owner, ProducerCfgOwnerId::Function { source: actual, declaration, .. }
            if actual == source && *declaration == self.name_span) { return Err(failure(source)); }
        for arm in &self.overload_arms { arm.validate(source)?; }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ItemInput {
    Namespace(ScalarNamespace),
    Extern(ScalarExtern),
    Binding(ScalarBinding),
    Function(FunctionInput),
    Executable(ScalarBlockItem),
}

impl ItemInput {
    fn freeze(item: &ScalarItem) -> Self {
        match item {
            ScalarItem::Namespace(value) => Self::Namespace(value.clone()),
            ScalarItem::Extern(value) => Self::Extern(value.clone()),
            ScalarItem::Binding(value) => Self::Binding(value.clone()),
            ScalarItem::Function(value) => Self::Function(FunctionInput::freeze(value)),
            ScalarItem::Executable(value) => Self::Executable(value.clone()),
        }
    }

    fn materialize(&self) -> ScalarItem {
        match self {
            Self::Namespace(value) => ScalarItem::Namespace(value.clone()),
            Self::Extern(value) => ScalarItem::Extern(value.clone()),
            Self::Binding(value) => ScalarItem::Binding(value.clone()),
            Self::Function(value) => ScalarItem::Function(value.materialize()),
            Self::Executable(value) => ScalarItem::Executable(value.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct TypeEpoch {
    source: SourceIdentity,
    literals: Vec<closure::ResolvedLiteralSignature>,
    assignments: Vec<ResolvedAssignmentTypeFact>,
    expressions: Vec<ResolvedExpressionOutputFact>,
    assignment_outputs: Vec<(ResolvedAssignmentOutputId, ScalarType)>,
}

impl TypeEpoch {
    fn freeze(module: &ScalarModule) -> Self {
        let mut assignment_outputs = module.resolved_assignment_outputs.borrow().iter()
            .map(|(id, ty)| (id.clone(), ty.clone())).collect::<Vec<_>>();
        assignment_outputs.sort_by_key(|(id, _)| (id.source.project.clone(), id.source.package.clone(),
            id.source.path.clone(), id.source.revision.clone(), id.function_span.start,
            id.function_span.end, id.target_span.start, id.target_span.end, id.output_index));
        Self {
            source: module.source.clone(), literals: module.resolved_literals.borrow().clone(),
            assignments: module.resolved_assignment_types.borrow().clone(),
            expressions: module.resolved_expression_outputs.borrow().clone(), assignment_outputs,
        }
    }

    fn validate(&self) -> Result<(), ReferenceAnalysisError> {
        let outputs = self.assignment_outputs.iter().cloned().collect::<HashMap<_, _>>();
        if outputs.len() != self.assignment_outputs.len() { return Err(failure(&self.source)); }
        let mut ordered = self.assignment_outputs.clone();
        ordered.sort_by_key(|(id, _)| (id.source.project.clone(), id.source.package.clone(),
            id.source.path.clone(), id.source.revision.clone(), id.function_span.start,
            id.function_span.end, id.target_span.start, id.target_span.end, id.output_index));
        if ordered != self.assignment_outputs { return Err(failure(&self.source)); }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ModuleInput {
    source: SourceIdentity,
    items: Vec<ItemInput>,
    namespace_bindings: Vec<ScalarNamespaceBinding>,
    members: BTreeMap<String, ScalarType>,
    initialization_nodes: Vec<ScalarInitializationNode>,
    structs: Vec<ScalarStruct>,
    enums: Vec<ScalarEnum>,
    target_layout: ScalarTargetLayout,
    types: TypeEpoch,
    owner: ProducerCfgOwnerId,
}

impl ModuleInput {
    fn freeze(module: &ScalarModule) -> Self {
        Self {
            source: module.source.clone(), items: module.items.iter().map(ItemInput::freeze).collect(),
            namespace_bindings: module.namespace_bindings.clone(), members: module.members.clone(),
            initialization_nodes: module.initialization_nodes.clone(), structs: module.structs.clone(),
            enums: module.enums.clone(), target_layout: module.target_layout,
            types: TypeEpoch::freeze(module), owner: module.top_level_reference_cfg.producer_owner.clone(),
        }
    }

    fn materialize(&self) -> ScalarModule {
        ScalarModule {
            source: self.source.clone(), items: self.items.iter().map(ItemInput::materialize).collect(),
            namespace_bindings: self.namespace_bindings.clone(), members: self.members.clone(),
            initialization_nodes: self.initialization_nodes.clone(), structs: self.structs.clone(),
            enums: self.enums.clone(), target_layout: self.target_layout,
            resolved_literals: RefCell::new(self.types.literals.clone()),
            resolved_assignment_types: RefCell::new(self.types.assignments.clone()),
            resolved_expression_outputs: RefCell::new(self.types.expressions.clone()),
            resolved_assignment_outputs: RefCell::new(self.types.assignment_outputs.iter().cloned().collect()),
            top_level_reference_cfg: ScalarReferenceCfg::new(self.owner.clone()), terminal_owners: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::scalar) struct SourceModuleResolverPreInput {
    source: SourceIdentity,
    items: Vec<ItemInput>,
    modules: Vec<ModuleInput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceModuleResolverInputEvidence {
    invocation: SourceModuleResolverInputRef,
    pre: SourceModuleResolverPreInput,
    result: Result<(Vec<ResolvedCallSite>, Vec<(ScalarCaptureOccurrence, CfgPointKind)>), ReferenceAnalysisError>,
    post: Vec<TypeEpoch>,
    pub(super) topology: module_source_topology::SourceModuleTopologyState,
}

fn failure(source: &SourceIdentity) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: SourceSpan::new(source.clone(), ByteSpan::new(0, 0)),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

pub(in crate::scalar) fn freeze(source: &SourceIdentity, items: &[ScalarItem], modules: &[ScalarModule]) -> SourceModuleResolverPreInput {
    SourceModuleResolverPreInput { source: source.clone(), items: items.iter().map(ItemInput::freeze).collect(),
        modules: modules.iter().map(ModuleInput::freeze).collect() }
}

pub(in crate::scalar) fn finish(pre: SourceModuleResolverPreInput,
    result: &Result<(Vec<ResolvedCallSite>, Vec<(ScalarCaptureOccurrence, CfgPointKind)>), ReferenceAnalysisError>,
    modules: &[ScalarModule]) -> SourceModuleResolverInputEvidence {
    SourceModuleResolverInputEvidence {
        invocation: SourceModuleResolverInputRef { owner: ProducerCfgOwnerId::Module(pre.source.clone()),
            role: SourceModuleResolverInputRole::InitialModuleResolver },
        pre, result: result.clone(), post: modules.iter().map(TypeEpoch::freeze).collect(),
        topology: module_source_topology::SourceModuleTopologyState::AwaitingConnectedConstruction,
    }
}

impl SourceModuleResolverInputEvidence {
    pub(in crate::scalar) fn validate(&self) -> Result<(), ReferenceAnalysisError> {
        let source = &self.pre.source;
        if self.invocation != (SourceModuleResolverInputRef { owner: ProducerCfgOwnerId::Module(source.clone()),
            role: SourceModuleResolverInputRole::InitialModuleResolver }) { return Err(failure(source)); }
        let identities = self.pre.modules.iter().map(|module| module.source.clone()).collect::<HashSet<_>>();
        if identities.len() != self.pre.modules.len() || !identities.contains(source)
            || self.post.len() != self.pre.modules.len() { return Err(failure(source)); }
        for (module, post) in self.pre.modules.iter().zip(&self.post) {
            if module.owner != ProducerCfgOwnerId::Module(module.source.clone()) || module.types.source != module.source
                || post.source != module.source { return Err(failure(source)); }
            module.types.validate()?;
            post.validate()?;
            for binding in &module.namespace_bindings {
                if !identities.contains(&binding.target) { return Err(failure(source)); }
            }
            for item in &module.items {
                if let ItemInput::Function(function) = item { function.validate(&module.source)?; }
            }
        }
        for item in &self.pre.items {
            if let ItemInput::Function(function) = item { function.validate(source)?; }
        }
        let modules = self.pre.modules.iter().map(ModuleInput::materialize).collect::<Vec<_>>();
        let items = self.pre.items.iter().map(ItemInput::materialize).collect::<Vec<_>>();
        let result = reference::resolve_module_call_sites(source, &items, &modules);
        if result != self.result || modules.iter().map(TypeEpoch::freeze).collect::<Vec<_>>() != self.post {
            return Err(failure(source));
        }
        Ok(())
    }

    pub(in crate::scalar) fn expression_epoch(&self, source: &SourceIdentity) -> Result<&[ResolvedExpressionOutputFact], ReferenceAnalysisError> {
        self.validate()?;
        let epochs = self.post.iter().filter(|epoch| epoch.source == *source).collect::<Vec<_>>();
        let [epoch] = epochs.as_slice() else { return Err(failure(source)); };
        Ok(epoch.expressions.as_slice())
    }

    pub(in crate::scalar) fn assignment_epoch(&self, source: &SourceIdentity) -> Result<&[ResolvedAssignmentTypeFact], ReferenceAnalysisError> {
        self.validate()?;
        let epochs = self.post.iter().filter(|epoch| epoch.source == *source).collect::<Vec<_>>();
        let [epoch] = epochs.as_slice() else { return Err(failure(source)); };
        Ok(epoch.assignments.as_slice())
    }

    pub(in crate::scalar) fn same_original_invocation(&self, other: &Self) -> bool {
        self.invocation == other.invocation && self.pre == other.pre && self.result == other.result && self.post == other.post
    }

    pub(in crate::scalar) fn named_root(&self, owner: &ProducerCfgOwnerId) -> Result<reference::name_resolution::SourceNameFunctionRoot, ReferenceAnalysisError> {
        self.validate()?;
        let ProducerCfgOwnerId::Function { source, .. } = owner else { return Err(failure(&self.pre.source)); };
        if *source != self.pre.source { return Err(failure(source)); }
        fn collect<'a>(function: &'a FunctionInput, owner: &ProducerCfgOwnerId, matches: &mut Vec<&'a FunctionInput>) {
            if function.owner == *owner { matches.push(function); }
            for arm in &function.overload_arms { collect(arm, owner, matches); }
        }
        let mut matches = Vec::new();
        for item in &self.pre.items {
            if let ItemInput::Function(function) = item { collect(function, owner, &mut matches); }
        }
        let [function] = matches.as_slice() else { return Err(failure(source)); };
        Ok(reference::name_resolution::SourceNameFunctionRoot {
            name: function.name.clone(), name_span: function.name_span, span: function.span,
            signature: function.signature.clone(), parameters: function.parameters.clone(),
            parameter_spans: function.parameter_spans.clone(), generic_declarations: function.generic_parameters.clone(),
            body: function.body.clone(),
        })
    }

    pub(in crate::scalar) fn policy_context(&self, owner: &ProducerCfgOwnerId) -> Result<SourceCapturePolicyContext, ReferenceAnalysisError> {
        self.validate()?;
        let mut scope = owner;
        while let ProducerCfgOwnerId::Literal { enclosing, .. } = scope { scope = enclosing; }
        let source = match scope {
            ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
            ProducerCfgOwnerId::Literal { .. } => return Err(failure(&self.pre.source)),
        };
        if *source != self.pre.source { return Err(failure(source)); }
        let modules = self.pre.modules.iter().filter(|module| module.source == *source).collect::<Vec<_>>();
        let [module] = modules.as_slice() else { return Err(failure(source)); };
        let generic_declarations = match scope {
            ProducerCfgOwnerId::Module(_) => Vec::new(),
            ProducerCfgOwnerId::Function { .. } => self.named_root(scope)?.generic_declarations,
            ProducerCfgOwnerId::Literal { .. } => return Err(failure(source)),
        };
        let generic_parameters = generic_declarations.iter().map(|row| row.name.clone()).collect::<BTreeSet<_>>();
        if generic_parameters.len() != generic_declarations.len() { return Err(failure(source)); }
        let context = SourceCapturePolicyContext {
            structs: module.structs.clone(), enums: module.enums.clone(), generic_scope: scope.clone(),
            generic_declarations, generic_parameters,
        };
        if !evaluated_value::template::valid_context(&context) { return Err(failure(source)); }
        Ok(context)
    }

    pub(in crate::scalar) fn inventory(&self) -> Result<(SourceModuleResolverInputRef, Vec<ScalarItem>, Vec<ResolvedCallSite>, Vec<(ScalarCaptureOccurrence, CfgPointKind)>), ReferenceAnalysisError> {
        self.validate()?;
        self.result.clone().map(|(calls, captures)| (self.invocation.clone(), self.pre.items.iter().map(ItemInput::materialize).collect(), calls, captures))
    }

    pub(in crate::scalar) fn install(self, cfg: &mut ScalarReferenceCfg, source: &SourceIdentity,
        items: &[ScalarItem], sites: &[ResolvedCallSite], captures: &[(ScalarCaptureOccurrence, CfgPointKind)]) -> Result<(), ReferenceAnalysisError> {
        if cfg.producer_owner != ProducerCfgOwnerId::Module(source.clone())
            || cfg.module_resolver_input != SourceModuleResolverInputState::PendingModuleInvocation
            || self.pre.source != *source || self.pre.items != items.iter().map(ItemInput::freeze).collect::<Vec<_>>()
            || self.result != Ok((sites.to_vec(), captures.to_vec())) { return Err(failure(source)); }
        self.validate()?;
        cfg.module_resolver_input = SourceModuleResolverInputState::OriginalModuleInvocation(Box::new(self));
        Ok(())
    }
}

pub(in crate::scalar) fn validate_endpoint(cfg: &ScalarReferenceCfg) -> Result<(), ValueSchemaRemapError> {
    let fail = || ValueSchemaRemapError::MissingProducer(cfg.producer_owner.clone());
    match (&cfg.producer_owner, &cfg.module_resolver_input) {
        (ProducerCfgOwnerId::Module(source), SourceModuleResolverInputState::OriginalModuleInvocation(evidence)) => {
            if evidence.pre.source != *source || evidence.result.is_err() { return Err(fail()); }
            evidence.validate().map_err(|_| fail())?;
            module_source_topology::validate(cfg).map_err(|_| fail())
        },
        (ProducerCfgOwnerId::Function { .. } | ProducerCfgOwnerId::Literal { .. }, SourceModuleResolverInputState::NotApplicable) => Ok(()),
        _ => Err(fail()),
    }
}

#[cfg(test)]
mod tests;
