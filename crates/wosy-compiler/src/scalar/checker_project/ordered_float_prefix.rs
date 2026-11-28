//! Typed, ordered prefix certificates. They are not Check results.
use super::*;
use crate::scalar::reference::{BlockOrdinal, NamedPlaceRootIdentity, OccurrenceEdge, OccurrenceNode, OccurrenceParent, PlainNameIdentity};
use wosy_syntax::SourceSpan;

type EffectId = usize;
use super::super::CoreOperationId;

#[derive(Clone, Debug)]
pub(super) enum Effect {
    Sequence { at: OccurrenceOrdinal, operands: Vec<EffectId>, kind: SequenceKind },
    LocalRead { at: OccurrenceOrdinal, binding: ReferenceBindingId, version: Option<EffectId> },
    Read { at: OccurrenceOrdinal, storage_site: OccurrenceOrdinal, declaration: SourceSpan, index: Option<u64>, projected: bool },
    Field { at: OccurrenceOrdinal, field: super::super::ScalarStructFieldId, receiver: EffectId },
    Branch { at: OccurrenceOrdinal, condition: EffectId, yes: EffectId, no: EffectId },
    ShortCircuit { at: OccurrenceOrdinal, conjunction: bool, left: EffectId, right: EffectId },
    Call { at: OccurrenceOrdinal, function: Option<PreparedCallable>, core: Option<CoreOperationId>, ty: ScalarType, arguments: Vec<EffectId> },
}

#[derive(Clone, Debug)]
pub(super) enum SequenceKind {
    Binary(BinaryOperator),
    Unary(UnaryOperator),
    Boolean(bool),
    Integer(ScalarType, BigInt),
    Float(ranked_float::Atom),
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
    pub(super) formals: Vec<(ReferenceBindingId, ScalarType)>,
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
    exact_mixed: bool,
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
    plan_prefix::<GUARDED>(project, calls, target)
}

pub(super) fn plan_prefix<const GUARDED: bool>(
    project: &ScalarProject, calls: &ProjectCallSiteResults, target: PreparedCallable,
) -> Option<PrefixCertificate> {
    let mut planner = Planner { project, calls, owner: target.0, exact_mixed: false, certificate: PrefixCertificate::default() };
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

/// Authenticate the complete effect of a prepared zero-argument Bool guard.
/// The ordered caller owns the position and returning alternatives. This
/// receipt does not supply a predicate outcome, availability or a witness.
pub(super) fn readonly_boolean_leaf(
    project: &ScalarProject, calls: &ProjectCallSiteResults, target: PreparedCallable,
    work: &mut usize,
) -> Option<()> {
    let mut planner = Planner { project, calls, owner: target.0, exact_mixed: false, certificate: PrefixCertificate::default() };
    let frame = planner.frame(target)?;
    let body = frame.prepared;
    let structure = &body.structure;
    if !body.boundary.parameters.is_empty()
        || !matches!(body.boundary.results.as_slice(), [output] if output.ty == ScalarType::Bool)
        || !body.call_occurrences.is_empty() || !structure.checker_direct_targets.is_empty()
        || !structure.checker_directives.is_empty() || !body.assignments.is_empty()
        || !body.callable_escapes.is_empty() || !body.allocation_length_calls.is_empty()
    { return None; }
    super::super::checker_flow::charge_call_work(work, body.occurrences.len())?;
    let (closed, dependencies) = planner.complete_callee(target)?;
    if !dependencies.is_empty() || !closed.bindings.is_empty() { return None; }
    // Keep the existing integer/Bool-only public guard lane. The camera's
    // general float effects are not production evidence of guard outcomes.
    for effect in &planner.certificate.effects {
        let at = match effect.operation {
            Effect::Sequence { at, .. } | Effect::Read { at, .. } | Effect::ShortCircuit { at, .. } => at,
            _ => return None,
        };
        let ty = structure.checked_types.expressions.get(&at)?;
        if !(*ty == ScalarType::Bool || is_integer_type(ty)) { return None; }
        if matches!(&effect.operation, Effect::Sequence { operands, .. } if !operands.is_empty())
            && *ty != ScalarType::Bool { return None; }
    }
    Some(())
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
        let formals = frame.prepared.boundary.checker_parameter_bindings.as_deref()?.iter()
            .zip(&frame.prepared.boundary.parameters).map(|(id, parameter)| (id.clone(), parameter.ty.clone())).collect();
        Some((FunctionEffects { bindings, returned, formals }, deps))
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
            Sequence(OccurrenceOrdinal, usize, SequenceKind),
            Field(OccurrenceOrdinal, super::super::ScalarStructFieldId),
            Branch(OccurrenceOrdinal),
            Short(OccurrenceOrdinal, bool),
            Call(OccurrenceOrdinal, Option<PreparedCallable>, Option<CoreOperationId>, ScalarType, usize),
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
                Work::Sequence(at, arity, kind) => Effect::Sequence { at, operands: take(&mut operands, arity)?, kind },
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
                Work::Call(at, function, core, ty, arity) => Effect::Call { at, function, core, ty, arguments: take(&mut operands, arity)? },
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
                                Effect::Read { at, storage_site: at, declaration: declaration.clone(), index: None, projected: false }
                            }
                            _ => return None,
                        },
                        ScalarExpression::Member { .. } if value_type(ty) => {
                            let (declaration, field) = frame.prepared.prepared_module_direct_member(at)?;
                            frame.field(&field.structure, field, ty)?;
                            frame.module_value(declaration, &ScalarType::Struct(field.structure.clone()))?;
                            let receiver = self.certificate.effects.len();
                            self.certificate.effects.push(EffectRecord { owner: frame.target,
                                operation: Effect::Read { at, storage_site: at, declaration: declaration.clone(), index: None, projected: true },
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
                                Equal | NotEqual if *ty == ScalarType::Bool && value_type(kind) => Work::Sequence(at, 2, SequenceKind::Binary(operator.clone())),
                                Less | LessEqual | Greater | GreaterEqual
                                    if *ty == ScalarType::Bool && (is_integer_type(kind) || float_type(kind)) => Work::Sequence(at, 2, SequenceKind::Binary(operator.clone())),
                                Add | Subtract | Multiply | Divide if float_type(ty) && kind == ty => Work::Sequence(at, 2, SequenceKind::Binary(operator.clone())),
                                BitAnd | BitXor | Multiply if self.exact_mixed && *ty == ScalarType::I64 && kind == ty =>
                                    Work::Sequence(at, 2, SequenceKind::Binary(operator.clone())),
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
                            pending.push(Work::Sequence(at, 1, SequenceKind::Unary(operator.clone())));
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
                            let (target,core) = match &frame.prepared.resolved_call_site(at)?.target {
                                ResolvedCallTarget::Core(op @ CoreOperationId::SintToFloat)
                                    if float_type(ty) && matches!(signature.parameters.as_slice(), [arg] if is_signed_integer(arg)) =>
                                    (None,Some(*op)),
                                ResolvedCallTarget::Core(op) if self.exact_mixed && match op {
                                    CoreOperationId::FloatExtend => *ty == ScalarType::F64 && signature.parameters.as_slice() == [ScalarType::F32],
                                    CoreOperationId::FloatToSintTrunc => *ty == ScalarType::I32 && signature.parameters.as_slice() == [ScalarType::F64],
                                    CoreOperationId::IntExtend => *ty == ScalarType::I64 && signature.parameters.as_slice() == [ScalarType::I32],
                                    CoreOperationId::IntTrunc => *ty == ScalarType::I32 && signature.parameters.as_slice() == [ScalarType::I64],
                                    _ => false,
                                } => (None,Some(*op)),
                                ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration } => {
                                    let target = *structure.checker_direct_targets.get(&at)?;
                                    let callee = self.frame(target)?;
                                    if target == frame.target || source != &callee.module.source || *declaration_span != callee.function.name_span
                                        || !callee.prepared.boundary.parameters.iter().map(|slot| &slot.ty).eq(&signature.parameters)
                                        || !matches!(callee.prepared.boundary.results.as_slice(), [result] if &result.ty == ty)
                                    { return None; }
                                    deps.push(target);
                                    (Some(target),None)
                                }
                                _ => return None,
                            };
                            pending.push(Work::Call(at, target, core, ty.clone(), arguments.len()));
                            for (index, argument) in arguments.iter().enumerate().rev() {
                                let argument_at = child(at, OccurrenceEdge::CallArgument(index))?;
                                if structure.checked_types.expressions.get(&argument_at)? != signature.parameters.get(index)? {
                                    return None;
                                }
                                pending.push(Work::Visit(argument_at, argument));
                            }
                            continue;
                        }
                        ScalarExpression::Boolean { value, .. } if *ty == ScalarType::Bool => Effect::Sequence { at, operands: Vec::new(), kind: SequenceKind::Boolean(*value) },
                        ScalarExpression::Float { .. } if float_type(ty) => Effect::Sequence { at, operands: Vec::new(), kind: SequenceKind::Float(ranked_float::Atom::literal(expr, ty)?) },
                        _ if is_integer_type(ty) && integer_literal_fits_type(concrete_integer_literal(expr)?.as_ref(), ty) =>
                            Effect::Sequence { at, operands: Vec::new(), kind: SequenceKind::Integer(ty.clone(), concrete_integer_literal(expr)?.into_owned()) },
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
        Some(Effect::Read { at, storage_site: *base, declaration: declaration.clone(), index: Some(constant), projected: true })
    }
}

fn take<T>(values: &mut Vec<T>, count: usize) -> Option<Vec<T>> {
    Some(values.split_off(values.len().checked_sub(count)?))
}

fn float_type(ty: &ScalarType) -> bool { matches!(ty, ScalarType::F32 | ScalarType::F64) }

fn value_type(ty: &ScalarType) -> bool { is_integer_type(ty) || *ty == ScalarType::Bool || float_type(ty) }

fn is_signed_integer(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) enum ExactValue {
    Boolean(bool),
    Enum(super::super::ScalarEnumId,u32),
    Integer(ScalarType, BigInt),
    F32(super::super::checker_flow::projected_f32::ExactF32),
    F64(u64),
}

impl ExactValue {
    pub(in super::super) fn ty(&self) -> ScalarType {
        match self {
            Self::Boolean(_) => ScalarType::Bool,
            Self::Enum(id,_) => ScalarType::Enum(id.clone()),
            Self::Integer(ty, _) => ty.clone(),
            Self::F32(_) => ScalarType::F32,
            Self::F64(_) => ScalarType::F64,
        }
    }
}

/// Execute only the reached certified operations on one authenticated row.
/// The caller owns storage and supplies current reads and real call-entry
/// formals. This cannot fill in any unknown value or publish a Check result.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(in super::super) enum ObservedStep {
    Branch { site: SourceSpan, accepted: bool },
    // Call and return bracket this exact nested trace. IDs are local to this
    // activation's canonical postorder trace table.
    Call { site: SourceSpan, nested: usize },
    Entered { site: SourceSpan, nested: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct ObservedTrace {
    pub(super) root: Vec<ObservedStep>,
    pub(super) segments: Vec<Vec<ObservedStep>>,
    pub(super) physical_cost: usize,
    fingerprint: u64,
}

impl ObservedTrace {
    pub(super) fn new(root: Vec<ObservedStep>, segments: Vec<Vec<ObservedStep>>, work: &mut usize) -> Option<Self> {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let physical_cost = segments.iter().fold(root.len().saturating_add(1), |cost, child|
            cost.saturating_add(child.len()).saturating_add(1));
        super::super::checker_flow::charge_call_work(work, physical_cost)?;
        let mut digest = DefaultHasher::new();
        root.hash(&mut digest);
        segments.hash(&mut digest);
        Some(Self { root, segments, physical_cost, fingerprint: digest.finish() })
    }
}

impl std::hash::Hash for ObservedTrace {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Fingerprint accelerates lookup; derived Eq still compares the exact
        // canonical root and segments when hashes collide.
        state.write_u64(self.fingerprint);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum MemoValue { Bool(bool), Enum(SourceIdentity,usize,u32), Int(BigInt), F32(u32), F64(u64) }

impl From<&ExactValue> for MemoValue {
    fn from(value: &ExactValue) -> Self {
        match value {
            ExactValue::Boolean(value) => Self::Bool(*value),
            ExactValue::Enum(id,tag) => Self::Enum(id.source.clone(),id.index,*tag),
            ExactValue::Integer(_, value) => Self::Int(value.clone()),
            ExactValue::F32(value) => Self::F32(value.0),
            ExactValue::F64(bits) => Self::F64(*bits),
        }
    }
}

type ObservedResult = (Vec<ExactValue>, Option<ObservedTrace>);

pub(super) fn plan_observed_math_call(project: &ScalarProject, calls: &ProjectCallSiteResults, target: PreparedCallable)
    -> Option<(PrefixCertificate, FunctionEffects)> {
    let mut planner = Planner { project, calls, owner: target.0, exact_mixed: true, certificate: PrefixCertificate::default() };
    let mut installed = Vec::new();
    planner.seal_dependencies(vec![target], &mut installed)?;
    // This math callback has no mutable/snapshot dependencies: even a typed
    // module read is rejected here and may not be equated to a constant.
    if planner.certificate.effects.iter().any(|effect| matches!(effect.operation, Effect::Read { .. } | Effect::Field { .. })) {
        return None;
    }
    let root = planner.certificate.callees.remove(&target)?;
    Some((planner.certificate, root))
}

pub(super) fn evaluate_observed_prefix(
    certificate: &PrefixCertificate,
    returning: Option<&FunctionEffects>,
    mut formals: HashMap<ReferenceBindingId, ExactValue>,
    mut read: impl FnMut(&SourceSpan, Option<u64>, Option<&super::super::ScalarStructFieldId>) -> Option<ExactValue>,
    trace: bool,
    work: &mut usize,
    resolve_span: impl Fn(PreparedCallable, OccurrenceOrdinal) -> Option<SourceSpan>,
) -> Option<ObservedResult> {
    use super::super::checker_flow::{charge_call_work, projected_f32::ExactF32};
    use std::cell::RefCell;
    type Locals = Rc<RefCell<HashMap<ReferenceBindingId, ExactValue>>>;
    type Key = (PreparedCallable, Vec<MemoValue>);
    enum Work<'a> {
        Eval(EffectId, Locals),
        Bind(&'a BindingFact, Locals),
        Select(PreparedCallable, OccurrenceOrdinal, EffectId, EffectId, Locals),
        Short(PreparedCallable, OccurrenceOrdinal, bool, EffectId, Locals),
        Sequence(&'a SequenceKind, usize),
        Call(PreparedCallable, OccurrenceOrdinal, Option<PreparedCallable>, Option<CoreOperationId>, &'a ScalarType, usize),
        CheckedReturn(PreparedCallable, OccurrenceOrdinal, &'a ScalarType, Key),
        Publish,
        DropLocalResult,
    }
    fn compute(kind: &SequenceKind, args: Vec<ExactValue>) -> Option<ExactValue> {
        use ExactValue::*;
        Some(match (kind, args.as_slice()) {
            (SequenceKind::Boolean(value), []) => Boolean(*value),
            (SequenceKind::Integer(ty, value), []) => Integer(ty.clone(), value.clone()),
            (SequenceKind::Float(ranked_float::Atom::Single(bits)), []) => F32(ExactF32::checked_bits(*bits)?),
            (SequenceKind::Float(ranked_float::Atom::Double(bits)), []) if f64::from_bits(*bits).is_finite() => F64(*bits),
            (SequenceKind::Binary(BinaryOperator::Multiply), [F64(left),F64(right)]) => {
                let v=f64::from_bits(*left)*f64::from_bits(*right);
                if !v.is_finite() { return None; }
                F64(v.to_bits())
            }
            (SequenceKind::Binary(op @ (BinaryOperator::BitAnd | BinaryOperator::BitXor | BinaryOperator::Multiply)),
                [Integer(ScalarType::I64,left),Integer(ScalarType::I64,right)]) => {
                let (l,r)=(i64::try_from(left).ok()?,i64::try_from(right).ok()?);
                let value=match op {
                    BinaryOperator::BitAnd => l&r,
                    BinaryOperator::BitXor => l^r,
                    BinaryOperator::Multiply => l.checked_mul(r)?,
                    _ => return None,
                };
                Integer(ScalarType::I64,value.into())
            }
            (SequenceKind::Unary(UnaryOperator::LogicalNot), [Boolean(value)]) => Boolean(!value),
            (SequenceKind::Unary(UnaryOperator::Negate), [F32(value)]) => F32(value.negate()),
            (SequenceKind::Binary(op @ (BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply | BinaryOperator::Divide)),
                [F32(left), F32(right)]) => F32(left.binary(op, *right)?),
            (SequenceKind::Binary(op), [F32(left), F32(right)]) => Boolean(
                super::super::symbolic_integer::IntegerRelation::from_operator(op)?
                    .accepts(f32::from_bits(left.0).partial_cmp(&f32::from_bits(right.0))?)),
            (SequenceKind::Binary(op), [Integer(left_ty, left), Integer(right_ty, right)]) if left_ty == right_ty =>
                Boolean(super::super::symbolic_integer::IntegerRelation::from_operator(op)?.accepts(left.cmp(right))),
            _ => return None,
        })
    }
    charge_call_work(work, formals.len().saturating_add(returning.map_or(certificate.bindings.len(), |function| function.bindings.len()).saturating_mul(3)))?;
    let locals = Rc::new(RefCell::new(std::mem::take(&mut formals)));
    let mut pending = Vec::new();
    let mut values = Vec::new();
    let mut published = Vec::new();
    // Scoped to this activation: the reader authenticates one immutable module
    // snapshot for the whole readonly prefix and all its callees. The checked
    // target signature fixes integer width/signedness for each memo position.
    // One last specialization per checked declaration bounds the memo by the
    // static call graph, even when the exact dynamic arguments all differ.
    // Replacement changes only reuse: every miss still executes exactly.
    let mut memo = HashMap::<PreparedCallable, (Vec<MemoValue>, ExactValue, Option<usize>)>::new();
    let mut trace_stack = trace.then(|| vec![Vec::new()]);
    let mut interned = HashMap::new();
    let mut segments = Vec::new();
    let bindings = returning.map_or(certificate.bindings.as_slice(), |function| {
        pending.push(Work::Publish);
        pending.push(Work::Eval(function.returned, Rc::clone(&locals)));
        function.bindings.as_slice()
    });
    for fact in bindings.iter().rev() {
        pending.push(if returning.is_some() { Work::DropLocalResult } else { Work::Publish });
        pending.push(Work::Bind(fact, Rc::clone(&locals)));
        pending.push(Work::Eval(fact.returning_effect, Rc::clone(&locals)));
    }
    while let Some(task) = pending.pop() {
        charge_call_work(work, 1)?;
        match task {
            Work::Eval(at, scope) => {
                let record = certificate.effects.get(at)?;
                match &record.operation {
                    Effect::LocalRead { binding, .. } => values.push(scope.borrow().get(binding)?.clone()),
                    Effect::Read { declaration, index, .. } => values.push(read(declaration, *index, None)?),
                    Effect::Branch { at, condition, yes, no } => {
                        pending.push(Work::Select(record.owner, *at, *yes, *no, Rc::clone(&scope)));
                        pending.push(Work::Eval(*condition, scope));
                    }
                    Effect::ShortCircuit { at, conjunction, left, right } => {
                        pending.push(Work::Short(record.owner, *at, *conjunction, *right, Rc::clone(&scope)));
                        pending.push(Work::Eval(*left, scope));
                    }
                    Effect::Sequence { kind, operands, .. } => {
                        charge_call_work(work, operands.len())?;
                        pending.push(Work::Sequence(kind, operands.len()));
                        pending.extend(operands.iter().rev().map(|id| Work::Eval(*id, Rc::clone(&scope))));
                    }
                    Effect::Call { at, function, core, arguments, ty } => {
                        charge_call_work(work, arguments.len())?;
                        pending.push(Work::Call(record.owner, *at, *function, *core, ty, arguments.len()));
                        pending.extend(arguments.iter().rev().map(|id| Work::Eval(*id, Rc::clone(&scope))));
                    }
                    Effect::Field { field, receiver, .. } => {
                        let receiver = certificate.effects.get(*receiver)?;
                        if receiver.owner != record.owner { return None; }
                        let Effect::Read { declaration, index, projected: true, .. } = &receiver.operation else { return None; };
                        values.push(read(declaration, *index, Some(field))?);
                    }
                }
            }
            Work::Select(owner, at, yes, no, scope) => {
                let ExactValue::Boolean(accepted) = values.pop()? else { return None; };
                if let Some(stack) = &mut trace_stack {
                    charge_call_work(work, 1)?;
                    stack.last_mut()?.push(ObservedStep::Branch { site: resolve_span(owner, at)?, accepted });
                }
                pending.push(Work::Eval(if accepted { yes } else { no }, scope));
            }
            Work::Short(owner, at, conjunction, right, scope) => {
                let ExactValue::Boolean(left) = values.pop()? else { return None; };
                if let Some(stack) = &mut trace_stack {
                    charge_call_work(work, 1)?;
                    stack.last_mut()?.push(ObservedStep::Branch { site: resolve_span(owner, at)?, accepted: left });
                }
                if left == conjunction {
                    pending.push(Work::Eval(right, scope));
                } else { values.push(ExactValue::Boolean(left)); }
            }
            Work::Bind(fact, scope) => {
                let value = values.pop()?;
                if value.ty() != fact.ty || scope.borrow_mut().insert(fact.slot.clone(), value.clone()).is_some() { return None; }
                values.push(value);
            }
            Work::Publish => published.push(values.pop()?),
            Work::DropLocalResult => { values.pop()?; }
            Work::Sequence(kind, arity) => {
                charge_call_work(work, arity)?;
                let args = take(&mut values, arity)?;
                values.push(compute(kind, args)?);
            }
            Work::Call(owner, at, target, core, result, arity) => {
                charge_call_work(work, arity)?;
                let args = take(&mut values, arity)?;
                if let Some(target) = target {
                    let function = certificate.callees.get(&target)?;
                    if args.len() != function.formals.len() { return None; }
                    if !function.formals.iter().zip(&args).all(|((_, ty), value)| value.ty() == *ty) { return None; }
                    charge_call_work(work, args.iter().fold(1usize, |cost, value| {
                        let words = match value { ExactValue::Integer(_, value) => (value.bits().saturating_add(63))/64, _ => 1 };
                        cost.saturating_add(usize::try_from(words).unwrap_or(usize::MAX))
                    }))?;
                    let key: Key = (target, args.iter().map(MemoValue::from).collect());
                    if let Some((_, value, nested)) = memo.get(&target).filter(|(known, _, _)| known == &key.1) {
                        if value.ty() != *result { return None; }
                        if let Some(stack) = &mut trace_stack {
                            charge_call_work(work, 1)?;
                            stack.last_mut()?.push(ObservedStep::Call { site: resolve_span(owner, at)?, nested: (*nested)? });
                        }
                        values.push(value.clone());
                        continue;
                    }
                    charge_call_work(work, function.formals.len().saturating_add(function.bindings.len().saturating_mul(3)))?;
                    let nested = function.formals.iter().zip(args).map(|((slot, _), value)| (slot.clone(), value)).collect();
                    let scope = Rc::new(RefCell::new(nested));
                    if let Some(stack) = &mut trace_stack { stack.push(Vec::new()); }
                    pending.push(Work::CheckedReturn(owner, at, result, key));
                    pending.push(Work::Eval(function.returned, Rc::clone(&scope)));
                    for fact in function.bindings.iter().rev() {
                        pending.push(Work::DropLocalResult);
                        pending.push(Work::Bind(fact, Rc::clone(&scope)));
                        pending.push(Work::Eval(fact.returning_effect, Rc::clone(&scope)));
                    }
                } else {
                    use ExactValue::{F32,F64,Integer};
                    let value = match (core?,result,args.as_slice()) {
                        (CoreOperationId::SintToFloat,ScalarType::F32,[Integer(ScalarType::I32,value)]) =>
                            F32(ExactF32::signed_i32(i32::try_from(value).ok()?)),
                        (CoreOperationId::FloatExtend,ScalarType::F64,[F32(value)]) =>
                            F64(f64::from(f32::from_bits(value.0)).to_bits()),
                        (CoreOperationId::FloatToSintTrunc,ScalarType::I32,[F64(bits)]) => {
                            let v=f64::from_bits(*bits).trunc();
                            if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&v) { return None; }
                            Integer(ScalarType::I32,(v as i32).into())
                        }
                        (CoreOperationId::IntExtend,ScalarType::I64,[Integer(ScalarType::I32,value)]) =>
                            Integer(ScalarType::I64,i64::from(i32::try_from(value).ok()?).into()),
                        (CoreOperationId::IntTrunc,ScalarType::I32,[Integer(ScalarType::I64,value)]) =>
                            Integer(ScalarType::I32,(i64::try_from(value).ok()? as i32).into()),
                        _ => return None,
                    };
                    values.push(value);
                }
            }
            Work::CheckedReturn(owner, at, ty, key) => {
                let value = values.last()?;
                if value.ty() != *ty { return None; }
                let nested = if let Some(stack) = &mut trace_stack {
                    let steps = stack.pop()?;
                    charge_call_work(work, steps.len().saturating_add(1))?;
                    let id = *interned.entry(steps).or_insert_with_key(|steps| {
                        let id = segments.len();
                        segments.push(steps.clone());
                        id
                    });
                    stack.last_mut()?.push(ObservedStep::Call { site: resolve_span(owner, at)?, nested: id });
                    Some(id)
                } else { None };
                memo.insert(key.0, (key.1, value.clone(), nested));
            }
        }
    }
    if !values.is_empty() { return None; }
    let trace = if let Some(mut stack) = trace_stack {
        let root = stack.pop()?;
        if !stack.is_empty() { return None; }
        Some(ObservedTrace::new(root, segments, work)?)
    } else { None };
    Some((published, trace))
}
