//! Typed, ordered prefix certificates. They are not Check results.
use super::*;
use crate::scalar::reference::{BlockOrdinal, NamedPlaceRootIdentity, OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};
use wosy_syntax::SourceSpan;

type EffectId = usize;

#[derive(Clone, Debug)]
pub(super) enum Effect {
    Sequence { at: OccurrenceOrdinal, operands: Vec<EffectId> },
    LocalRead { at: OccurrenceOrdinal, binding: ReferenceBindingId, version: Option<EffectId> },
    Read { at: OccurrenceOrdinal, declaration: SourceSpan, index: Option<u64> },
    Field { at: OccurrenceOrdinal, field: super::super::ScalarStructFieldId, receiver: EffectId },
    Branch { at: OccurrenceOrdinal, condition: EffectId, yes: EffectId, no: EffectId },
    ShortCircuit { at: OccurrenceOrdinal, conjunction: bool, left: EffectId, right: EffectId },
    Call { at: OccurrenceOrdinal, function: Option<PreparedCallable>, arguments: Vec<EffectId> },
}

#[derive(Clone, Debug)]
pub(super) struct BindingFact {
    pub(super) slot: ReferenceBindingId,
    pub(super) ty: ScalarType,
    pub(super) returning_effect: EffectId,
}

#[derive(Clone, Debug)]
pub(super) struct EffectRecord {
    pub(super) owner: PreparedCallable,
    pub(super) operation: Effect,
}

#[derive(Clone)]
struct LocalFact {
    ty: ScalarType,
    // None denotes an arbitrary formal, not a known value.
    version: Option<EffectId>,
}

pub(super) struct GuardedWrite {
    pub(super) binding: ReferenceBindingId,
    pub(super) destination: OccurrenceOrdinal,
    pub(super) prior: EffectId,
    pub(super) assigned: EffectId,
}

pub(super) struct GuardedJoin {
    pub(super) binding: ReferenceBindingId,
    pub(super) taken: EffectId,
    pub(super) bypass: EffectId,
}

pub(super) struct GuardedTransfer {
    pub(super) condition: EffectId,
    pub(super) writes: Vec<GuardedWrite>,
    pub(super) joins: Vec<GuardedJoin>,
}

#[derive(Default)]
pub(super) struct PrefixCertificate {
    pub(super) effects: Vec<EffectRecord>,
    pub(super) bindings: Vec<BindingFact>,
    pub(super) callees: HashMap<PreparedCallable, FunctionEffects>,
    pub(super) guarded: Option<GuardedTransfer>,
}

pub(super) struct FunctionEffects {
    pub(super) bindings: Vec<BindingFact>,
    pub(super) returned: EffectId,
}

struct Frame<'a> {
    target: PreparedCallable,
    module: &'a ScalarModule,
    function: &'a ScalarFunction,
    prepared: &'a FunctionCallSites,
    init: &'a ModuleInitializationTopology,
}

struct Planner<'a> {
    project: &'a ScalarProject,
    calls: &'a ProjectCallSiteResults,
    owner: usize,
    certificate: PrefixCertificate,
}

pub(super) fn prepare_readonly_prefix(
    prepared: &PreparedCheckerProject, initialization: &CheckedInitialization<'_>, target: PreparedCallable,
) -> Option<PrefixCertificate> {
    prepare_prefix::<false>(prepared, initialization, target)
}

pub(super) fn prepare_guarded_prefix(
    prepared: &PreparedCheckerProject, initialization: &CheckedInitialization<'_>, target: PreparedCallable,
) -> Option<PrefixCertificate> {
    prepare_prefix::<true>(prepared, initialization, target)
}

fn prepare_prefix<const GUARDED: bool>(
    prepared: &PreparedCheckerProject, initialization: &CheckedInitialization<'_>, target: PreparedCallable,
) -> Option<PrefixCertificate> {
    let (project, calls) = prepared.parts();
    let module = project.modules.get(target.0)?;
    if initialization.visits.get(target.0) != Some(&InitializationVisit::Completed)
        || !std::ptr::eq(*initialization.ordinals.get_key_value(&module.source)?.0, &module.source)
        || initialization.ordinals.get(&module.source) != Some(&target.0)
    { return None; }
    let mut planner = Planner { project, calls, owner: target.0, certificate: PrefixCertificate::default() };
    let frame = planner.frame(target)?;
    let mut locals = frame.formals()?;
    let checked = frame.prepared.structure.blocks.get(frame.prepared.structure.root.0)?;
    for (item, prepared) in frame.function.body.items.iter().zip(&checked.items) {
        if GUARDED && matches!(item, ScalarBlockItem::Expression(ScalarExpression::UnitIf { .. })) {
            let rewind = planner.certificate.effects.len();
            let mut dependencies = Vec::new();
            let mut installed = Vec::new();
            let guarded = planner.guarded_transfer(&frame, item, prepared, &locals, &mut dependencies)
                .and_then(|guarded| planner.seal_dependencies(dependencies, &mut installed).map(|()| guarded));
            if guarded.is_none() {
                planner.certificate.effects.truncate(rewind);
                for target in installed { planner.certificate.callees.remove(&target); }
            }
            planner.certificate.guarded = guarded;
            break;
        }
        let rewind = planner.certificate.effects.len();
        let mut dependencies = Vec::new();
        let mut installed = Vec::new();
        let next = planner.binding(&frame, item, prepared, &locals, &mut dependencies)
            .and_then(|binding| planner.seal_dependencies(dependencies, &mut installed).map(|()| binding));
        let Some(binding) = next else {
            planner.certificate.effects.truncate(rewind);
            for target in installed { planner.certificate.callees.remove(&target); }
            break;
        };
        if locals.insert(binding.slot.clone(), LocalFact { ty: binding.ty.clone(), version: Some(binding.returning_effect) }).is_some() { return None; }
        planner.certificate.bindings.push(binding);
    }
    (!planner.certificate.bindings.is_empty()).then_some(planner.certificate)
}

impl<'a> Planner<'a> {
    fn frame(&self, target: PreparedCallable) -> Option<Frame<'a>> {
        if target.0 != self.owner { return None; }
        let module = self.project.modules.get(target.0)?;
        let ScalarItem::Function(function) = module.items.get(target.1)? else { return None; };
        let prepared = self.calls.function_body(target.0, target.1, None)?;
        let region = self.calls.initialization_region(target.0)?;
        let ModuleInitializationDetail::Occurrences(init) = &region.detail else { return None; };
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty()
            || function.body.unsafe_context || prepared.contextual_formal().is_some()
        { return None; }
        Some(Frame { target, module, function, prepared, init })
    }

    fn seal_dependencies(&mut self, dependencies: Vec<PreparedCallable>, installed: &mut Vec<PreparedCallable>) -> Option<()> {
        enum Work {
            Enter(PreparedCallable),
            Finish(PreparedCallable, FunctionEffects),
        }
        let mut pending = dependencies.into_iter().rev().map(Work::Enter).collect::<Vec<_>>();
        let mut active = HashSet::new();
        while let Some(task) = pending.pop() {
            match task {
                Work::Enter(target) => {
                    if self.certificate.callees.contains_key(&target) { continue; }
                    if !active.insert(target) { return None; }
                    let (summary, dependencies) = self.complete_callee(target)?;
                    pending.push(Work::Finish(target, summary));
                    pending.extend(dependencies.into_iter().rev().map(Work::Enter));
                }
                Work::Finish(target, summary) => {
                    active.remove(&target);
                    self.certificate.callees.insert(target, summary);
                    installed.push(target);
                }
            }
        }
        Some(())
    }

    fn complete_callee(&mut self, target: PreparedCallable) -> Option<(FunctionEffects, Vec<PreparedCallable>)> {
        let frame = self.frame(target)?;
        let mut locals = frame.formals()?;
        let structure = &frame.prepared.structure;
        let checked = &structure.blocks.get(structure.root.0)?.items;
        let [result] = frame.prepared.boundary.results.as_slice() else { return None; };
        if !value_type(&result.ty) || checked.len() != frame.function.body.items.len() { return None; }
        let (last, preceding) = checked.split_last()?;
        let mut bindings = Vec::new();
        let mut deps = Vec::new();
        for (ast, prepared) in frame.function.body.items.iter().zip(preceding) {
            let fact = self.binding(&frame, ast, prepared, &locals, &mut deps)?;
            if locals.insert(fact.slot.clone(), LocalFact { ty: fact.ty.clone(), version: Some(fact.returning_effect) }).is_some() { return None; }
            bindings.push(fact);
        }
        let (tail, at) = frame.returned(&frame.function.body, structure.root, last)?;
        if structure.checked_types.expressions.get(&at)? != &result.ty { return None; }
        let returned = self.expression(&frame, tail, at, &locals, &mut deps)?;
        Some((FunctionEffects { bindings, returned }, deps))
    }

    fn binding(
        &mut self, frame: &Frame<'a>, item: &'a ScalarBlockItem, checked: &BlockItem,
        locals: &HashMap<ReferenceBindingId, LocalFact>, deps: &mut Vec<PreparedCallable>,
    ) -> Option<BindingFact> {
        let ScalarBlockItem::LocalBinding(binding) = item else { return None; };
        let BlockItemKind::Binding(prepared) = &checked.kind else { return None; };
        let BindingInitialization::Outputs(outputs) = &prepared.initialization else { return None; };
        let [slot] = prepared.receivers() else { return None; };
        let structure = &frame.prepared.structure;
        if binding.is_allocation || !outputs.secondary.is_empty() || !value_type(&slot.ty)
            || slot.contextual_storage.is_some() || structure.checked_types.expressions.get(&outputs.primary)? != &slot.ty
        { return None; }
        let returning_effect = self.expression(frame, &binding.value, outputs.primary, locals, deps)?;
        Some(BindingFact {
            slot: ReferenceBindingId { source: frame.module.source.clone(), function_span: frame.function.span,
                declaration_span: slot.span, block_span: frame.function.body.span, kind: super::super::ReferenceBindingKind::Declared },
            ty: slot.ty.clone(), returning_effect,
        })
    }

    fn guarded_transfer(
        &mut self, frame: &Frame<'a>, item: &'a ScalarBlockItem, checked: &BlockItem,
        locals: &HashMap<ReferenceBindingId, LocalFact>, deps: &mut Vec<PreparedCallable>,
    ) -> Option<GuardedTransfer> {
        let ScalarBlockItem::Expression(expr @ ScalarExpression::UnitIf { condition, then_branch, .. }) = item else { return None; };
        let BlockItemKind::Evaluation { expression } = checked.kind else { return None; };
        let ConditionalRegion::Unit { condition: test, then_block, fallthrough, .. } =
            frame.prepared.conditionals.get(&expression)? else { return None; };
        let structure = &frame.prepared.structure;
        let parent = OccurrenceParent::Expression(expression);
        if *fallthrough != expression || then_branch.unsafe_context
            || frame.prepared.occurrences.get(expression.0)?.span != super::super::typecheck::expression_span(expr)
            || structure.checked_types.expressions.get(&expression)? != &ScalarType::Unit
            || structure.checker_contextual_occurrences.contains(&expression)
            || structure.checked_types.expressions.get(test)? != &ScalarType::Bool
            || structure.edges.get(&(parent, OccurrenceEdge::Condition)) != Some(&OccurrenceNode::Expression(*test))
            || structure.edges.get(&(parent, OccurrenceEdge::ThenBranch)) != Some(&OccurrenceNode::Block(*then_block))
        { return None; }
        let block = structure.blocks.get(then_block.0)?;
        if block.items.is_empty() || block.items.len() != then_branch.items.len() { return None; }
        let condition = self.expression(frame, condition, *test, locals, deps)?;
        let mut branch = locals.clone();
        let mut writes = Vec::with_capacity(block.items.len());
        let mut joins = Vec::<GuardedJoin>::new();
        let mut by_binding = HashMap::new();
        for (ast, checked) in then_branch.items.iter().zip(&block.items) {
            let ScalarBlockItem::Assignment(assignment) = ast else { return None; };
            let BlockItemKind::Assignment { group } = checked.kind else { return None; };
            let [rhs] = assignment.values.as_slice() else { return None; };
            if group.block != *then_block || group.item != checked.position { return None; }
            let (binding, replacement, rhs_at, destination) = super::super::checker_flow::checked_scalar_assignment(frame.prepared, group)?;
            let prior = branch.get(binding)?;
            // Do not invent a generation for parameters, other types or aliased destinations.
            if prior.ty != ScalarType::F32 || *replacement != prior.ty
                || structure.checked_types.expressions.get(&rhs_at)? != replacement
                || structure.checker_contextual_occurrences.contains(&destination)
            { return None; }
            let prior = prior.version?;
            let assigned = self.expression(frame, rhs, rhs_at, &branch, deps)?;
            branch.get_mut(binding)?.version = Some(assigned);
            writes.push(GuardedWrite { binding: binding.clone(), destination, prior, assigned });
            if let Some(&index) = by_binding.get(binding) {
                let join: &mut GuardedJoin = joins.get_mut(index)?;
                join.taken = assigned;
            } else {
                by_binding.insert(binding.clone(), joins.len());
                joins.push(GuardedJoin { binding: binding.clone(), taken: assigned, bypass: prior });
            }
        }
        Some(GuardedTransfer { condition, writes, joins })
    }

    fn expression(
        &mut self, frame: &Frame<'a>, expression: &'a ScalarExpression, initial: OccurrenceOrdinal,
        locals: &HashMap<ReferenceBindingId, LocalFact>, deps: &mut Vec<PreparedCallable>,
    ) -> Option<EffectId> {
        enum Work<'a> {
            Visit(OccurrenceOrdinal, &'a ScalarExpression),
            Sequence(OccurrenceOrdinal, usize),
            Field(OccurrenceOrdinal, super::super::ScalarStructFieldId),
            Branch(OccurrenceOrdinal),
            Short(OccurrenceOrdinal, bool),
            Call(OccurrenceOrdinal, Option<PreparedCallable>, usize),
        }

        let mut pending = vec![Work::Visit(initial, expression)];
        let mut operands = Vec::new();
        let structure = &frame.prepared.structure;
        let child = |at, edge| match structure.edges.get(&(OccurrenceParent::Expression(at), edge))? {
            OccurrenceNode::Expression(child) => Some(*child),
            _ => None,
        };
        while let Some(task) = pending.pop() {
            let effect = match task {
                Work::Sequence(at, arity) => Effect::Sequence { at, operands: take(&mut operands, arity)? },
                Work::Field(at, field) => Effect::Field { at, field, receiver: operands.pop()? },
                Work::Branch(at) => {
                    let no = operands.pop()?;
                    let yes = operands.pop()?;
                    let condition = operands.pop()?;
                    Effect::Branch { at, condition, yes, no }
                }
                Work::Short(at, conjunction) => {
                    let right = operands.pop()?;
                    let left = operands.pop()?;
                    Effect::ShortCircuit { at, conjunction, left, right }
                }
                Work::Call(at, function, arity) => Effect::Call { at, function, arguments: take(&mut operands, arity)? },
                Work::Visit(at, expr) => {
                    let ty = structure.checked_types.expressions.get(&at)?;
                    if frame.prepared.occurrences.get(at.0)?.span != super::super::typecheck::expression_span(expr)
                        || structure.checker_contextual_occurrences.contains(&at)
                    { return None; }
                    match expr {
                        ScalarExpression::Name { .. } if value_type(ty) => match frame.prepared.prepared_plain_name(at)? {
                            PlainNameIdentity::Lexical(binding) => {
                                let current = locals.get(binding)?;
                                if &current.ty != ty { return None; }
                                Effect::LocalRead { at, binding: binding.clone(), version: current.version }
                            },
                            PlainNameIdentity::ModuleValue(declaration) => {
                                frame.module_value(declaration, ty)?;
                                Effect::Read { at, declaration: declaration.clone(), index: None }
                            }
                            _ => return None,
                        },
                        ScalarExpression::Member { .. } if value_type(ty) => {
                            let (declaration, field) = frame.prepared.prepared_module_direct_member(at)?;
                            frame.field(&field.structure, field, ty)?;
                            frame.module_value(declaration, &ScalarType::Struct(field.structure.clone()))?;
                            let receiver = self.certificate.effects.len();
                            self.certificate.effects.push(EffectRecord { owner: frame.target,
                                operation: Effect::Read { at, declaration: declaration.clone(), index: None },
                            });
                            Effect::Field { at, field: field.clone(), receiver }
                        }
                        ScalarExpression::IndexedRead { place, .. } => {
                            let OccurrenceNode::Place(place_at) = structure.edges.get(
                                &(OccurrenceParent::Expression(at), OccurrenceEdge::ExpressionPlace),
                            )? else { return None; };
                            if let super::super::ScalarPlace::Field { base, field, .. } = place {
                                let super::super::ScalarFieldReference::Resolved(field) = field else { return None; };
                                let OccurrenceNode::Place(base_at) = structure.edges.get(
                                    &(OccurrenceParent::Place(*place_at), OccurrenceEdge::PlaceBase),
                                )? else { return None; };
                                let ScalarType::Struct(id) = structure.checked_types.places.get(base_at)? else { return None; };
                                frame.field(id, field, ty)?;
                                if structure.checked_types.places.get(place_at)? != ty { return None; }
                                let read = frame.static_index(at, *base_at, base, &ScalarType::Struct(id.clone()))?;
                                let receiver = self.certificate.effects.len();
                                self.certificate.effects.push(EffectRecord { owner: frame.target, operation: read });
                                Effect::Field { at, field: field.clone(), receiver }
                            } else {
                                frame.static_index(at, *place_at, place, ty)?
                            }
                        },
                        ScalarExpression::ValueField { receiver, .. } if value_type(ty) => {
                            let parent = child(at, OccurrenceEdge::ValueFieldReceiver)?;
                            let ScalarType::Struct(id) = structure.checked_types.expressions.get(&parent)? else { return None; };
                            let field = structure.value_fields.get(&at)?;
                            frame.field(id, field, ty)?;
                            pending.push(Work::Field(at, field.clone()));
                            pending.push(Work::Visit(parent, receiver));
                            continue;
                        }
                        ScalarExpression::Binary { operator, left, right, .. } => {
                            let lhs = child(at, OccurrenceEdge::BinaryLeft)?;
                            let rhs = child(at, OccurrenceEdge::BinaryRight)?;
                            let kind = structure.checked_types.expressions.get(&lhs)?;
                            if structure.checked_types.expressions.get(&rhs)? != kind { return None; }
                            use BinaryOperator::*;
                            let op = match operator {
                                And | Or if *ty == ScalarType::Bool && *kind == ScalarType::Bool => {
                                    let cut = frame.prepared.short_circuits.get(&at)?;
                                    let conjunction = *operator == And;
                                    if cut.left != lhs || cut.right != rhs || cut.join != at
                                        || cut.evaluate_right_when != conjunction || cut.bypass == conjunction
                                    { return None; }
                                    Work::Short(at, conjunction)
                                }
                                Equal | NotEqual | Less | LessEqual | Greater | GreaterEqual
                                    if *ty == ScalarType::Bool && value_type(kind) => Work::Sequence(at, 2),
                                Add | Subtract | Multiply | Divide if float_type(ty) && kind == ty => Work::Sequence(at, 2),
                                _ => return None,
                            };
                            pending.push(op);
                            pending.push(Work::Visit(rhs, right));
                            pending.push(Work::Visit(lhs, left));
                            continue;
                        }
                        ScalarExpression::Unary { operator, operand, .. }
                            if (*operator == UnaryOperator::LogicalNot && *ty == ScalarType::Bool)
                                || (*operator == UnaryOperator::Negate && float_type(ty)) =>
                        {
                            let operand_at = child(at, OccurrenceEdge::UnaryOperand)?;
                            if structure.checked_types.expressions.get(&operand_at)? != ty { return None; }
                            pending.push(Work::Sequence(at, 1));
                            pending.push(Work::Visit(operand_at, operand));
                            continue;
                        }
                        ScalarExpression::If { condition, then_branch, else_branch, .. } if value_type(ty) => {
                            let ConditionalRegion::Value { condition: test, then_block, else_block, resume, .. } =
                                frame.prepared.conditionals.get(&at)? else { return None; };
                            if *resume != at || child(at, OccurrenceEdge::Condition)? != *test
                                || structure.checked_types.expressions.get(test)? != &ScalarType::Bool
                                || structure.edges.get(&(OccurrenceParent::Expression(at), OccurrenceEdge::ThenBranch))
                                    != Some(&OccurrenceNode::Block(*then_block))
                                || structure.edges.get(&(OccurrenceParent::Expression(at), OccurrenceEdge::ElseBranch))
                                    != Some(&OccurrenceNode::Block(*else_block))
                            { return None; }
                            let [then_item] = structure.blocks.get(then_block.0)?.items.as_slice() else { return None; };
                            let [else_item] = structure.blocks.get(else_block.0)?.items.as_slice() else { return None; };
                            let (yes, yes_at) = frame.returned(then_branch, *then_block, then_item)?;
                            let (no, no_at) = frame.returned(else_branch, *else_block, else_item)?;
                            if structure.checked_types.expressions.get(&yes_at)? != ty || structure.checked_types.expressions.get(&no_at)? != ty {
                                return None;
                            }
                            pending.push(Work::Branch(at));
                            pending.push(Work::Visit(no_at, no));
                            pending.push(Work::Visit(yes_at, yes));
                            pending.push(Work::Visit(*test, condition));
                            continue;
                        }
                        ScalarExpression::Call { arguments, .. } if value_type(ty) => {
                            let signature = frame.prepared.checked_call_signatures.get(&at)?;
                            if arguments.len() != signature.parameters.len()
                                || !matches!(signature.outputs.outputs.as_slice(), [result] if &result.ty == ty)
                            { return None; }
                            let target = match &frame.prepared.resolved_call_site(at)?.target {
                                ResolvedCallTarget::Core(super::super::CoreOperationId::SintToFloat)
                                    if float_type(ty) && matches!(signature.parameters.as_slice(), [arg] if is_signed_integer(arg)) => None,
                                ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration } => {
                                    let target = *structure.checker_direct_targets.get(&at)?;
                                    let callee = self.frame(target)?;
                                    if target == frame.target || source != &callee.module.source || *declaration_span != callee.function.name_span
                                        || !callee.prepared.boundary.parameters.iter().map(|slot| &slot.ty).eq(&signature.parameters)
                                        || !matches!(callee.prepared.boundary.results.as_slice(), [result] if &result.ty == ty)
                                    { return None; }
                                    deps.push(target);
                                    Some(target)
                                }
                                _ => return None,
                            };
                            pending.push(Work::Call(at, target, arguments.len()));
                            for (index, argument) in arguments.iter().enumerate().rev() {
                                let argument_at = child(at, OccurrenceEdge::CallArgument(index))?;
                                if structure.checked_types.expressions.get(&argument_at)? != signature.parameters.get(index)? {
                                    return None;
                                }
                                pending.push(Work::Visit(argument_at, argument));
                            }
                            continue;
                        }
                        ScalarExpression::Boolean { .. } if *ty == ScalarType::Bool => Effect::Sequence { at, operands: Vec::new() },
                        ScalarExpression::Float { .. } if float_type(ty) => Effect::Sequence { at, operands: Vec::new() },
                        _ if is_integer_type(ty) && integer_literal_fits_type(concrete_integer_literal(expr)?.as_ref(), ty) =>
                            Effect::Sequence { at, operands: Vec::new() },
                        _ => return None,
                    }
                }
            };
            operands.push(self.certificate.effects.len());
            self.certificate.effects.push(EffectRecord { owner: frame.target, operation: effect });
        }
        let [output] = operands.as_slice() else { return None; };
        Some(*output)
    }
}

impl Frame<'_> {
    fn formals(&self) -> Option<HashMap<ReferenceBindingId, LocalFact>> {
        let bindings = self.prepared.boundary.checker_parameter_bindings.as_deref()?;
        if bindings.len() != self.prepared.boundary.parameters.len() { return None; }
        bindings.iter().zip(&self.prepared.boundary.parameters).map(|(binding, slot)| {
            value_type(&slot.ty).then(|| (binding.clone(), LocalFact { ty: slot.ty.clone(), version: None }))
        }).collect()
    }

    fn returned<'a>(&self, ast: &'a ScalarBlock, block: BlockOrdinal, item: &BlockItem) -> Option<(&'a ScalarExpression, OccurrenceOrdinal)> {
        let BlockItemKind::FinalOutput(slot) = item.kind else { return None; };
        let [tail] = ast.final_output_values.as_slice() else { return None; };
        let structure = &self.prepared.structure;
        if ast.unsafe_context || slot.output != 0
            || structure.edges.get(&(OccurrenceParent::Block(block), OccurrenceEdge::FinalOutput { item: slot.item, output: 0 }))
                != Some(&OccurrenceNode::Expression(slot.expression))
        { return None; }
        Some((&tail.value, slot.expression))
    }

    fn module_value(&self, at: &SourceSpan, ty: &ScalarType) -> Option<()> {
        (&at.source == &self.module.source && self.init.structure.blocks.get(self.init.structure.root.0)?.items.iter()
            .any(|item| matches!(&item.kind, BlockItemKind::Binding(binding) if binding.receivers().iter().any(|slot|
                slot.span == at.range && slot.contextual_storage.is_none() && super::super::scalar_type_equal(&slot.ty, ty)))))
            .then_some(())
    }

    fn field(&self, id: &super::super::ScalarStructId, field: &super::super::ScalarStructFieldId, ty: &ScalarType) -> Option<()> {
        let declaration = self.module.structs.get(id.index)?;
        let member = declaration.fields.get(field.index)?;
        (id.source == self.module.source && declaration.id == *id && field.structure == *id
            && member.id == *field && &member.ty == ty && value_type(ty)).then_some(())
    }

    fn static_index(&self, at: OccurrenceOrdinal, place: OccurrenceOrdinal, source: &super::super::ScalarPlace, ty: &ScalarType) -> Option<Effect> {
        use super::super::ScalarPlace;
        let ScalarPlace::Index { base, index, .. } = source else { return None; };
        if !matches!(base.as_ref(), ScalarPlace::Name { .. }) { return None; }
        let structure = &self.prepared.structure;
        let parent = OccurrenceParent::Place(place);
        let OccurrenceNode::Place(base) = structure.edges.get(&(parent, OccurrenceEdge::PlaceBase))? else { return None; };
        let OccurrenceNode::Expression(index_at) = structure.edges.get(&(parent, OccurrenceEdge::PlaceIndex))? else { return None; };
        let base_type = structure.checked_types.places.get(base)?;
        let ScalarType::Array { element, length, .. } = base_type else { return None; };
        let constant = u64::try_from(concrete_integer_literal(index)?.as_ref()).ok()?;
        if constant >= *length || !super::super::scalar_type_equal(element, ty)
            || structure.checked_types.places.get(&place)? != ty
            || self.prepared.occurrences.get(index_at.0)?.span != super::super::typecheck::expression_span(index)
            || structure.checked_types.expressions.get(index_at)? != &ScalarType::U64
        { return None; }
        if let ScalarType::Struct(id) = ty {
            let declaration = self.module.structs.get(id.index)?;
            if id.source != self.module.source || declaration.id != *id || declaration.copy_policy != super::super::CopyPolicy::Copy {
                return None;
            }
        } else if !value_type(ty) { return None; }
        let NamedPlaceRootIdentity::ModuleValue(declaration) = self.prepared.prepared_named_place_root(*base)? else { return None; };
        self.module_value(declaration, base_type)?;
        Some(Effect::Read { at, declaration: declaration.clone(), index: Some(constant) })
    }
}

fn take(values: &mut Vec<EffectId>, count: usize) -> Option<Vec<EffectId>> {
    Some(values.split_off(values.len().checked_sub(count)?))
}

fn float_type(ty: &ScalarType) -> bool { matches!(ty, ScalarType::F32 | ScalarType::F64) }

fn value_type(ty: &ScalarType) -> bool { is_integer_type(ty) || *ty == ScalarType::Bool || float_type(ty) }

fn is_signed_integer(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128)
}
