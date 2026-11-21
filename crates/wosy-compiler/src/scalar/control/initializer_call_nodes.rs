use std::marker::PhantomData;

use super::super::reference::{
    BlockOrdinal, FunctionCallSites, OccurrenceEdge as Edge, OccurrenceNode as Node, OccurrenceParent as Parent,
    OccurrenceRole,
};
use super::super::typecheck::expression_span;
use super::super::{
    ResolvedCallTarget, ScalarBindingOutput, ScalarBindingOutputOrigin, ScalarBlock,
    ScalarBlockItem, ScalarBlockItemRole, ScalarExpression, ScalarPlace,
};
use super::*;

#[cfg(test)]
mod initializer_facts_tests;

pub(crate) struct InitializerCallNodes<'a> {
    source: &'a SourceIdentity,
    targets: HashMap<*const ScalarExpression, (usize, &'a ModuleInitializationCallTarget)>,
    members: HashMap<*const ScalarExpression, BoundInitializerMember<'a>>,
    float_types: HashMap<*const ScalarExpression, &'a ScalarType>,
    items: PhantomData<&'a [ScalarItem]>,
}

#[derive(Clone, Copy)]
pub(crate) struct BoundInitializerMember<'a> {
    pub(crate) identity: &'a InitializationMember,
    pub(crate) ty: &'a ScalarType,
}

impl<'a> InitializerCallNodes<'a> {
    pub(super) fn bind(
        region: &'a ModuleInitializationRegion,
        items: &'a [ScalarItem],
    ) -> Result<Self, String> {
        let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
            return Err(format!(
                "internal error: missing initializer call topology for {:?}",
                region.source,
            ));
        };
        let mut nodes = Self {
            source: &region.source,
            targets: HashMap::with_capacity(topology.calls.len()),
            members: HashMap::with_capacity(topology.identities.members.len()),
            float_types: HashMap::new(),
            items: PhantomData,
        };
        let mut bind_call = |ordinal: OccurrenceOrdinal, expression: &'a ScalarExpression| {
            if matches!(expression, ScalarExpression::Member { .. }) {
                use super::super::reference::InitializerMemberRequirement;
                let InitializerMemberRequirement::Token(span) =
                    topology.occurrences[ordinal.0].initializer_member
                else {
                    return Err(format!(
                        "internal error: missing initializer member token in {:?} (occurrence {})",
                        region.source, ordinal.0
                    ));
                };
                let missing = |fact| {
                    format!("internal error: missing initializer member {fact} in {:?} at {}..{} (occurrence {})", region.source, span.start, span.end, ordinal.0)
                };
                let identity = topology
                    .identities
                    .members
                    .get(&ordinal)
                    .ok_or_else(|| missing("identity"))?;
                let ty = topology
                    .structure
                    .checked_types
                    .expressions
                    .get(&ordinal)
                    .ok_or_else(|| missing("type"))?;
                nodes.members.insert(
                    std::ptr::from_ref(expression),
                    BoundInitializerMember { identity, ty },
                );
                return Ok(());
            }
            let target = topology
                .call_targets
                .get(&ordinal)
                .ok_or_else(|| topology.missing_call_target(&region.source, ordinal))?;
            nodes.targets.insert(std::ptr::from_ref(expression), (ordinal.0, target));
            Ok(())
        };
        let mut bind_type = |ordinal, expression: &'a ScalarExpression| {
            if matches!(expression, ScalarExpression::Float { .. }) {
                if let Some(ty @ (ScalarType::F32 | ScalarType::F64)) =
                    topology.structure.checked_types.expressions.get(&ordinal)
                {
                    nodes.float_types.insert(std::ptr::from_ref(expression), ty);
                }
            }
        };
        Binder {
            structure: &topology.structure,
            source: &region.source,
            region: "initializer",
            bind_call: &mut bind_call,
            bind_type: Some(&mut bind_type),
            bind_named_place: None,
            proof_nodes: None,
        }
        .initializer_items(region, items)?;
        if nodes.targets.len() != topology.calls.len() {
            return Err(format!(
                "internal error: initializer call topology does not match canonical items in {:?}",
                region.source,
            ));
        }
        Ok(nodes)
    }

    pub(crate) fn checked_float_type(&self, expression: &ScalarExpression) -> Option<&'a ScalarType> {
        self.float_types.get(&std::ptr::from_ref(expression)).copied()
    }

    pub(crate) fn target(
        &self,
        expression: &ScalarExpression,
    ) -> Result<&'a ModuleInitializationCallTarget, String> {
        self.bound_call(expression).map(|(_, target)| target)
    }

    pub(crate) fn bound_call(
        &self,
        expression: &ScalarExpression,
    ) -> Result<(usize, &'a ModuleInitializationCallTarget), String> {
        self.targets
            .get(&std::ptr::from_ref(expression))
            .copied()
            .ok_or_else(|| {
                let span = expression_span(expression);
                format!(
                    "internal error: missing canonical initializer call node in {:?} at {}..{}",
                    self.source, span.start, span.end,
                )
            })
    }

    pub(crate) fn member(
        &self,
        expression: &ScalarExpression,
    ) -> Result<BoundInitializerMember<'a>, String> {
        self.members
            .get(&std::ptr::from_ref(expression))
            .copied()
            .ok_or_else(|| {
                let ScalarExpression::Member { name_span, .. } = expression else {
                    unreachable!("member expression")
                };
                format!(
                    "internal error: missing canonical initializer member node in {:?} at {}..{}",
                    self.source, name_span.start, name_span.end
                )
            })
    }
}
use crate::scalar::reference::self_tail::checked::CheckedForwardingProof;
use crate::scalar::{CfgPointId, ScalarReferenceCfg};

pub(crate) struct CallableBodyCallNodes<'a> {
    source: &'a SourceIdentity,
    targets: HashMap<*const ScalarExpression, &'a ResolvedCallTarget>,
    self_tail_candidates: HashMap<*const ScalarExpression, &'a ScalarExpression>,
    checked_self_tail: Option<(&'a ScalarExpression, CheckedForwardingProof)>,
    expression_types: HashMap<*const ScalarExpression, &'a ScalarType>,
    named_place_roots: HashMap<*const ScalarPlace, &'a NamedPlaceRootIdentity>,
    proof_nodes: CallableProofNodes<'a>,
    scalar_self_tail_types: bool,
    body: PhantomData<&'a ScalarBlock>,
}

#[derive(Default)]
struct CallableProofNodes<'a> {
    calls: Option<&'a FunctionCallSites>,
    initializer_bindings: bool,
    expressions: HashMap<*const ScalarExpression, OccurrenceOrdinal>,
    places: HashMap<*const ScalarPlace, OccurrenceOrdinal>,
    bindings: HashMap<*const ScalarBinding, BlockPosition>,
    blocks: HashMap<*const ScalarBlock, BlockOrdinal>,
    final_outputs: HashMap<*const ScalarExpression, usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InitializerFactsError {
    Noncanonical,
    Missing(&'static str),
    Malformed(&'static str),
    Unsupported(&'static str),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QualifiedInitializerStorage {
    Lexical {
        declaration: SourceSpan,
        block_span: ByteSpan,
    },
    Module(SourceSpan),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InitializerDestinationFacts {
    Existing(QualifiedInitializerStorage),
    Introduce(QualifiedInitializerStorage),
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct CheckedInitializerCall<'a> {
    pub(crate) declaration: &'a SourceSpan,
    pub(crate) concrete: &'a ConcreteCallSelection,
    pub(crate) parameters: &'a [ScalarType],
    pub(crate) outputs: &'a [crate::ScalarOutput],
}

#[allow(dead_code)]
pub(crate) struct InitializerFacts<'a> {
    source: &'a SourceIdentity,
    region: &'a ModuleInitializationRegion,
    items: &'a [ScalarItem],
    topology: &'a ModuleInitializationTopology,
    nodes: CallableProofNodes<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InitializerOutputRoute {
    Primary,
    EvaluateSecondary,
    ProjectPrimary { output: usize },
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct InitializerBindingOutput<'a> {
    pub(crate) destination: SourceSpan,
    pub(crate) ty: &'a ScalarType,
    pub(crate) route: InitializerOutputRoute,
}

#[allow(dead_code)]
impl<'a> InitializerFacts<'a> {
    pub(in crate::scalar) fn bind(
        region: &'a ModuleInitializationRegion,
        items: &'a [ScalarItem],
    ) -> Result<Self, InitializerFactsError> {
        use InitializerFactsError as Error;
        let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
            return Err(Error::Missing("initializer topology"));
        };
        let mut nodes = CallableProofNodes {
            initializer_bindings: true,
            ..Default::default()
        };
        let mut no_calls = |_: OccurrenceOrdinal, _: &'a ScalarExpression| Ok(());
        Binder {
            structure: &topology.structure,
            source: &region.source,
            region: "initializer facts",
            bind_call: &mut no_calls,
            bind_type: None,
            bind_named_place: None,
            proof_nodes: Some(&mut nodes),
        }
        .initializer_items(region, items)
        .map_err(|_| Error::Malformed("initializer topology"))?;
        Ok(Self {
            source: &region.source,
            region,
            items,
            topology,
            nodes,
        })
    }

    fn root_binding_item(&self, binding: &ScalarBinding) -> Result<usize, InitializerFactsError> {
        use InitializerFactsError as Error;
        let position = self
            .nodes
            .bindings
            .get(&std::ptr::from_ref(binding))
            .ok_or(Error::Noncanonical)?;
        if position.block != self.topology.structure.root
            || !matches!(self.items.get(position.item), Some(ScalarItem::Binding(stored)) if std::ptr::eq(stored, binding))
        {
            return Err(Error::Unsupported("initializer root declaration"));
        }
        Ok(position.item)
    }

    fn root_kind(&self, item: usize) -> Result<&'a BlockItemKind, InitializerFactsError> {
        use InitializerFactsError as Error;
        let structure = &self.topology.structure;
        let mut rows = structure
            .blocks
            .get(structure.root.0)
            .ok_or(Error::Missing("initializer root block"))?
            .items
            .iter()
            .filter(|row| row.position == item);
        let row = rows.next().ok_or(Error::Missing("initializer root item"))?;
        if rows.next().is_some() {
            return Err(Error::Malformed("initializer root item"));
        }
        Ok(&row.kind)
    }

    fn root_edge(
        &self,
        edge: Edge,
        expression: &ScalarExpression,
    ) -> Result<(), InitializerFactsError> {
        use InitializerFactsError as Error;
        let ordinal = self.expression(expression)?;
        let parent = Parent::Block(self.topology.structure.root);
        if self.topology.structure.edges.get(&(parent, edge)) != Some(&Node::Expression(ordinal))
            || self.metadata(ordinal)?.span != expression_span(expression)
        {
            return Err(Error::Malformed("initializer root expression"));
        }
        Ok(())
    }

    pub(crate) fn binding_output(
        &self,
        binding: &ScalarBinding,
        output: usize,
        call: &ScalarExpression,
    ) -> Result<InitializerBindingOutput<'a>, InitializerFactsError> {
        use InitializerFactsError as Error;
        let item = self.root_binding_item(binding)?;
        let BlockItemKind::Binding(region) = self.root_kind(item)? else {
            return Err(Error::Malformed("initializer binding row"));
        };
        let BindingInitialization::Outputs(outputs) = &region.initialization else {
            return Err(Error::Unsupported("initializer allocation binding"));
        };
        let arity = binding.receivers.len().max(1);
        if region.receivers().len() != arity || outputs.secondary.len() != arity - 1 {
            return Err(Error::Malformed("initializer receiver arity"));
        }
        let slot = region
            .receivers()
            .get(output)
            .ok_or(Error::Missing("initializer receiver slot"))?;
        let (name, span, ty) = if output == 0 {
            (&binding.name, binding.name_span, &binding.declared_type)
        } else {
            let receiver = &binding.receivers[output];
            (&receiver.name, receiver.name_span, &receiver.ty)
        };
        if slot.position != output || slot.name != *name || slot.span != span || slot.ty != *ty {
            return Err(Error::Malformed("initializer receiver slot"));
        }
        let (evaluation, ordinal, route) = if output == 0 {
            (0, outputs.primary, InitializerOutputRoute::Primary)
        } else {
            match outputs.secondary.get(output - 1) {
                Some(BindingOutput::Evaluate {
                    position,
                    expression,
                }) if *position == output => (
                    output,
                    *expression,
                    InitializerOutputRoute::EvaluateSecondary,
                ),
                Some(BindingOutput::ProjectPrimary { output: stored }) if *stored == output => (
                    0,
                    outputs.primary,
                    InitializerOutputRoute::ProjectPrimary { output },
                ),
                _ => return Err(Error::Malformed("initializer output route")),
            }
        };
        if output > 0
            && binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions
            && binding.output_values.get(output).is_none()
        {
            return Err(Error::Malformed("initializer secondary expression"));
        }
        let ast = match (route, binding.output_source(output)) {
            (
                InitializerOutputRoute::Primary | InitializerOutputRoute::EvaluateSecondary,
                ScalarBindingOutput::Evaluate(value),
            ) => value,
            (
                InitializerOutputRoute::ProjectPrimary { output: route },
                ScalarBindingOutput::ProjectPrimary { output: ast },
            ) if route == ast => &binding.value,
            _ => return Err(Error::Malformed("initializer output origin")),
        };
        if !std::ptr::eq(ast, call)
            || self.expression(call)? != ordinal
            || !matches!(call, ScalarExpression::Call { .. })
        {
            return Err(Error::Noncanonical);
        }
        self.root_edge(
            Edge::BindingRoot {
                item,
                output: evaluation,
            },
            call,
        )?;
        Ok(InitializerBindingOutput {
            destination: SourceSpan::new(self.source.clone(), slot.span),
            ty: &slot.ty,
            route,
        })
    }

    pub(crate) fn immediately_dispatched_call(
        &self,
        binding: &ScalarBinding,
        call: &ScalarExpression,
    ) -> Result<(), InitializerFactsError> {
        use InitializerFactsError as Error;
        let item = self.root_binding_item(binding)?;
        let mut rows = self
            .region
            .steps
            .iter()
            .enumerate()
            .filter(|(_, step)| step.item == item);
        let (index, step) = rows
            .next()
            .ok_or(Error::Missing("initializer binding dispatch"))?;
        if rows.next().is_some() || step.kind != ModuleInitializationStepKind::BindingInitialization
        {
            return Err(Error::Malformed("initializer binding dispatch"));
        }
        let next = self
            .region
            .steps
            .get(index + 1)
            .ok_or(Error::Missing("initializer next dispatch"))?;
        if next.kind != ModuleInitializationStepKind::ExecutableItem
            || !matches!(self.items.get(next.item), Some(ScalarItem::Executable(ScalarBlockItem::Expression(stored))) if std::ptr::eq(stored, call) && matches!(stored, ScalarExpression::Call { .. }))
        {
            return Err(Error::Unsupported("initializer next direct call"));
        }
        if self
            .region
            .steps
            .iter()
            .filter(|step| step.item == next.item)
            .count()
            != 1
            || !matches!(self.root_kind(next.item)?, BlockItemKind::Evaluation { expression } if *expression == self.expression(call)?)
        {
            return Err(Error::Malformed("initializer call dispatch"));
        }
        self.root_edge(Edge::ExpressionItem { item: next.item }, call)
    }

    fn expression(
        &self,
        expression: &ScalarExpression,
    ) -> Result<OccurrenceOrdinal, InitializerFactsError> {
        let ordinal = self
            .nodes
            .expressions
            .get(&std::ptr::from_ref(expression))
            .copied()
            .ok_or(InitializerFactsError::Noncanonical)?;
        if self.metadata(ordinal)?.role != OccurrenceRole::Expression {
            return Err(InitializerFactsError::Malformed(
                "initializer expression occurrence",
            ));
        }
        Ok(ordinal)
    }

    fn place(&self, place: &ScalarPlace) -> Result<OccurrenceOrdinal, InitializerFactsError> {
        let ordinal = self
            .nodes
            .places
            .get(&std::ptr::from_ref(place))
            .copied()
            .ok_or(InitializerFactsError::Noncanonical)?;
        if self.metadata(ordinal)?.role != OccurrenceRole::Place {
            return Err(InitializerFactsError::Malformed(
                "initializer place occurrence",
            ));
        }
        Ok(ordinal)
    }

    fn metadata(
        &self,
        ordinal: OccurrenceOrdinal,
    ) -> Result<&OccurrenceMetadata, InitializerFactsError> {
        self.topology
            .occurrences
            .get(ordinal.0)
            .ok_or(InitializerFactsError::Malformed("initializer occurrence"))
    }

    pub(crate) fn checked_direct_call(
        &self,
        expression: &ScalarExpression,
    ) -> Result<CheckedInitializerCall<'a>, InitializerFactsError> {
        use InitializerFactsError as Error;
        let ordinal = self.expression(expression)?;
        let ScalarExpression::Call { arguments, .. } = expression else {
            return Err(Error::Unsupported("initializer call shape"));
        };
        let target = self
            .topology
            .call_targets
            .get(&ordinal)
            .ok_or(Error::Missing("initializer call target"))?;
        let ModuleInitializationCallTarget::Direct {
            declaration,
            concrete,
        } = target
        else {
            return Err(Error::Unsupported("initializer callable target"));
        };
        if matches!(concrete, ConcreteCallSelection::Overload(selected) if !selected.substitutions.is_empty())
        {
            return Err(Error::Unsupported("initializer call specialization"));
        }
        let signature = self
            .topology
            .checked_call_signatures
            .get(&ordinal)
            .ok_or(Error::Missing("initializer call signature"))?;
        let consistent = match concrete {
            ConcreteCallSelection::Declaration => signature.overload.is_none(),
            ConcreteCallSelection::Overload(selected) => {
                signature.overload.as_ref() == Some(selected)
            }
        };
        if !consistent {
            return Err(Error::Malformed("initializer call selection"));
        }
        if signature.parameters.len() != arguments.len() {
            return Err(Error::Malformed("initializer call arity"));
        }
        Ok(CheckedInitializerCall {
            declaration,
            concrete,
            parameters: &signature.parameters,
            outputs: &signature.outputs.outputs,
        })
    }

    pub(crate) fn checked_expression_type(
        &self,
        expression: &ScalarExpression,
    ) -> Result<&'a ScalarType, InitializerFactsError> {
        let ordinal = self.expression(expression)?;
        self.topology
            .structure
            .checked_types
            .expressions
            .get(&ordinal)
            .ok_or(InitializerFactsError::Missing(
                "initializer expression type",
            ))
    }

    fn qualify(&self, storage: InitializationStorage) -> QualifiedInitializerStorage {
        match storage {
            InitializationStorage::Module(span) => {
                QualifiedInitializerStorage::Module(SourceSpan::new(self.source.clone(), span))
            }
            InitializationStorage::Lexical(local) => QualifiedInitializerStorage::Lexical {
                declaration: SourceSpan::new(self.source.clone(), local.declaration_span),
                block_span: local.block_span,
            },
        }
    }

    pub(crate) fn plain_name(
        &self,
        expression: &ScalarExpression,
    ) -> Result<QualifiedInitializerStorage, InitializerFactsError> {
        use InitializerFactsError as Error;
        let ordinal = self.expression(expression)?;
        if !matches!(expression, ScalarExpression::Name { .. })
            || !self.metadata(ordinal)?.requires_plain_name
        {
            return Err(Error::Unsupported("initializer plain name shape"));
        }
        match self.topology.identities.names.get(&ordinal) {
            Some(InitializationName::Storage(storage)) => Ok(self.qualify(*storage)),
            Some(InitializationName::Null) => Err(Error::Unsupported("initializer null")),
            None => Err(Error::Missing("initializer plain name")),
        }
    }

    pub(crate) fn named_place_root(
        &self,
        place: &ScalarPlace,
    ) -> Result<QualifiedInitializerStorage, InitializerFactsError> {
        use InitializerFactsError as Error;
        let ordinal = self.place(place)?;
        if !matches!(place, ScalarPlace::Name { .. })
            || !self.metadata(ordinal)?.requires_named_place_root
            || self
                .topology
                .structure
                .resolved_namespace_places
                .contains_key(&ordinal)
        {
            return Err(Error::Unsupported("initializer named root shape"));
        }
        let storage = self
            .topology
            .identities
            .roots
            .get(&ordinal)
            .ok_or(Error::Missing("initializer named root"))?;
        Ok(self.qualify(*storage))
    }

    pub(crate) fn direct_assignment_destination(
        &self,
        place: &ScalarPlace,
    ) -> Result<InitializerDestinationFacts, InitializerFactsError> {
        use InitializerFactsError as Error;
        let ordinal = self.place(place)?;
        if !matches!(place, ScalarPlace::Name { .. })
            || !self.metadata(ordinal)?.requires_direct_destination
        {
            return Err(Error::Unsupported(
                "initializer assignment destination shape",
            ));
        }
        let destination = self
            .topology
            .identities
            .destinations
            .get(&ordinal)
            .ok_or(Error::Missing("initializer assignment destination"))?;
        Ok(match *destination {
            InitializationDestination::Existing(storage) => {
                InitializerDestinationFacts::Existing(self.qualify(storage))
            }
            InitializationDestination::Introduce(local) => InitializerDestinationFacts::Introduce(
                self.qualify(InitializationStorage::Lexical(local)),
            ),
            InitializationDestination::IntroduceModule(span) => {
                InitializerDestinationFacts::Introduce(
                    self.qualify(InitializationStorage::Module(span)),
                )
            }
        })
    }
}

impl<'a> CallableBodyCallNodes<'a> {
    pub(crate) fn block_ordinal(&self, block: &ScalarBlock) -> Option<usize> {
        self.proof_nodes.blocks.get(&std::ptr::from_ref(block)).map(|ordinal| ordinal.0)
    }

    pub(crate) fn expression_ordinal(&self, expression: &ScalarExpression) -> Option<usize> {
        self.proof_nodes
            .expressions
            .get(&std::ptr::from_ref(expression))
            .map(|ordinal| ordinal.0)
    }

    pub(crate) fn place_ordinal(&self, place: &ScalarPlace) -> Option<usize> {
        self.proof_nodes.places.get(&std::ptr::from_ref(place)).map(|ordinal| ordinal.0)
    }

    pub(crate) fn binding_position(&self, binding: &ScalarBinding) -> Option<(usize, usize)> {
        self.proof_nodes
            .bindings
            .get(&std::ptr::from_ref(binding))
            .map(|position| (position.block.0, position.item))
    }

    pub(in crate::scalar) fn bind(
        source: &'a SourceIdentity,
        body: &'a ScalarBlock,
        calls: &'a FunctionCallSites,
        checked_proof: Option<CheckedForwardingProof>,
    ) -> Result<Self, String> {
        let targets_by_ordinal: HashMap<_, _> = calls
            .call_occurrences
            .iter()
            .filter_map(|call| Some((call.ordinal, &calls.sites.get(call.site)?.target)))
            .chain(
                calls
                    .allocation_length_call_targets
                    .iter()
                    .map(|(&ordinal, target)| (ordinal, target)),
            )
            .collect();
        let mut nodes = Self {
            source,
            targets: HashMap::with_capacity(targets_by_ordinal.len()),
            self_tail_candidates: HashMap::with_capacity(calls.self_tail_candidates.len()),
            checked_self_tail: None,
            expression_types: HashMap::new(),
            named_place_roots: HashMap::new(),
            proof_nodes: CallableProofNodes {
                calls: Some(calls),
                ..Default::default()
            },
            scalar_self_tail_types: !calls.self_tail_candidates.is_empty()
                && super::super::reference::self_tail::scalar_body_types(calls),
            body: PhantomData,
        };
        let mut bind_type = |ordinal, expression| {
            if let Some(ty) = calls.structure.checked_types.expressions.get(&ordinal) {
                nodes
                    .expression_types
                    .insert(std::ptr::from_ref(expression), ty);
            }
        };
        let mut bind_call = |ordinal: OccurrenceOrdinal, expression: &'a ScalarExpression| {
            if matches!(expression, ScalarExpression::Member { .. }) {
                return Ok(());
            }
            let target = targets_by_ordinal.get(&ordinal).copied().ok_or_else(|| {
                let span = calls.occurrences[ordinal.0].span;
                format!(
                    "internal error: missing callable body call target in {source:?} at {}..{} (occurrence {})",
                    span.start, span.end, ordinal.0,
                )
            })?;
            let node = std::ptr::from_ref(expression);
            nodes.targets.insert(node, target);
            if let Some(proof) = checked_proof.filter(|proof| proof.recursive() == ordinal) {
                if matches!(expression, ScalarExpression::Call { .. }) {
                    nodes.checked_self_tail = Some((expression, proof));
                }
            }
            if calls
                .self_tail_candidates
                .binary_search_by_key(&ordinal.0, |candidate| candidate.0)
                .is_ok()
            {
                nodes.self_tail_candidates.insert(node, expression);
            }
            Ok(())
        };
        let mut bind_named_place = |ordinal, place: &'a ScalarPlace| {
            if let Some(root) = calls.prepared_named_place_root(ordinal) {
                nodes
                    .named_place_roots
                    .insert(std::ptr::from_ref(place), root);
            }
        };
        Binder {
            structure: &calls.structure,
            source,
            region: "callable body",
            bind_call: &mut bind_call,
            bind_type: (checked_proof.is_some() || !calls.self_tail_candidates.is_empty())
                .then_some(&mut bind_type as _),
            bind_named_place: Some(&mut bind_named_place),
            proof_nodes: Some(&mut nodes.proof_nodes),
        }
        .block_items(Parent::Block(calls.structure.root), body)?;
        if nodes.self_tail_candidates.len() != calls.self_tail_candidates.len() {
            return Err(format!(
                "internal error: self-tail candidates do not match canonical callable body in {source:?}"
            ));
        }
        Ok(nodes)
    }

    pub(crate) fn has_self_tail_candidates(&self) -> bool {
        !self.self_tail_candidates.is_empty()
    }

    pub(crate) fn has_scalar_self_tail_types(&self) -> bool {
        self.scalar_self_tail_types
    }

    pub(crate) fn has_checked_self_tail(&self) -> bool {
        self.checked_self_tail.is_some()
    }

    pub(crate) fn checked_primitive_outputs(
        &self,
        cfg: &ScalarReferenceCfg,
    ) -> Option<[CfgPointId; 2]> {
        self.checked_self_tail?.1.primitive_outputs(cfg)
    }

    pub(crate) fn is_self_tail_candidate(&self, expression: &ScalarExpression) -> bool {
        self.checked_self_tail
            .is_some_and(|(checked, _)| std::ptr::eq(checked, expression))
            || self
                .self_tail_candidates
                .contains_key(&std::ptr::from_ref(expression))
    }

    pub(crate) fn self_tail_candidates(&self) -> impl Iterator<Item = &'a ScalarExpression> + '_ {
        self.self_tail_candidates
            .values()
            .copied()
            .chain(self.checked_self_tail.map(|(node, _)| node))
    }

    pub(crate) fn expression_type(&self, expression: &ScalarExpression) -> Option<&'a ScalarType> {
        self.expression_types
            .get(&std::ptr::from_ref(expression))
            .copied()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn named_place_root(
        &self,
        place: &ScalarPlace,
    ) -> Option<&'a NamedPlaceRootIdentity> {
        self.named_place_roots
            .get(&std::ptr::from_ref(place))
            .copied()
    }

    pub(crate) fn target(
        &self,
        expression: &ScalarExpression,
    ) -> Result<&'a ResolvedCallTarget, String> {
        self.targets
            .get(&std::ptr::from_ref(expression))
            .copied()
            .ok_or_else(|| {
                let span = expression_span(expression);
                format!(
                    "internal error: missing canonical callable body call node in {:?} at {}..{}",
                    self.source, span.start, span.end,
                )
            })
    }
}

#[allow(dead_code)]
impl<'a> CallableBodyCallNodes<'a> {
    pub(crate) fn literal_field(
        &self,
        value: &ScalarExpression,
    ) -> Option<&'a ScalarStructFieldId> {
        let ordinal = self
            .proof_nodes
            .expressions
            .get(&std::ptr::from_ref(value))?;
        self.proof_nodes
            .calls?
            .structure
            .literal_fields
            .get(ordinal)
    }

    pub(crate) fn plain_name(&self, value: &ScalarExpression) -> Option<&'a PlainNameIdentity> {
        let ordinal = *self
            .proof_nodes
            .expressions
            .get(&std::ptr::from_ref(value))?;
        self.proof_nodes.calls?.prepared_plain_name(ordinal)
    }

    pub(crate) fn lexical_member(
        &self,
        value: &ScalarExpression,
    ) -> Option<(
        &'a crate::scalar::ReferenceBindingId,
        &'a ScalarStructFieldId,
    )> {
        if !matches!(value, ScalarExpression::Member { .. }) {
            return None;
        }
        let ordinal = *self
            .proof_nodes
            .expressions
            .get(&std::ptr::from_ref(value))?;
        self.proof_nodes.calls?.prepared_lexical_member(ordinal)
    }

    pub(crate) fn direct_destination(
        &self,
        place: &ScalarPlace,
    ) -> Option<&'a DirectDestinationIdentity> {
        let ordinal = *self.proof_nodes.places.get(&std::ptr::from_ref(place))?;
        self.proof_nodes.calls?.prepared_direct_destination(ordinal)
    }

    pub(crate) fn checked_expression_type(
        &self,
        value: &ScalarExpression,
    ) -> Option<&'a ScalarType> {
        let ordinal = self
            .proof_nodes
            .expressions
            .get(&std::ptr::from_ref(value))?;
        self.proof_nodes
            .calls?
            .structure
            .checked_types
            .expressions
            .get(ordinal)
    }

    pub(crate) fn checked_place_type(&self, place: &ScalarPlace) -> Option<&'a ScalarType> {
        let ordinal = self.proof_nodes.places.get(&std::ptr::from_ref(place))?;
        self.proof_nodes
            .calls?
            .structure
            .checked_types
            .places
            .get(ordinal)
    }

    pub(crate) fn final_output(&self, value: &ScalarExpression) -> Option<usize> {
        self.proof_nodes
            .final_outputs
            .get(&std::ptr::from_ref(value))
            .copied()
    }

    pub(crate) fn evaluation_point(
        &self,
        facts: &CallablePointFacts,
        value: &ScalarExpression,
    ) -> Option<CfgPointId> {
        let ordinal = self
            .proof_nodes
            .expressions
            .get(&std::ptr::from_ref(value))?;
        facts.evaluations.get(ordinal).copied()
    }

    pub(crate) fn frozen_install(
        &self,
        facts: &CallablePointFacts,
        place: &ScalarPlace,
    ) -> Option<FrozenInstall> {
        let ordinal = self.proof_nodes.places.get(&std::ptr::from_ref(place))?;
        facts.installs.get(ordinal).copied()
    }

    pub(crate) fn binding_definition(
        &self,
        facts: &CallablePointFacts,
        binding: &ScalarBinding,
        receiver: usize,
    ) -> Option<BindingDefinition> {
        let position = self
            .proof_nodes
            .bindings
            .get(&std::ptr::from_ref(binding))?;
        facts
            .definitions
            .get(&(position.block, position.item, receiver))
            .copied()
    }

    pub(crate) fn formal<'facts>(
        &self,
        facts: &'facts CallablePointFacts,
        parameter: usize,
    ) -> Option<(&'facts crate::scalar::ReferenceBindingId, &'a ScalarType)> {
        let slot = self.proof_nodes.calls?.boundary.parameters.get(parameter)?;
        if slot.ordinal != parameter {
            return None;
        }
        Some((facts.formals.get(&parameter)?, &slot.ty))
    }
}

struct Binder<'a, 'bind> {
    structure: &'a OccurrenceStructure,
    source: &'a SourceIdentity,
    region: &'static str,
    bind_call: &'bind mut dyn FnMut(OccurrenceOrdinal, &'a ScalarExpression) -> Result<(), String>,
    bind_type: Option<&'bind mut dyn FnMut(OccurrenceOrdinal, &'a ScalarExpression)>,
    bind_named_place: Option<&'bind mut dyn FnMut(OccurrenceOrdinal, &'a ScalarPlace)>,
    proof_nodes: Option<&'bind mut CallableProofNodes<'a>>,
}

impl<'a> Binder<'a, '_> {
    fn initializer_items(
        &mut self,
        region: &ModuleInitializationRegion,
        items: &'a [ScalarItem],
    ) -> Result<(), String> {
        let root = Parent::Block(self.structure.root);
        for step in &region.steps {
            match (&step.kind, &items[step.item]) {
                (
                    ModuleInitializationStepKind::NamespaceDispatch { .. },
                    ScalarItem::Namespace(_),
                ) => {}
                (
                    ModuleInitializationStepKind::BindingInitialization,
                    ScalarItem::Binding(binding),
                ) => self.binding(root, step.item, binding)?,
                (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                    self.item(root, step.item, item, ScalarBlockItemRole::Ordinary)?
                }
                _ => unreachable!("initializer dispatch derives from the exact item slice"),
            }
        }
        Ok(())
    }

    fn invalid_edge(&self, parent: Parent, edge: Edge) -> String {
        format!(
            "internal error: invalid {} structural edge {parent:?}/{edge:?} in {:?}",
            self.region, self.source,
        )
    }

    fn expression(
        &mut self,
        parent: Parent,
        edge: Edge,
        expression: &'a ScalarExpression,
    ) -> Result<(), String> {
        let Some(&Node::Expression(ordinal)) = self.structure.edges.get(&(parent, edge)) else {
            return Err(self.invalid_edge(parent, edge));
        };
        if let Some(nodes) = &mut self.proof_nodes {
            nodes
                .expressions
                .insert(std::ptr::from_ref(expression), ordinal);
            if parent == Parent::Block(self.structure.root) {
                if let Edge::FinalOutput { output, .. } = edge {
                    nodes
                        .final_outputs
                        .insert(std::ptr::from_ref(expression), output);
                }
            }
        }
        if let Some(bind_type) = &mut self.bind_type {
            bind_type(ordinal, expression);
        }
        let parent = Parent::Expression(ordinal);
        match expression {
            ScalarExpression::Call { arguments, .. } => {
                (self.bind_call)(ordinal, expression)?;
                for (index, argument) in arguments.iter().enumerate() {
                    self.expression(parent, Edge::CallArgument(index), argument)?;
                }
            }
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(parent, Edge::BinaryLeft, left)?;
                self.expression(parent, Edge::BinaryRight, right)?;
            }
            ScalarExpression::Unary { operand, .. } => {
                self.expression(parent, Edge::UnaryOperand, operand)?;
            }
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(parent, Edge::Condition, condition)?;
                self.block(parent, Edge::ThenBranch, then_branch)?;
                self.block(parent, Edge::ElseBranch, else_branch)?;
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(parent, Edge::Condition, condition)?;
                self.block(parent, Edge::ThenBranch, then_branch)?;
            }
            ScalarExpression::Block(block) => self.block(parent, Edge::BlockBody, block)?,
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. }
            | ScalarExpression::IndexedRead { place, .. } => {
                self.place(parent, Edge::ExpressionPlace, place)?;
            }
            ScalarExpression::StructLiteral { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    self.expression(parent, Edge::AggregateField(index), &field.value)?;
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for (index, element) in elements.iter().enumerate() {
                    self.expression(parent, Edge::AggregateElement(index), element)?;
                }
            }
            ScalarExpression::Member { .. } => (self.bind_call)(ordinal, expression)?,
            ScalarExpression::Name { .. }
            | ScalarExpression::Integer { .. }
            | ScalarExpression::InvalidInteger { .. }
            | ScalarExpression::Float { .. }
            | ScalarExpression::InvalidFloat { .. }
            | ScalarExpression::Boolean { .. }
            | ScalarExpression::Char { .. }
            | ScalarExpression::Utf8 { .. } => {}
        }
        Ok(())
    }

    fn place(&mut self, parent: Parent, edge: Edge, place: &'a ScalarPlace) -> Result<(), String> {
        let Some(&Node::Place(ordinal)) = self.structure.edges.get(&(parent, edge)) else {
            return Err(self.invalid_edge(parent, edge));
        };
        if let Some(nodes) = &mut self.proof_nodes {
            nodes.places.insert(std::ptr::from_ref(place), ordinal);
        }
        let parent = Parent::Place(ordinal);
        match place {
            ScalarPlace::Name { .. } => {
                if let Some(bind_named_place) = &mut self.bind_named_place {
                    bind_named_place(ordinal, place);
                }
                Ok(())
            }
            ScalarPlace::Field { base, .. } => self.place(parent, Edge::PlaceBase, base),
            ScalarPlace::Index { base, index, .. } => {
                self.place(parent, Edge::PlaceBase, base)?;
                self.expression(parent, Edge::PlaceIndex, index)
            }
            ScalarPlace::Dereference { pointer, .. } => {
                self.expression(parent, Edge::DereferencePointer, pointer)
            }
        }
    }

    fn block(&mut self, parent: Parent, edge: Edge, block: &'a ScalarBlock) -> Result<(), String> {
        let Some(&Node::Block(ordinal)) = self.structure.edges.get(&(parent, edge)) else {
            return Err(self.invalid_edge(parent, edge));
        };
        self.block_items(Parent::Block(ordinal), block)
    }

    fn block_items(&mut self, parent: Parent, block: &'a ScalarBlock) -> Result<(), String> {
        if let (Some(nodes), Parent::Block(ordinal)) = (&mut self.proof_nodes, parent) {
            nodes.blocks.insert(std::ptr::from_ref(block), ordinal);
        }
        for (index, (item, role)) in block.items_with_output_roles().enumerate() {
            self.item(parent, index, item, role)?;
        }
        Ok(())
    }

    fn binding(
        &mut self,
        parent: Parent,
        item: usize,
        binding: &'a ScalarBinding,
    ) -> Result<(), String> {
        if let (Some(nodes), Parent::Block(block)) = (&mut self.proof_nodes, parent) {
            if nodes.calls.is_some() || nodes.initializer_bindings {
                nodes
                    .bindings
                    .insert(std::ptr::from_ref(binding), BlockPosition { block, item });
            }
        }
        if binding.is_allocation {
            for (position, receiver) in binding.receivers.iter().enumerate() {
                if let Some(length) = &receiver.allocation_length {
                    self.expression(
                        parent,
                        Edge::BindingAllocationLength { item, position },
                        length,
                    )?;
                }
            }
        } else {
            let outputs = match binding.output_origin {
                ScalarBindingOutputOrigin::SingleExpression => 1,
                ScalarBindingOutputOrigin::IndependentExpressions => {
                    binding.output_values.len().max(1)
                }
            };
            for output in 0..outputs {
                let ScalarBindingOutput::Evaluate(value) = binding.output_source(output) else {
                    unreachable!("initializer binding outputs share the primary expression");
                };
                self.expression(parent, Edge::BindingRoot { item, output }, value)?;
            }
        }
        Ok(())
    }

    fn item(
        &mut self,
        parent: Parent,
        item: usize,
        statement: &'a ScalarBlockItem,
        role: ScalarBlockItemRole<'_>,
    ) -> Result<(), String> {
        match statement {
            ScalarBlockItem::LocalBinding(binding) => self.binding(parent, item, binding),
            ScalarBlockItem::Expression(expression) => {
                let edge = match role {
                    ScalarBlockItemRole::Ordinary => Edge::ExpressionItem { item },
                    ScalarBlockItemRole::FinalOutput { output, .. } => {
                        Edge::FinalOutput { item, output }
                    }
                };
                self.expression(parent, edge, expression)
            }
            ScalarBlockItem::Assignment(assignment) => {
                for (output, value) in assignment.rhs().evaluations() {
                    self.expression(parent, Edge::AssignmentRhs { item, output }, value)?;
                }
                for (destination, target) in assignment.targets.iter().enumerate() {
                    self.place(
                        parent,
                        Edge::AssignmentDestination { item, destination },
                        &target.place,
                    )?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
