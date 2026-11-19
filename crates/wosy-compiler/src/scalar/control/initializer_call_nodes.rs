use std::marker::PhantomData;

use super::super::reference::{
    FunctionCallSites, OccurrenceEdge as Edge, OccurrenceNode as Node, OccurrenceParent as Parent,
};
use super::super::typecheck::expression_span;
use super::super::{
    ResolvedCallTarget, ScalarBindingOutput, ScalarBindingOutputOrigin, ScalarBlock,
    ScalarBlockItem, ScalarBlockItemRole, ScalarExpression, ScalarPlace,
};
use super::*;

pub(crate) struct InitializerCallNodes<'a> {
    source: &'a SourceIdentity,
    targets: HashMap<*const ScalarExpression, &'a ModuleInitializationCallTarget>,
    items: PhantomData<&'a [ScalarItem]>,
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
            items: PhantomData,
        };
        let mut bind_call = |ordinal, expression: &'a ScalarExpression| {
            let target = topology
                .call_targets
                .get(&ordinal)
                .ok_or_else(|| topology.missing_call_target(&region.source, ordinal))?;
            nodes.targets.insert(std::ptr::from_ref(expression), target);
            Ok(())
        };
        let mut binder = Binder {
            structure: &topology.structure,
            source: &region.source,
            region: "initializer",
            bind_call: &mut bind_call,
        };
        let root = Parent::Block(topology.structure.root);
        for step in &region.steps {
            match (&step.kind, &items[step.item]) {
                (
                    ModuleInitializationStepKind::NamespaceDispatch { .. },
                    ScalarItem::Namespace(_),
                ) => {}
                (
                    ModuleInitializationStepKind::BindingInitialization,
                    ScalarItem::Binding(binding),
                ) => {
                    binder.binding(root, step.item, binding)?;
                }
                (ModuleInitializationStepKind::ExecutableItem, ScalarItem::Executable(item)) => {
                    binder.item(root, step.item, item, ScalarBlockItemRole::Ordinary)?;
                }
                _ => unreachable!("initializer dispatch derives from the exact item slice"),
            }
        }
        if nodes.targets.len() != topology.calls.len() {
            return Err(format!(
                "internal error: initializer call topology does not match canonical items in {:?}",
                region.source,
            ));
        }
        Ok(nodes)
    }

    pub(crate) fn target(
        &self,
        expression: &ScalarExpression,
    ) -> Result<&'a ModuleInitializationCallTarget, String> {
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
}

pub(crate) struct CallableBodyCallNodes<'a> {
    source: &'a SourceIdentity,
    targets: HashMap<*const ScalarExpression, &'a ResolvedCallTarget>,
    body: PhantomData<&'a ScalarBlock>,
}

impl<'a> CallableBodyCallNodes<'a> {
    pub(in crate::scalar) fn bind(
        source: &'a SourceIdentity,
        body: &'a ScalarBlock,
        calls: &'a FunctionCallSites,
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
            body: PhantomData,
        };
        let mut bind_call = |ordinal: OccurrenceOrdinal, expression: &'a ScalarExpression| {
            let target = targets_by_ordinal.get(&ordinal).copied().ok_or_else(|| {
                let span = calls.occurrences[ordinal.0].span;
                format!(
                    "internal error: missing callable body call target in {source:?} at {}..{} (occurrence {})",
                    span.start, span.end, ordinal.0,
                )
            })?;
            nodes.targets.insert(std::ptr::from_ref(expression), target);
            Ok(())
        };
        Binder {
            structure: &calls.structure,
            source,
            region: "callable body",
            bind_call: &mut bind_call,
        }
        .block_items(Parent::Block(calls.structure.root), body)?;
        Ok(nodes)
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

struct Binder<'a, 'bind> {
    structure: &'a OccurrenceStructure,
    source: &'a SourceIdentity,
    region: &'static str,
    bind_call: &'bind mut dyn FnMut(OccurrenceOrdinal, &'a ScalarExpression) -> Result<(), String>,
}

impl<'a> Binder<'a, '_> {
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
            ScalarExpression::Name { .. }
            | ScalarExpression::Member { .. }
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
        let parent = Parent::Place(ordinal);
        match place {
            ScalarPlace::Name { .. } => Ok(()),
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
            ScalarBlockItem::While(statement) => {
                self.expression(parent, Edge::WhileCondition { item }, &statement.condition)?;
                self.block(parent, Edge::WhileBody { item }, &statement.body)
            }
        }
    }
}

#[cfg(test)]
mod tests;
