#![allow(dead_code)]

use std::collections::{BTreeSet, VecDeque};

use crate::scalar::control::{
    BindingDefinition, CallableBodyCallNodes, CallablePointFacts, DirectDestinationIdentity,
    NamedPlaceRootIdentity, PlainNameIdentity,
};
use crate::scalar::{
    CfgPointId, CfgPointKind, ConcreteCallSelection, CopyPolicy, CoreOperationId,
    ReferenceBindingId, ResolvedCallTarget, ScalarReferenceCfg, ScalarStructFieldId,
};
use crate::{
    ScalarBlock, ScalarBlockItem, ScalarBuildValidation, ScalarExpression, ScalarFunction,
    ScalarItem, ScalarPlace, ScalarType,
};

type Body = (usize, usize);
type Formal = (usize, usize, usize);

#[derive(Debug)]
pub(super) struct Edge {
    pub(super) from: Body,
    pub(super) site: CfgPointId,
    pub(super) to: Formal,
}

#[derive(Debug)]
pub(super) struct Proof {
    pub(super) owner: ReferenceBindingId,
    pub(super) field: ScalarStructFieldId,
    pub(super) output: usize,
    pub(super) edges: Vec<Edge>,
}

#[derive(Debug)]
pub(super) enum Unproved {
    Missing(Body, &'static str),
    Unsupported(Body, &'static str),
    Call(
        Body,
        CfgPointId,
        ResolvedCallTarget,
        Vec<Option<ScalarType>>,
    ),
}

struct View<'a> {
    build: &'a ScalarBuildValidation<'a>,
    body: Body,
    function: &'a ScalarFunction,
    nodes: CallableBodyCallNodes<'a>,
    facts: &'a CallablePointFacts,
    cfg: &'a ScalarReferenceCfg,
}

impl<'a> View<'a> {
    fn new(build: &'a ScalarBuildValidation<'a>, body: Body) -> Option<Self> {
        let (m, i) = body;
        let ScalarItem::Function(function) =
            build.prepared().project().modules.get(m)?.items.get(i)?
        else {
            return None;
        };
        Some(Self {
            build,
            body,
            function,
            nodes: build
                .prepared()
                .bind_callable_body_call_nodes(m, i, None, &function.body)
                .ok()?,
            facts: build.callable_point_facts(m, i, None)?,
            cfg: build.reference_cfg(m, i, None)?,
        })
    }

    fn eval(&self, value: &ScalarExpression) -> Option<CfgPointId> {
        self.nodes.evaluation_point(self.facts, value)
    }

    fn literal_field(&self, value: &ScalarExpression) -> Option<&ScalarStructFieldId> {
        let field = self.nodes.literal_field(value)?;
        #[cfg(test)]
        if tests::is_withheld(tests::Withheld::LiteralField(self.body, field.index)) {
            return None;
        }
        Some(field)
    }

    fn formal(&self, parameter: usize) -> Option<(&ReferenceBindingId, &ScalarType)> {
        let formal = self.nodes.formal(self.facts, parameter)?;
        #[cfg(test)]
        if tests::is_withheld(tests::Withheld::Formal((
            self.body.0,
            self.body.1,
            parameter,
        ))) {
            return None;
        }
        Some(formal)
    }

    fn lexical(&self, value: &ScalarExpression) -> Option<&ReferenceBindingId> {
        match self.nodes.plain_name(value)? {
            PlainNameIdentity::Lexical(binding) => Some(binding),
            _ => None,
        }
    }

    fn read(&self, value: &ScalarExpression) -> Option<(&ReferenceBindingId, CfgPointId)> {
        let binding = self.lexical(value)?;
        let id = self.eval(value)?;
        matches!(&self.cfg.points.get(id.0)?.kind, CfgPointKind::Read { binding: read } if read == binding)
            .then_some((binding, id))
    }

    fn declaration(&self, bind: CfgPointId) -> Option<&ReferenceBindingId> {
        match &self.cfg.points.get(bind.0)?.kind {
            CfgPointKind::Bind { binding } => Some(binding),
            _ => None,
        }
    }

    fn unique_definition(
        &self,
        binding: &ReferenceBindingId,
        read: CfgPointId,
    ) -> Option<CfgPointId> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum State {
            Absent,
            Defined(CfgPointId),
            Conflict,
        }
        let mut before = vec![None; self.cfg.points.len()];
        *before.first_mut()? = Some(State::Absent);
        let mut work = VecDeque::from([CfgPointId(0)]);
        while let Some(id) = work.pop_front() {
            let point = self.cfg.points.get(id.0)?;
            let after = match &point.kind {
                CfgPointKind::Bind { binding: bound } if bound == binding => State::Defined(id),
                CfgPointKind::Assign { target, .. } if &target.binding == binding => {
                    if target.projections.is_empty() {
                        State::Defined(id)
                    } else {
                        State::Conflict
                    }
                }
                CfgPointKind::Move { place, .. } if &place.binding == binding => State::Absent,
                CfgPointKind::AssignThrough { .. }
                | CfgPointKind::MoveThrough { .. }
                | CfgPointKind::DeferredMoveThrough { .. }
                | CfgPointKind::Invalidate { .. }
                | CfgPointKind::Release { .. } => State::Conflict,
                _ => before[id.0]?,
            };
            for next in &point.successors {
                let row = before.get_mut(next.0)?;
                let joined = match *row {
                    Some(old) if old != after => State::Conflict,
                    _ => after,
                };
                if *row != Some(joined) {
                    *row = Some(joined);
                    work.push_back(*next);
                }
            }
        }
        match before.get(read.0)?.as_ref()? {
            State::Defined(definition) => Some(*definition),
            _ => None,
        }
    }
}

fn bytes(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::RuntimeArray { element, .. } if **element == ScalarType::U8)
}

fn scalar(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::Unit
            | ScalarType::Bool
            | ScalarType::I32
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
    )
}

struct Trace<'a> {
    owner: &'a ReferenceBindingId,
    allocation: CfgPointId,
    field: ScalarStructFieldId,
    address: CfgPointId,
    literal: CfgPointId,
    field_read: CfgPointId,
    result_read: CfgPointId,
    raw_binding: &'a ReferenceBindingId,
    descriptor: &'a ReferenceBindingId,
}

fn trace<'a>(view: &'a View<'_>) -> Option<Trace<'a>> {
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &view.function.signature
    else {
        return None;
    };
    if parameters.as_slice() != [ScalarType::U8] {
        return None;
    }
    let [output] = outputs.outputs.as_slice() else {
        return None;
    };
    let ScalarType::Struct(struct_id) = &output.ty else {
        return None;
    };
    let structure = view
        .build
        .prepared()
        .project()
        .modules
        .iter()
        .flat_map(|m| &m.structs)
        .find(|s| &s.id == struct_id)?;
    if structure.copy_policy != CopyPolicy::Move || structure.fields.len() != 2 {
        return None;
    }
    let field = structure
        .fields
        .iter()
        .find(|field| field.ty == ScalarType::RawPointer(Box::new(ScalarType::U8)))?;
    if structure
        .fields
        .iter()
        .filter(|field| field.ty == ScalarType::U64)
        .count()
        != 1
    {
        return None;
    }
    let mut returns = view.function.body.final_expressions();
    let (result, _) = returns.next()?;
    if returns.next().is_some() || view.nodes.final_output(result) != Some(0) {
        return None;
    }
    let (descriptor, result_read) = view.read(result)?;
    let mut outputs = view.cfg.points.iter().filter_map(|p| match p.kind {
        CfgPointKind::ReturnOutput {
            output,
            value_point,
            checked: false,
        } => Some((output, value_point)),
        _ => None,
    });
    if outputs.next()? != (0, result_read) || outputs.next().is_some() {
        return None;
    }
    let definition = view.unique_definition(descriptor, result_read)?;
    let (literal, fields) = view.function.body.items.iter().find_map(|item| {
        let ScalarBlockItem::LocalBinding(binding) = item else {
            return None;
        };
        let BindingDefinition::Initialized { bind, value } =
            view.nodes.binding_definition(view.facts, binding, 0)?
        else {
            return None;
        };
        let ScalarExpression::StructLiteral { fields, .. } = &binding.value else {
            return None;
        };
        (bind == definition
            && view.eval(&binding.value) == Some(value)
            && binding.declared_type == output.ty)
            .then_some((value, fields))
    })?;
    let [first, second] = fields.as_slice() else {
        return None;
    };
    let first_id = view.literal_field(&first.value)?;
    let second_id = view.literal_field(&second.value)?;
    if first_id == second_id
        || first_id.structure != *struct_id
        || second_id.structure != *struct_id
    {
        return None;
    }
    let (data, _) = [(first, first_id), (second, second_id)]
        .into_iter()
        .find(|(_, id)| *id == &field.id)?;
    let (raw_binding, field_read) = view.read(&data.value)?;
    let install = view.unique_definition(raw_binding, field_read)?;
    let CfgPointKind::Assign {
        target, rhs_point, ..
    } = &view.cfg.points.get(install.0)?.kind
    else {
        return None;
    };
    if &target.binding != raw_binding || !target.projections.is_empty() {
        return None;
    }
    let address = *rhs_point;
    let mut walk = Vec::new();
    collect_addresses(&view.function.body, &mut walk);
    let place = walk.into_iter().find_map(|value| match value {
        ScalarExpression::RawAddress { place, .. } if view.eval(value) == Some(address) => {
            Some(place)
        }
        _ => None,
    })?;
    let ScalarPlace::Index { base, index, .. } = place else {
        return None;
    };
    if view.nodes.checked_place_type(place)? != &ScalarType::U8
        || view.nodes.checked_expression_type(index)? != &ScalarType::U64
        || !matches!(index.as_ref(), ScalarExpression::Integer { value, .. } if *value == 0.into())
    {
        return None;
    }
    let NamedPlaceRootIdentity::Lexical(owner) = view.nodes.named_place_root(base)? else {
        return None;
    };
    if !view.cfg.allocations.contains(owner) {
        return None;
    }
    let allocation = view.function.body.items.iter().find_map(|item| {
        let ScalarBlockItem::LocalBinding(binding) = item else {
            return None;
        };
        let BindingDefinition::Allocation { bind } =
            view.nodes.binding_definition(view.facts, binding, 0)?
        else {
            return None;
        };
        (view.declaration(bind)? == owner && bytes(&binding.declared_type)).then_some(bind)
    })?;
    if !dominates(view.cfg, allocation, address) || !dominates(view.cfg, allocation, result_read) {
        return None;
    }
    Some(Trace {
        owner,
        allocation,
        field: field.id.clone(),
        address,
        literal,
        field_read,
        result_read,
        raw_binding,
        descriptor,
    })
}

fn collect_addresses<'a>(block: &'a ScalarBlock, values: &mut Vec<&'a ScalarExpression>) {
    for item in &block.items {
        match item {
            ScalarBlockItem::Assignment(assignment) => {
                values.extend(&assignment.values);
            }
            ScalarBlockItem::Expression(ScalarExpression::Block(inner)) => {
                collect_addresses(inner, values)
            }
            ScalarBlockItem::Expression(ScalarExpression::If {
                then_branch,
                else_branch,
                ..
            }) => {
                collect_addresses(then_branch, values);
                collect_addresses(else_branch, values);
            }
            ScalarBlockItem::Expression(ScalarExpression::UnitIf { then_branch, .. }) => {
                collect_addresses(then_branch, values)
            }
            _ => {}
        }
    }
}

pub(super) fn prove(
    build: &ScalarBuildValidation<'_>,
    module: usize,
    item: usize,
) -> Result<Proof, Unproved> {
    let body = (module, item);
    if !build.diagnostics().is_empty() {
        return Err(Unproved::Missing(body, "validated body"));
    }
    let view = View::new(build, body).ok_or(Unproved::Missing(body, "producer body"))?;
    let trace = trace(&view).ok_or(Unproved::Unsupported(body, "local allocation trace"))?;
    let mut edges = Vec::new();
    Uses {
        view: &view,
        owner: trace.owner,
        trace: Some(&trace),
        edges: &mut edges,
    }
    .check()?;
    let mut visited = BTreeSet::new();
    let mut cursor = 0;
    while let Some(edge) = edges.get(cursor) {
        let (m, i, parameter) = edge.to;
        cursor += 1;
        if !visited.insert((m, i, parameter)) {
            continue;
        }
        let body = (m, i);
        let helper = View::new(build, body).ok_or(Unproved::Missing(body, "helper body"))?;
        let (owner, ty) = helper
            .formal(parameter)
            .ok_or(Unproved::Missing(body, "helper formal"))?;
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &helper.function.signature
        else {
            return Err(Unproved::Unsupported(body, "helper signature"));
        };
        if !bytes(ty)
            || parameters
                .iter()
                .enumerate()
                .any(|(i, ty)| i != parameter && !scalar(ty))
            || outputs.outputs.len() > 1
            || outputs.outputs.iter().any(|output| !scalar(&output.ty))
        {
            return Err(Unproved::Unsupported(body, "helper signature"));
        }
        Uses {
            view: &helper,
            owner,
            trace: None,
            edges: &mut edges,
        }
        .check()?;
    }
    Ok(Proof {
        owner: trace.owner.clone(),
        field: trace.field,
        output: 0,
        edges,
    })
}

struct Uses<'a, 'b> {
    view: &'a View<'b>,
    owner: &'a ReferenceBindingId,
    trace: Option<&'a Trace<'a>>,
    edges: &'a mut Vec<Edge>,
}

impl Uses<'_, '_> {
    fn unsupported(&self, message: &'static str) -> Unproved {
        Unproved::Unsupported(self.view.body, message)
    }
    fn missing(&self, message: &'static str) -> Unproved {
        Unproved::Missing(self.view.body, message)
    }

    fn check(&mut self) -> Result<(), Unproved> {
        for p in &self.view.cfg.points {
            match &p.kind {
                CfgPointKind::Release { .. }
                | CfgPointKind::Invalidate { .. }
                | CfgPointKind::AssignThrough { .. }
                | CfgPointKind::MoveThrough { .. }
                | CfgPointKind::DeferredMoveThrough { .. }
                | CfgPointKind::DeferredMove { .. } => {
                    return Err(self.unsupported("release or indirect storage effect"));
                }
                _ => {}
            }
        }
        self.block(&self.view.function.body)
    }

    fn block(&mut self, block: &ScalarBlock) -> Result<(), Unproved> {
        let view = self.view;
        for item in &block.items {
            match item {
                ScalarBlockItem::LocalBinding(binding) => {
                    let definition = view
                        .nodes
                        .binding_definition(view.facts, binding, 0)
                        .ok_or(self.missing("binding definition"))?;
                    if binding.receivers.len() != 1 {
                        return Err(self.unsupported("multi-output binding"));
                    }
                    match definition {
                        BindingDefinition::Allocation { .. } => {
                            if !bytes(&binding.declared_type)
                                || binding.receivers.iter().any(|r| {
                                    !matches!(
                                        r.allocation_length,
                                        Some(ScalarExpression::Integer { .. })
                                    )
                                })
                            {
                                return Err(self.unsupported("allocation size or type"));
                            }
                        }
                        BindingDefinition::Initialized { bind, value } => {
                            let identity = view
                                .declaration(bind)
                                .ok_or(self.missing("binding identity"))?;
                            if self
                                .trace
                                .is_some_and(|trace| trace.raw_binding == identity)
                            {
                                if !matches!(
                                    view.nodes.plain_name(&binding.value),
                                    Some(PlainNameIdentity::Null)
                                ) {
                                    return Err(self.unsupported("raw pointer initializer"));
                                }
                            } else if self.trace.is_some_and(|trace| {
                                trace.descriptor == identity && trace.literal == value
                            }) {
                                self.expression(&binding.value)?;
                            } else if scalar(&binding.declared_type) {
                                self.expression(&binding.value)?;
                            } else {
                                return Err(self.unsupported("address-bearing local"));
                            }
                        }
                    }
                }
                ScalarBlockItem::Assignment(assignment) => {
                    if assignment.values.len() != assignment.targets.len() {
                        return Err(self.unsupported("multi-output assignment"));
                    }
                    for rhs in &assignment.values {
                        self.expression(rhs)?;
                    }
                    for (target, rhs) in assignment.targets.iter().zip(&assignment.values) {
                        let place = &target.place;
                        let frozen = view
                            .nodes
                            .frozen_install(view.facts, place)
                            .ok_or(self.missing("assignment install"))?;
                        let CfgPointKind::Assign {
                            target, rhs_point, ..
                        } = &view.cfg.points[frozen.install.0].kind
                        else {
                            return Err(self.unsupported("nonlexical assignment"));
                        };
                        if frozen.rhs != *rhs_point || view.eval(rhs) != Some(frozen.rhs) {
                            return Err(self.missing("frozen RHS"));
                        }
                        match place {
                            ScalarPlace::Index { .. } => self.index(place)?,
                            ScalarPlace::Name { .. } => {
                                match view
                                    .nodes
                                    .direct_destination(place)
                                    .ok_or(self.missing("direct destination"))?
                                {
                                    DirectDestinationIdentity::Existing(binding)
                                    | DirectDestinationIdentity::Introduce { binding, .. }
                                        if *binding == target.binding && binding != self.owner => {}
                                    _ => {
                                        return Err(
                                            self.unsupported("nonlocal or backing replacement")
                                        )
                                    }
                                }
                                if self.trace.is_some_and(|trace| {
                                    trace.raw_binding == &target.binding
                                        && trace.address == frozen.rhs
                                }) {
                                    continue;
                                }
                                let ty = view
                                    .nodes
                                    .checked_place_type(place)
                                    .ok_or(self.missing("destination type"))?;
                                if !scalar(ty) {
                                    return Err(self.unsupported("address store"));
                                }
                            }
                            _ => return Err(self.unsupported("projected write")),
                        }
                    }
                }
                ScalarBlockItem::Expression(expression) => self.expression(expression)?,
                ScalarBlockItem::While(_) => return Err(self.unsupported("loop")),
            }
        }
        Ok(())
    }

    fn index(&mut self, place: &ScalarPlace) -> Result<(), Unproved> {
        let ScalarPlace::Index { base, index, .. } = place else {
            return Err(self.unsupported("byte projection"));
        };
        let view = self.view;
        let root = view
            .nodes
            .named_place_root(base)
            .ok_or(self.missing("byte place root"))?;
        if !matches!(root, NamedPlaceRootIdentity::Lexical(binding) if binding == self.owner || view.cfg.allocations.contains(binding))
            || view.nodes.checked_place_type(place) != Some(&ScalarType::U8)
            || view.nodes.checked_expression_type(index) != Some(&ScalarType::U64)
        {
            return Err(self.unsupported("byte projection type or root"));
        }
        self.expression(index)
    }

    fn expression(&mut self, value: &ScalarExpression) -> Result<(), Unproved> {
        let view = self.view;
        match value {
            ScalarExpression::Name { .. } => {
                if self.trace.is_some_and(|trace| matches!(view.eval(value), Some(id) if id == trace.result_read || id == trace.field_read)) {
                    return Ok(());
                }
                let name = view
                    .nodes
                    .plain_name(value)
                    .ok_or(self.missing("plain name"))?;
                if !matches!(name, PlainNameIdentity::Lexical(binding) if binding != self.owner)
                    || !view
                        .nodes
                        .checked_expression_type(value)
                        .is_some_and(scalar)
                {
                    return Err(self.unsupported("address-bearing name"));
                }
            }
            ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. } => {}
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left)?;
                self.expression(right)?;
            }
            ScalarExpression::Unary { operand, .. } => self.expression(operand)?,
            ScalarExpression::Block(block) => self.block(block)?,
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition)?;
                self.block(then_branch)?;
                self.block(else_branch)?;
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition)?;
                self.block(then_branch)?;
            }
            ScalarExpression::IndexedRead { place, .. } => self.index(place)?,
            ScalarExpression::RawAddress { place, .. }
                if self
                    .trace
                    .is_some_and(|trace| view.eval(value) == Some(trace.address)) =>
            {
                self.index(place)?
            }
            ScalarExpression::StructLiteral { fields, .. }
                if self
                    .trace
                    .is_some_and(|trace| view.eval(value) == Some(trace.literal)) =>
            {
                for field in fields {
                    self.expression(&field.value)?;
                }
            }
            ScalarExpression::Call { arguments, .. } => self.call(value, arguments)?,
            _ => return Err(self.unsupported("untracked expression or address escape")),
        }
        Ok(())
    }

    fn call(
        &mut self,
        value: &ScalarExpression,
        arguments: &[ScalarExpression],
    ) -> Result<(), Unproved> {
        let view = self.view;
        let target = view
            .nodes
            .target(value)
            .map_err(|_| self.missing("call target"))?;
        let site = view.eval(value).ok_or(self.missing("call point"))?;
        let CfgPointKind::Call {
            target: event,
            output: 0,
            argument_points,
            ..
        } = &view.cfg.points[site.0].kind
        else {
            return Err(self.missing("call event"));
        };
        if event != target || argument_points.len() != arguments.len() {
            return Err(self.missing("call event identity"));
        }
        let mut actual = None;
        for (position, (argument, point)) in arguments.iter().zip(argument_points).enumerate() {
            if view.eval(argument) != Some(*point) {
                return Err(self.missing("actual evaluation"));
            }
            if view.lexical(argument) == Some(self.owner) {
                if actual.replace(position).is_some()
                    || !view
                        .nodes
                        .checked_expression_type(argument)
                        .is_some_and(bytes)
                {
                    return Err(self.unsupported("nonunique or untyped backing actual"));
                }
                if let Some(trace) = self.trace {
                    if !dominates(view.cfg, trace.allocation, *point) {
                        return Err(self.unsupported("backing definition at call"));
                    }
                }
            } else {
                self.expression(argument)?;
            }
        }
        match (target, actual) {
            (ResolvedCallTarget::Core(CoreOperationId::IntExtend), None)
                if arguments
                    .iter()
                    .all(|a| view.nodes.checked_expression_type(a).is_some_and(scalar)) =>
            {
                Ok(())
            }
            (
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete: ConcreteCallSelection::Declaration,
                },
                Some(parameter),
            ) => {
                let mut targets =
                    view.build
                        .prepared()
                        .project()
                        .modules
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| &m.source == source)
                        .flat_map(|(m, module)| {
                            module.items.iter().enumerate().filter_map(
                                move |(i, item)| match item {
                                    ScalarItem::Function(f)
                                        if f.name_span == *declaration_span
                                            && f.generic_parameters.is_empty()
                                            && f.overload_arms.is_empty() =>
                                    {
                                        Some((m, i))
                                    }
                                    _ => None,
                                },
                            )
                        });
                let (m, i) = targets
                    .next()
                    .ok_or(self.missing("direct concrete helper"))?;
                if targets.next().is_some() {
                    return Err(self.unsupported("ambiguous helper"));
                }
                self.edges.push(Edge {
                    from: view.body,
                    site,
                    to: (m, i, parameter),
                });
                Ok(())
            }
            _ => Err(Unproved::Call(
                view.body,
                site,
                target.clone(),
                arguments
                    .iter()
                    .map(|a| view.nodes.checked_expression_type(a).cloned())
                    .collect(),
            )),
        }
    }
}

fn dominates(cfg: &ScalarReferenceCfg, definition: CfgPointId, use_point: CfgPointId) -> bool {
    let mut seen = BTreeSet::new();
    let mut work = vec![CfgPointId(0)];
    let mut reached_definition = false;
    while let Some(id) = work.pop() {
        if id == definition {
            reached_definition = true;
            continue;
        }
        if id == use_point {
            return false;
        }
        if seen.insert(id) {
            let Some(point) = cfg.points.get(id.0) else {
                return false;
            };
            work.extend(&point.successors);
        }
    }
    reached_definition
}

#[cfg(test)]
mod tests;
