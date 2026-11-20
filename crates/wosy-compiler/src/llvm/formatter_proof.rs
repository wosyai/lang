#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use wosy_syntax::{ByteSpan, SourceIdentity};

use crate::scalar::control::{
    BindingDefinition, CallableBodyCallNodes, CallablePointFacts, DirectDestinationIdentity,
    NamedPlaceRootIdentity, PlainNameIdentity,
};
use crate::scalar::{
    BinaryOperator, CfgPointId, CfgPointKind, ConcreteCallSelection, CopyPolicy, CoreOperationId,
    ReferenceBindingId, ReferencePlaceId, ReferencePlaceProjection, ResolvedCallTarget,
    ScalarBindingOutputOrigin, ScalarReferenceCfg, ScalarStructFieldId, ScalarStructId,
    UnaryOperator,
};
use crate::{
    ScalarAssignment, ScalarBinding, ScalarBlock, ScalarBlockItem, ScalarBuildValidation,
    ScalarExpression, ScalarFunction, ScalarItem, ScalarPlace, ScalarStruct, ScalarType,
};

type Body = (usize, usize);
type Formal = (usize, usize, usize);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Dependency {
    Backing(Formal),
    ScalarOnly(Body),
}

#[derive(Debug)]
pub(super) struct Edge {
    pub(super) from: Body,
    pub(super) site: CfgPointId,
    pub(super) to: Dependency,
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
    Definition(Body, CfgPointId, IndexFailure),
    Call(
        Body,
        CfgPointId,
        ResolvedCallTarget,
        Vec<Option<ScalarType>>,
    ),
}

type SourceKey<'a> = (&'a str, &'a str, &'a str, &'a str);

fn source_key(source: &SourceIdentity) -> SourceKey<'_> {
    (
        &source.project,
        &source.package,
        &source.path,
        &source.revision,
    )
}

fn span_key(span: ByteSpan) -> (u32, u32) {
    (span.start, span.end)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IndexFailure {
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy)]
enum Indexed {
    Unique((usize, usize)),
    Ambiguous,
}

impl Indexed {
    fn lookup(entry: Option<&Self>) -> Result<(usize, usize), IndexFailure> {
        match entry {
            Some(Self::Unique(location)) => Ok(*location),
            Some(Self::Ambiguous) => Err(IndexFailure::Ambiguous),
            None => Err(IndexFailure::Missing),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Work {
    index_builds: usize,
    modules: usize,
    items: usize,
    structs: usize,
    definition_queries: usize,
    struct_queries: usize,
    helper_checks: usize,
    dominance_walks: usize,
    points: usize,
    arcs: usize,
    dominance_queries: usize,
    float_return_walks: usize,
    float_return_points: usize,
    float_return_arcs: usize,
}

pub(super) struct ProofBatch<'a> {
    build: &'a ScalarBuildValidation<'a>,
    definitions: BTreeMap<SourceKey<'a>, BTreeMap<(u32, u32), Indexed>>,
    structs: BTreeMap<SourceKey<'a>, BTreeMap<usize, Indexed>>,
    #[cfg(test)]
    work: std::cell::Cell<Work>,
}

impl<'a> ProofBatch<'a> {
    pub(super) fn new(build: &'a ScalarBuildValidation<'a>) -> Self {
        let mut batch = Self {
            build,
            definitions: BTreeMap::new(),
            structs: BTreeMap::new(),
            #[cfg(test)]
            work: std::cell::Cell::new(Work {
                index_builds: 1,
                ..Work::default()
            }),
        };
        for (m, module) in build.prepared().project().modules.iter().enumerate() {
            #[cfg(test)]
            batch.work.update(|mut work| {
                work.modules += 1;
                work
            });
            for (i, item) in module.items.iter().enumerate() {
                #[cfg(test)]
                batch.work.update(|mut work| {
                    work.items += 1;
                    work
                });
                if let ScalarItem::Function(function) = item {
                    if function.generic_parameters.is_empty() && function.overload_arms.is_empty() {
                        batch.insert_definition(&module.source, function.name_span, (m, i));
                    }
                }
            }
            for (i, structure) in module.structs.iter().enumerate() {
                #[cfg(test)]
                batch.work.update(|mut work| {
                    work.structs += 1;
                    work
                });
                batch.insert_struct(&structure.id, (m, i));
            }
        }
        batch
    }

    fn insert_definition(&mut self, source: &'a SourceIdentity, span: ByteSpan, body: Body) {
        self.definitions
            .entry(source_key(source))
            .or_default()
            .entry(span_key(span))
            .and_modify(|entry| *entry = Indexed::Ambiguous)
            .or_insert(Indexed::Unique(body));
    }

    fn insert_struct(&mut self, id: &'a ScalarStructId, location: (usize, usize)) {
        self.structs
            .entry(source_key(&id.source))
            .or_default()
            .entry(id.index)
            .and_modify(|entry| *entry = Indexed::Ambiguous)
            .or_insert(Indexed::Unique(location));
    }

    fn definition(&self, source: &SourceIdentity, span: ByteSpan) -> Result<Body, IndexFailure> {
        #[cfg(test)]
        self.work.update(|mut work| {
            work.definition_queries += 1;
            work
        });
        Indexed::lookup(
            self.definitions
                .get(&source_key(source))
                .and_then(|rows| rows.get(&span_key(span))),
        )
    }

    fn structure(&self, id: &ScalarStructId) -> Result<&'a ScalarStruct, IndexFailure> {
        #[cfg(test)]
        self.work.update(|mut work| {
            work.struct_queries += 1;
            work
        });
        let (m, i) = Indexed::lookup(
            self.structs
                .get(&source_key(&id.source))
                .and_then(|rows| rows.get(&id.index)),
        )?;
        Ok(&self.build.prepared().project().modules[m].structs[i])
    }

    fn dominates(&self, summary: &AllocationDominance, point: CfgPointId) -> bool {
        #[cfg(test)]
        self.work.update(|mut work| {
            work.dominance_queries += 1;
            work
        });
        summary.dominates(point)
    }

    pub(super) fn prove(&self, module: usize, item: usize) -> Result<Proof, Unproved> {
        prove_in_batch(self, module, item)
    }
}

struct View<'a> {
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

fn simple_producer_input(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::Bool
            | ScalarType::Char
            | ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
    )
}

fn scalar(ty: &ScalarType) -> bool {
    *ty == ScalarType::Unit || simple_producer_input(ty)
}

#[derive(Clone, Copy)]
enum Profile {
    Simple,
    WideSigned,
    WideUnsigned,
    Float32,
    Float64,
}

impl Profile {
    fn wide(self) -> Option<ScalarType> {
        match self {
            Self::Simple | Self::Float32 | Self::Float64 => None,
            Self::WideSigned => Some(ScalarType::I128),
            Self::WideUnsigned => Some(ScalarType::U128),
        }
    }

    fn float(self) -> bool {
        matches!(self, Self::Float32 | Self::Float64)
    }

    fn strict(self) -> bool {
        !matches!(self, Self::Simple)
    }

    fn scalar(self, ty: &ScalarType) -> bool {
        scalar(ty)
            || self.wide().as_ref() == Some(ty)
            || (self.float() && *ty == ScalarType::F64)
            || matches!((self, ty), (Self::Float32, ScalarType::F32))
    }

    fn integer(self, ty: &ScalarType) -> bool {
        self.scalar(ty)
            && !matches!(
                ty,
                ScalarType::Unit
                    | ScalarType::Bool
                    | ScalarType::Char
                    | ScalarType::F32
                    | ScalarType::F64
            )
    }

    fn local_scalar(self, ty: &ScalarType) -> bool {
        self.scalar(ty) && !(self.float() && *ty == ScalarType::F32)
    }

    fn tuple(self, backing: bool) -> Option<[ScalarType; 2]> {
        if self.float() {
            return Some([
                if backing {
                    ScalarType::U64
                } else {
                    ScalarType::F64
                },
                ScalarType::I32,
            ]);
        }
        (!backing)
            .then(|| self.wide().map(|width| [width, ScalarType::U8]))
            .flatten()
    }

    fn conversion(self, op: &CoreOperationId, from: &ScalarType, to: &ScalarType) -> bool {
        use ScalarType::*;

        if self.float() {
            return matches!(
                (self, op, from, to),
                (Self::Float32, CoreOperationId::FloatExtend, F32, F64)
                    | (_, CoreOperationId::FloatToSintTrunc, F64, I32)
                    | (_, CoreOperationId::SintToFloat, I32, F64)
                    | (_, CoreOperationId::IntExtend, I32, U64)
                    | (_, CoreOperationId::IntTrunc, I32, U8)
            );
        }
        simple_scalar_conversion(op, from, to)
            || (*op == CoreOperationId::IntExtend
                && *from == ScalarType::I32
                && self.wide().as_ref() == Some(to))
    }
}

fn simple_scalar_conversion(op: &CoreOperationId, from: &ScalarType, to: &ScalarType) -> bool {
    use ScalarType::*;

    // Only conversions observed in the bounded producer/helper component.
    // The operation and both types come from canonical prepared facts.
    matches!(
        (op, from, to),
        (CoreOperationId::Bitcast, Char, U32)
            | (CoreOperationId::IntExtend, I32, U64)
            | (CoreOperationId::IntExtend, I8, U16)
            | (
                CoreOperationId::IntTrunc,
                I16 | I32 | I64 | U16 | U32 | U64,
                U8
            )
    )
}

struct Trace<'a> {
    owner: &'a ReferenceBindingId,
    dominance: AllocationDominance,
    field: ScalarStructFieldId,
    address: CfgPointId,
    literal: CfgPointId,
    field_read: CfgPointId,
    result_read: CfgPointId,
    raw_binding: &'a ReferenceBindingId,
    descriptor: &'a ReferenceBindingId,
}

fn trace<'a>(
    view: &'a View<'_>,
    batch: &ProofBatch<'_>,
    structure: &ScalarStruct,
) -> Option<Trace<'a>> {
    let struct_id = &structure.id;
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
            && matches!(&binding.declared_type, ScalarType::Struct(id) if id == struct_id))
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
    let dominance = AllocationDominance::new(
        view.cfg,
        allocation,
        #[cfg(test)]
        &batch.work,
    )?;
    if !batch.dominates(&dominance, address) || !batch.dominates(&dominance, result_read) {
        return None;
    }
    Some(Trace {
        owner,
        dominance,
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
    ProofBatch::new(build).prove(module, item)
}

fn prove_in_batch(batch: &ProofBatch<'_>, module: usize, item: usize) -> Result<Proof, Unproved> {
    let build = batch.build;
    let body = (module, item);
    if !build.diagnostics().is_empty() {
        return Err(Unproved::Missing(body, "validated body"));
    }
    let view = View::new(build, body).ok_or(Unproved::Missing(body, "producer body"))?;
    let unsupported_trace = Unproved::Unsupported(body, "local allocation trace");
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &view.function.signature
    else {
        return Err(unsupported_trace);
    };
    let [output] = outputs.outputs.as_slice() else {
        return Err(unsupported_trace);
    };
    let ScalarType::Struct(id) = &output.ty else {
        return Err(unsupported_trace);
    };
    let profile = match parameters.as_slice() {
        [ScalarType::I128] => Profile::WideSigned,
        [ScalarType::U128] => Profile::WideUnsigned,
        [ScalarType::F32] => Profile::Float32,
        [ScalarType::F64] => Profile::Float64,
        [parameter] if simple_producer_input(parameter) => Profile::Simple,
        _ => return Err(unsupported_trace),
    };
    let structure = batch.structure(id).map_err(|failure| match failure {
        IndexFailure::Missing => Unproved::Missing(body, "result struct"),
        IndexFailure::Ambiguous => Unproved::Unsupported(body, "ambiguous result struct"),
    })?;
    let trace = trace(&view, batch, structure).ok_or(unsupported_trace)?;
    let mut edges = Vec::new();
    Uses {
        batch,
        view: &view,
        profile,
        owner: Some(trace.owner),
        trace: Some(&trace),
        edges: &mut edges,
    }
    .check()?;
    let mut visited = BTreeSet::new();
    let mut cursor = 0;
    while let Some(edge) = edges.get(cursor) {
        let dependency = edge.to;
        cursor += 1;
        if !visited.insert(dependency) {
            continue;
        }
        #[cfg(test)]
        batch.work.update(|mut work| {
            work.helper_checks += 1;
            work
        });
        let (body, parameter) = match dependency {
            Dependency::Backing((m, i, parameter)) => ((m, i), Some(parameter)),
            Dependency::ScalarOnly(body) => (body, None),
        };
        let helper = View::new(build, body).ok_or(Unproved::Missing(body, "helper body"))?;
        let owner = if let Some(parameter) = parameter {
            let (owner, ty) = helper
                .formal(parameter)
                .ok_or(Unproved::Missing(body, "helper formal"))?;
            if !bytes(ty) {
                return Err(Unproved::Unsupported(body, "helper signature"));
            }
            Some(owner)
        } else {
            None
        };
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &helper.function.signature
        else {
            return Err(Unproved::Unsupported(body, "helper signature"));
        };
        if parameters
            .iter()
            .enumerate()
            .any(|(i, ty)| Some(i) != parameter && !profile.scalar(ty))
            || !helper_outputs(profile, &outputs.outputs, owner.is_none())
        {
            return Err(Unproved::Unsupported(body, "helper signature"));
        }
        let mut uses = Uses {
            batch,
            view: &helper,
            profile,
            owner,
            trace: None,
            edges: &mut edges,
        };
        uses.check()?;
        if outputs.outputs.len() == 2 {
            uses.tuple_return(
                profile
                    .tuple(owner.is_some())
                    .ok_or(Unproved::Unsupported(body, "helper tuple schema"))?,
            )?;
        } else if profile.float() && owner.is_some() {
            uses.float_carry_return()?;
        }
    }
    Ok(Proof {
        owner: trace.owner.clone(),
        field: trace.field,
        output: 0,
        edges,
    })
}

struct Uses<'a, 'b> {
    batch: &'a ProofBatch<'b>,
    view: &'a View<'b>,
    profile: Profile,
    owner: Option<&'a ReferenceBindingId>,
    trace: Option<&'a Trace<'a>>,
    edges: &'a mut Vec<Edge>,
}

fn helper_outputs(profile: Profile, outputs: &[crate::ScalarOutput], scalar_only: bool) -> bool {
    if profile.float() {
        return match outputs {
            [output] => !scalar_only && output.ty == ScalarType::I32,
            [first, second] => profile
                .tuple(!scalar_only)
                .is_some_and(|types| first.ty == types[0] && second.ty == types[1]),
            _ => false,
        };
    }
    match outputs {
        [] => !scalar_only,
        [output] => profile.scalar(&output.ty),
        [first, second] if scalar_only => {
            profile.wide().as_ref() == Some(&first.ty) && second.ty == ScalarType::U8
        }
        _ => false,
    }
}

impl<'a, 'b> Uses<'a, 'b> {
    fn unsupported(&self, message: &'static str) -> Unproved {
        Unproved::Unsupported(self.view.body, message)
    }
    fn missing(&self, message: &'static str) -> Unproved {
        Unproved::Missing(self.view.body, message)
    }

    fn tuple_call(
        &mut self,
        call: &ScalarExpression,
    ) -> Result<(CfgPointId, [ScalarType; 2]), Unproved> {
        let ScalarExpression::Call { arguments, .. } = call else {
            return Err(self.unsupported("tuple producer"));
        };
        let (point, outputs) = self.call(call, arguments, true)?;
        Ok((point, outputs.ok_or(self.missing("tuple producer schema"))?))
    }

    fn successor(&self, from: CfgPointId, to: CfgPointId) -> bool {
        from != to
            && self
                .view
                .cfg
                .points
                .get(from.0)
                .is_some_and(|point| point.successors == [to])
    }

    fn tuple_projection(&self, first: CfgPointId, second: CfgPointId) -> Result<(), Unproved> {
        #[cfg(test)]
        if tests::is_withheld(tests::Withheld::TupleProjection(self.view.body, second)) {
            return Err(self.missing("tuple projection"));
        }
        #[cfg(test)]
        let second = if tests::is_withheld(tests::Withheld::ReorderedTupleProjection(
            self.view.body,
            second,
        )) {
            first
        } else {
            second
        };
        let cfg = self.view.cfg;
        let (Some(primary), Some(projection)) = (cfg.points.get(first.0), cfg.points.get(second.0))
        else {
            return Err(self.missing("tuple projection"));
        };
        if !matches!(
            (&primary.kind, &projection.kind),
            (CfgPointKind::Call { output: 0, target: target0, argument_points: args0, .. },
             CfgPointKind::Call { output: 1, target: target1, argument_points: args1, .. })
                if target0 == target1 && args0 == args1
        ) || first == second
            || primary.scope != projection.scope
            || primary.source_span != projection.source_span
        {
            return Err(self.unsupported("tuple projection identity"));
        }
        Ok(())
    }

    fn tuple_binding(&mut self, binding: &ScalarBinding) -> Result<(), Unproved> {
        let [first, second] = binding.receivers.as_slice() else {
            return Err(self.unsupported("tuple binding arity"));
        };
        if binding.output_origin != ScalarBindingOutputOrigin::SingleExpression
            || first.allocation_length.is_some()
            || second.allocation_length.is_some()
        {
            return Err(self.unsupported("tuple binding slots"));
        }
        let (call, tuple) = self.tuple_call(&binding.value)?;
        if first.ty != tuple[0] || second.ty != tuple[1] {
            return Err(self.unsupported("tuple binding slots"));
        }
        let view = self.view;
        let mut defs = Vec::new();
        for (slot, receiver) in binding.receivers.iter().enumerate() {
            let Some(BindingDefinition::Initialized { bind, value }) =
                view.nodes.binding_definition(view.facts, binding, slot)
            else {
                return Err(self.missing("tuple receiver definition"));
            };
            let id = view
                .declaration(bind)
                .ok_or(self.missing("tuple receiver bind"))?;
            let definition = &view.cfg.points[bind.0];
            if id.declaration_span != receiver.name_span
                || Some(id) == self.owner
                || !self.successor(value, bind)
                || definition.scope != view.cfg.points[call.0].scope
            {
                return Err(self.unsupported("tuple receiver identity"));
            }
            defs.push((bind, value, id));
        }
        let [(bind0, value0, id0), (_, value1, id1)] = defs.as_slice() else {
            return Err(self.missing("tuple receiver definition"));
        };
        if *value0 != call || id0 == id1 || !self.successor(*bind0, *value1) {
            return Err(self.unsupported("tuple receiver order"));
        }
        self.tuple_projection(call, *value1)
    }

    fn tuple_assignment(&mut self, assignment: &ScalarAssignment) -> Result<(), Unproved> {
        let [call] = assignment.values.as_slice() else {
            return Err(self.unsupported("tuple assignment arity"));
        };
        let (call, tuple) = self.tuple_call(call)?;
        let view = self.view;
        let mut installs = Vec::new();
        for (slot, target) in assignment.targets.iter().enumerate() {
            let place = &target.place;
            let frozen = view
                .nodes
                .frozen_install(view.facts, place)
                .ok_or(self.missing("tuple frozen install"))?;
            let Some(point) = view.cfg.points.get(frozen.install.0) else {
                return Err(self.missing("tuple install point"));
            };
            let CfgPointKind::Assign {
                target, rhs_point, ..
            } = &point.kind
            else {
                return Err(self.unsupported("tuple nonlexical install"));
            };
            let Some(DirectDestinationIdentity::Existing(id)) =
                view.nodes.direct_destination(place)
            else {
                return Err(self.unsupported("tuple direct destination"));
            };
            if !matches!(place, ScalarPlace::Name { .. })
                || view.nodes.checked_place_type(place) != Some(&tuple[slot])
                || !target.projections.is_empty()
                || &target.binding != id
                || Some(id) == self.owner
                || *rhs_point != frozen.rhs
                || point.scope != view.cfg.points[call.0].scope
            {
                return Err(self.unsupported("tuple install identity"));
            }
            installs.push((frozen, id));
        }
        let [(first, id0), (second, id1)] = installs.as_slice() else {
            return Err(self.missing("tuple frozen install"));
        };
        self.tuple_projection(call, second.rhs)?;
        let next = view.cfg.points[second.rhs.0].successors.as_slice();
        let [after_projection] = next else {
            return Err(self.unsupported("tuple install order"));
        };
        let staged = *after_projection == first.install
            || view
                .cfg
                .points
                .get(after_projection.0)
                .is_some_and(|capture| {
                    matches!(&capture.kind, CfgPointKind::CaptureAssignmentPrevious { writes }
                    if writes == &[first.install, second.install])
                        && self.successor(*after_projection, first.install)
                });
        if first.rhs != call
            || id0 == id1
            || !self.successor(call, second.rhs)
            || !staged
            || !self.successor(first.install, second.install)
        {
            return Err(self.unsupported("tuple install order"));
        }
        Ok(())
    }

    fn tuple_return(&self, tuple: [ScalarType; 2]) -> Result<(), Unproved> {
        let view = self.view;
        let mut expressions = view.function.body.final_expressions();
        let Some((value @ ScalarExpression::If { .. }, _)) = expressions.next() else {
            return Err(self.unsupported("tuple return shape"));
        };
        if expressions.next().is_some() || view.nodes.final_output(value) != Some(0) {
            return Err(self.unsupported("tuple return shape"));
        }
        let mut returns = view.cfg.points.iter().filter_map(|point| match point.kind {
            CfgPointKind::ReturnOutput {
                output,
                value_point,
                checked,
            } => Some((point, output, value_point, checked)),
            _ => None,
        });
        let [Some((ret0, 0, join0, false)), Some((ret1, 1, join1, false)), None] =
            [returns.next(), returns.next(), returns.next()]
        else {
            return Err(self.unsupported("tuple return order"));
        };
        for (returned, join) in [(ret0, join0), (ret1, join1)] {
            let Some(join_point) = view.cfg.points.get(join.0) else {
                return Err(self.missing("tuple join point"));
            };
            if returned.scope != join_point.scope || returned.source_span != join_point.source_span
            {
                return Err(self.unsupported("tuple arm/return slot identity"));
            }
        }
        if !self.successor(join1, ret0.id) || !self.successor(ret0.id, ret1.id) {
            return Err(self.unsupported("tuple return order"));
        }
        self.tuple_conditional(value, [join0, join1], &tuple, &ret0.source_span.source)
    }

    fn float_carry_return(&self) -> Result<(), Unproved> {
        let view = self.view;
        let owner = self.owner.ok_or(self.missing("float carry owner"))?;
        let ScalarType::Callable { outputs, .. } = &view.function.signature else {
            return Err(self.missing("float carry signature"));
        };
        if !matches!(outputs.outputs.as_slice(), [output] if output.ty == ScalarType::I32) {
            return Err(self.unsupported("float carry signature"));
        }
        let mut expressions = view.function.body.final_expressions();
        let (value, slot) = expressions
            .next()
            .ok_or(self.missing("float carry output"))?;
        if expressions.next().is_some()
            || slot.position != 0
            || view.nodes.final_output(value) != Some(0)
        {
            return Err(self.unsupported("float carry output shape"));
        }
        if self.wide_primitive_type(value)? != &ScalarType::I32 {
            return Err(self.unsupported("float carry result type"));
        }
        let root = view
            .eval(value)
            .ok_or(self.missing("float carry evaluation"))?;
        let root_point = view
            .cfg
            .points
            .get(root.0)
            .ok_or(self.missing("float carry evaluation"))?;
        let mut returns = view
            .cfg
            .points
            .iter()
            .filter(|point| matches!(point.kind, CfgPointKind::ReturnOutput { .. }));
        let returned = returns.next().ok_or(self.missing("float carry return"))?;
        let CfgPointKind::ReturnOutput {
            output,
            value_point,
            checked,
        } = returned.kind
        else {
            return Err(self.unsupported("float carry return identity"));
        };
        #[cfg(test)]
        let value_point = if tests::is_withheld(tests::Withheld::FloatCarryReturnValue(
            view.body,
            returned.id,
        )) {
            returned.id
        } else {
            value_point
        };
        if returns.next().is_some()
            || output != 0
            || value_point != root
            || checked
            || root_point.scope.0 != 0
            || returned.scope != root_point.scope
            || root_point.source_span != returned.source_span
            || root_point.source_span.source != owner.source
            || root_point.source_span.range != slot.span
            || !self.successor(root, returned.id)
        {
            return Err(self.unsupported("float carry return identity"));
        }
        let mut required = vec![root, returned.id];
        if let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } = value
        {
            if !matches!(root_point.kind, CfgPointKind::Join)
                || root_point.source_span.range != *span
            {
                return Err(self.unsupported("float carry join"));
            }
            let condition_id = view
                .eval(condition)
                .ok_or(self.missing("float carry condition"))?;
            let condition_point = view
                .cfg
                .points
                .get(condition_id.0)
                .ok_or(self.missing("float carry condition"))?;
            let [branch_id] = condition_point.successors.as_slice() else {
                return Err(self.unsupported("float carry branch identity"));
            };
            let branch = view
                .cfg
                .points
                .get(branch_id.0)
                .ok_or(self.missing("float carry branch"))?;
            let [then_entry, else_entry] = branch.successors.as_slice() else {
                return Err(self.unsupported("float carry branch identity"));
            };
            let [Some(then_entry), Some(else_entry)] =
                [then_entry, else_entry].map(|id| view.cfg.points.get(id.0))
            else {
                return Err(self.missing("float carry arm entry"));
            };
            if !matches!(&branch.kind, CfgPointKind::Branch { condition: actual } if actual == condition.as_ref())
                || self.wide_primitive_type(condition)? != &ScalarType::Bool
                || branch.source_span != condition_point.source_span
                || branch.source_span.source != owner.source
                || branch.scope != root_point.scope
                || condition_point.scope != root_point.scope
                || !self.successor(condition_id, *branch_id)
                || then_entry.scope == else_entry.scope
                || [then_entry.scope, else_entry.scope].contains(&root_point.scope)
            {
                return Err(self.unsupported("float carry branch identity"));
            }
            required.extend([condition_id, *branch_id, then_entry.id, else_entry.id]);
            for (block, entry) in [(then_branch, then_entry), (else_branch, else_entry)] {
                let mut values = block.final_expressions();
                let (arm, slot) = values
                    .next()
                    .ok_or(self.missing("float carry arm result"))?;
                if values.next().is_some()
                    || slot.position != 0
                    || !matches!(entry.kind, CfgPointKind::ScopeEnter)
                    || entry.source_span.source != owner.source
                    || entry.source_span.range != block.span
                {
                    return Err(self.unsupported("float carry arm identity"));
                }
                if self.wide_primitive_type(arm)? != &ScalarType::I32 {
                    return Err(self.unsupported("float carry result type"));
                }
                let arm_id = view
                    .eval(arm)
                    .ok_or(self.missing("float carry arm result"))?;
                let arm_point = view
                    .cfg
                    .points
                    .get(arm_id.0)
                    .ok_or(self.missing("float carry arm result"))?;
                let [exit] = arm_point.successors.as_slice() else {
                    return Err(self.unsupported("float carry arm identity"));
                };
                let exit_point = view
                    .cfg
                    .points
                    .get(exit.0)
                    .ok_or(self.missing("float carry arm exit"))?;
                if arm_point.scope != entry.scope
                    || exit_point.scope != entry.scope
                    || !matches!(exit_point.kind, CfgPointKind::ScopeExit)
                    || exit_point.source_span != entry.source_span
                    || arm_point.source_span.source != owner.source
                    || arm_point.source_span.range != slot.span
                    || !self.successor(*exit, root)
                {
                    return Err(self.unsupported("float carry arm identity"));
                }
                required.extend([arm_id, *exit]);
            }
        }
        if !matches!(
            view.cfg.points.first().map(|point| &point.kind),
            Some(CfgPointKind::Entry)
        ) {
            return Err(self.missing("float carry entry"));
        }
        // One independent entry reachability pass per scalar carry that passed the
        // local identity checks. It is not an allocation dominance query.
        #[cfg(test)]
        let counters = std::cell::Cell::new(Work::default());
        let reachable = walk(
            view.cfg,
            None,
            #[cfg(test)]
            &counters,
        )
        .ok_or(self.missing("float carry reachability"))?;
        #[cfg(test)]
        self.batch.work.update(|mut work| {
            let counts = counters.get();
            work.float_return_walks += counts.dominance_walks;
            work.float_return_points += counts.points;
            work.float_return_arcs += counts.arcs;
            work
        });
        if required
            .into_iter()
            .any(|id| reachable.get(id.0) != Some(&true))
        {
            return Err(self.unsupported("float carry unreachable return"));
        }
        Ok(())
    }

    fn tuple_conditional(
        &self,
        value: &ScalarExpression,
        joins: [CfgPointId; 2],
        tuple: &[ScalarType; 2],
        source: &SourceIdentity,
    ) -> Result<(), Unproved> {
        let ScalarExpression::If {
            then_branch,
            else_branch,
            span,
            ..
        } = value
        else {
            return Err(self.unsupported("tuple conditional shape"));
        };
        let view = self.view;
        let mut then_points = joins;
        let mut else_points = joins;
        let [Some(first), Some(second)] = joins.map(|id| view.cfg.points.get(id.0)) else {
            return Err(self.missing("tuple join point"));
        };
        if first.scope != second.scope || !self.successor(joins[0], joins[1]) {
            return Err(self.unsupported("tuple conditional order"));
        }
        for (slot, point) in [first, second].into_iter().enumerate() {
            #[cfg(test)]
            if tests::is_withheld(tests::Withheld::TupleArm(view.body, point.id)) {
                return Err(self.missing("tuple arm output"));
            }
            let CfgPointKind::ConditionalOutput {
                then_point,
                else_point,
            } = point.kind
            else {
                return Err(self.unsupported("tuple conditional output"));
            };
            if then_point == else_point
                || &point.source_span.source != source
                || point.source_span.range != *span
            {
                return Err(self.unsupported("tuple arm/return slot identity"));
            }
            then_points[slot] = then_point;
            else_points[slot] = else_point;
        }
        #[cfg(test)]
        if tests::is_withheld(tests::Withheld::SwappedTupleArmSlots(view.body, joins[0])) {
            else_points.swap(0, 1);
        }
        self.tuple_arm(then_branch, then_points, tuple, source)?;
        self.tuple_arm(else_branch, else_points, tuple, source)
    }

    fn tuple_arm(
        &self,
        block: &ScalarBlock,
        points: [CfgPointId; 2],
        tuple: &[ScalarType; 2],
        source: &SourceIdentity,
    ) -> Result<(), Unproved> {
        let mut values = block.final_expressions();
        let first = values.next();
        let second = values.next();
        if self.profile.float() {
            if let (Some((value @ ScalarExpression::If { .. }, metadata)), None) = (first, second) {
                if metadata.position != 0 {
                    return Err(self.unsupported("tuple arm/return slot identity"));
                }
                return self.tuple_conditional(value, points, tuple, source);
            }
        }
        let [Some(first), Some(second), None] = [first, second, values.next()] else {
            return Err(self.missing("tuple arm slot"));
        };
        let view = self.view;
        let [Some(point0), Some(point1)] = points.map(|id| view.cfg.points.get(id.0)) else {
            return Err(self.missing("tuple arm output"));
        };
        if points[0] == points[1] || point0.scope != point1.scope {
            return Err(self.unsupported("tuple arm/return slot identity"));
        }
        for (slot, ((value, metadata), point)) in [first, second]
            .into_iter()
            .zip([point0, point1])
            .enumerate()
        {
            if metadata.position != slot
                || view.eval(value) != Some(point.id)
                || view.nodes.checked_expression_type(value) != Some(&tuple[slot])
                || &point.source_span.source != source
            {
                return Err(self.unsupported("tuple arm/return slot identity"));
            }
        }
        Ok(())
    }

    fn checked_call_signature(
        &self,
        callee: Body,
        arguments: &[ScalarExpression],
        backing: Option<usize>,
        tuple: bool,
        value: &ScalarExpression,
    ) -> Result<Option<[ScalarType; 2]>, Unproved> {
        let view = self.view;
        let module = &self.batch.build.prepared().project().modules[callee.0];
        let ScalarItem::Function(function) = &module.items[callee.1] else {
            return Err(self.missing("wide concrete helper"));
        };
        let ScalarType::Callable {
            outputs,
            parameters,
        } = &function.signature
        else {
            return Err(self.missing("wide helper signature"));
        };
        if parameters.len() != arguments.len() {
            return Err(self.unsupported("wide helper arity"));
        }
        if self.profile.float() {
            use ScalarType::*;
            let exact = match (backing, tuple, parameters.as_slice()) {
                (None, true, [F64, I32, U64]) => true,
                (Some(0), true, [buffer, U64, U64, U64, F64, I32]) => bytes(buffer),
                (Some(0), false, [buffer, U64, U64, I32, Bool]) => bytes(buffer),
                _ => false,
            };
            if !exact {
                return Err(self.unsupported("float helper signature"));
            }
        }
        for (slot, (argument, formal)) in arguments.iter().zip(parameters).enumerate() {
            let argument = view
                .nodes
                .checked_expression_type(argument)
                .ok_or(self.missing("wide argument type"))?;
            if (Some(slot) == backing && (!bytes(argument) || !bytes(formal)))
                || (Some(slot) != backing && (argument != formal || !self.profile.scalar(formal)))
            {
                return Err(self.unsupported("wide argument/formal type"));
            }
        }
        let Some(first) = outputs.outputs.first() else {
            return Err(self.unsupported("wide helper output arity"));
        };
        if outputs.outputs.len() != if tuple { 2 } else { 1 }
            || !helper_outputs(self.profile, &outputs.outputs, backing.is_none())
            || view.nodes.checked_expression_type(value) != Some(&first.ty)
        {
            return Err(self.unsupported("wide helper output type"));
        }
        Ok(if tuple {
            self.profile.tuple(backing.is_some())
        } else {
            None
        })
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
                    if binding.receivers.len() == 2 && self.profile.strict() {
                        self.tuple_binding(binding)?;
                        continue;
                    }
                    let definition = view
                        .nodes
                        .binding_definition(view.facts, binding, 0)
                        .ok_or(self.missing("binding definition"))?;
                    if binding.receivers.len() != 1 {
                        return Err(self.unsupported("multi-output binding"));
                    }
                    match definition {
                        BindingDefinition::Allocation { .. } => {
                            if self.owner.is_none() {
                                return Err(self.unsupported("scalar-only allocation"));
                            }
                            if self.profile.float() && self.trace.is_none() {
                                return Err(self.unsupported("float backing helper allocation"));
                            }
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
                            } else if self.profile.local_scalar(&binding.declared_type) {
                                self.expression(&binding.value)?;
                                if self.profile.strict() {
                                    if view.eval(&binding.value) != Some(value) {
                                        return Err(self.missing("wide scalar binding value"));
                                    }
                                    if self.wide_primitive_type(&binding.value)?
                                        != &binding.declared_type
                                    {
                                        return Err(self.unsupported("wide primitive binding type"));
                                    }
                                }
                            } else {
                                return Err(self.unsupported("address-bearing local"));
                            }
                        }
                    }
                }
                ScalarBlockItem::Assignment(assignment) => {
                    if matches!(
                        assignment.values.as_slice(),
                        [ScalarExpression::Call { .. }]
                    ) && assignment.targets.len() == 2
                        && self.profile.wide().is_some()
                    {
                        self.tuple_assignment(assignment)?;
                        continue;
                    }
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
                        #[cfg(test)]
                        let target = if tests::is_withheld(tests::Withheld::FloatIndexedBinding(
                            view.body,
                            frozen.install,
                        )) {
                            let mut faulty = target.clone();
                            faulty.binding =
                                view.formal(3).expect("carry exponent formal").0.clone();
                            std::borrow::Cow::Owned(faulty)
                        } else if tests::is_withheld(tests::Withheld::FloatIndexedProjection(
                            view.body,
                            frozen.install,
                        )) {
                            let mut faulty = target.clone();
                            faulty.projections.push(faulty.projections[0].clone());
                            std::borrow::Cow::Owned(faulty)
                        } else {
                            std::borrow::Cow::Borrowed(target)
                        };
                        #[cfg(test)]
                        let target = target.as_ref();
                        match place {
                            ScalarPlace::Index { .. } => {
                                self.index(place)?;
                                if self.profile.float() {
                                    self.float_indexed_install(place, target, frozen.install)?;
                                }
                                self.wide_assignment_type(place, rhs)?;
                            }
                            ScalarPlace::Name { .. } => {
                                match view
                                    .nodes
                                    .direct_destination(place)
                                    .ok_or(self.missing("direct destination"))?
                                {
                                    DirectDestinationIdentity::Existing(binding)
                                    | DirectDestinationIdentity::Introduce { binding, .. }
                                        if *binding == target.binding
                                            && Some(binding) != self.owner => {}
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
                                if !self.profile.local_scalar(ty) {
                                    return Err(self.unsupported("address store"));
                                }
                                self.wide_assignment_type(place, rhs)?;
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

    fn wide_assignment_type(
        &self,
        place: &ScalarPlace,
        rhs: &ScalarExpression,
    ) -> Result<(), Unproved> {
        if self.profile.strict() {
            let destination = self
                .view
                .nodes
                .checked_place_type(place)
                .ok_or(self.missing("wide assignment destination type"))?;
            if self.wide_primitive_type(rhs)? != destination {
                return Err(self.unsupported("wide primitive assignment type"));
            }
        }
        Ok(())
    }

    fn float_indexed_install(
        &self,
        place: &ScalarPlace,
        target: &ReferencePlaceId,
        install: CfgPointId,
    ) -> Result<(), Unproved> {
        let owner = self
            .owner
            .ok_or(self.missing("float indexed install owner"))?;
        let ScalarPlace::Index { base, index, .. } = place else {
            return Err(self.unsupported("float indexed install identity"));
        };
        let view = self.view;
        let install = view
            .cfg
            .points
            .get(install.0)
            .ok_or(self.missing("float indexed install"))?;
        if !matches!(view.nodes.named_place_root(base), Some(NamedPlaceRootIdentity::Lexical(id)) if id == owner)
            || &target.binding != owner
            || target.source != owner.source
            || target.function_span != owner.function_span
            || target.declaration_span != owner.declaration_span
            || target.block_span != owner.block_span
            || install.source_span.source != owner.source
            || !matches!(target.projections.as_slice(), [ReferencePlaceProjection::Index(actual)] if actual == index.as_ref())
            || &target.place != place
        {
            return Err(self.unsupported("float indexed install identity"));
        }
        Ok(())
    }

    fn index(&mut self, place: &ScalarPlace) -> Result<(), Unproved> {
        let owner = self
            .owner
            .ok_or(self.unsupported("scalar-only byte projection"))?;
        let ScalarPlace::Index { base, index, .. } = place else {
            return Err(self.unsupported("byte projection"));
        };
        let view = self.view;
        let root = view
            .nodes
            .named_place_root(base)
            .ok_or(self.missing("byte place root"))?;
        if !matches!(root, NamedPlaceRootIdentity::Lexical(binding) if binding == owner || (!self.profile.float() && view.cfg.allocations.contains(binding)))
            || view.nodes.checked_place_type(place) != Some(&ScalarType::U8)
            || view.nodes.checked_expression_type(index) != Some(&ScalarType::U64)
        {
            return Err(self.unsupported("byte projection type or root"));
        }
        self.expression(index)
    }

    fn expression(&mut self, value: &ScalarExpression) -> Result<(), Unproved> {
        let view = self.view;
        let wide = self.profile.strict();
        match value {
            ScalarExpression::Name { .. } => {
                if self.trace.is_some_and(|trace| matches!(view.eval(value), Some(id) if id == trace.result_read || id == trace.field_read)) {
                    return Ok(());
                }
                let name = view
                    .nodes
                    .plain_name(value)
                    .ok_or(self.missing("plain name"))?;
                if !matches!(name, PlainNameIdentity::Lexical(binding) if Some(binding) != self.owner)
                    || !view
                        .nodes
                        .checked_expression_type(value)
                        .is_some_and(|ty| self.profile.scalar(ty))
                {
                    return Err(self.unsupported("address-bearing name"));
                }
            }
            ScalarExpression::Integer { .. } => {
                if wide && !self.profile.integer(self.wide_primitive_type(value)?) {
                    return Err(self.unsupported("wide integer expression type"));
                }
            }
            ScalarExpression::Boolean { .. } => {
                if wide && self.wide_primitive_type(value)? != &ScalarType::Bool {
                    return Err(self.unsupported("wide boolean expression type"));
                }
            }
            ScalarExpression::Float { .. } if self.profile.float() => {
                if self.wide_primitive_type(value)? != &ScalarType::F64 {
                    return Err(self.unsupported("float literal type"));
                }
            }
            ScalarExpression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                self.expression(left)?;
                self.expression(right)?;
                if wide
                    && !self.wide_binary_types(
                        operator,
                        self.wide_primitive_type(left)?,
                        self.wide_primitive_type(right)?,
                        self.wide_primitive_type(value)?,
                    )
                {
                    return Err(self.unsupported("wide binary expression types"));
                }
            }
            ScalarExpression::Unary {
                operator, operand, ..
            } => {
                if wide {
                    let result = self.wide_primitive_type(value)?;
                    if self.wide_folded_minimum_shape(operand, operator) {
                        if *result != ScalarType::I128
                            || !self.wide_folded_minimum_identity(value, operand)
                        {
                            return Err(self.unsupported("wide folded minimum identity"));
                        }
                    } else {
                        self.expression(operand)?;
                        let operand = self.wide_primitive_type(operand)?;
                        if result != operand
                            || !match operator {
                                UnaryOperator::LogicalNot => *operand == ScalarType::Bool,
                                UnaryOperator::BitwiseNot => self.profile.integer(operand),
                                UnaryOperator::Negate => {
                                    self.profile.integer(operand)
                                        && matches!(
                                            operand,
                                            ScalarType::I8
                                                | ScalarType::I16
                                                | ScalarType::I32
                                                | ScalarType::I64
                                                | ScalarType::I128
                                        )
                                }
                            }
                        {
                            return Err(self.unsupported("wide unary expression types"));
                        }
                    }
                } else {
                    self.expression(operand)?;
                }
            }
            ScalarExpression::Block(block) => self.block(block)?,
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition)?;
                self.float_condition(condition)?;
                self.block(then_branch)?;
                self.block(else_branch)?;
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition)?;
                self.float_condition(condition)?;
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
            ScalarExpression::Call { arguments, .. } => {
                self.call(value, arguments, false)?;
            }
            _ => return Err(self.unsupported("untracked expression or address escape")),
        }
        Ok(())
    }

    fn float_condition(&self, condition: &ScalarExpression) -> Result<(), Unproved> {
        if self.profile.float() && self.wide_primitive_type(condition)? != &ScalarType::Bool {
            return Err(self.unsupported("float branch condition type"));
        }
        Ok(())
    }

    fn wide_checked_type(&self, value: &ScalarExpression) -> Option<&'b ScalarType> {
        let view = self.view;
        #[cfg(test)]
        if view.eval(value).is_some_and(|point| {
            tests::is_withheld(tests::Withheld::ContradictorySignedMagnitude(
                view.body, point,
            ))
        }) {
            return Some(&ScalarType::I128);
        }
        view.nodes.checked_expression_type(value)
    }

    fn wide_primitive_type(&self, value: &ScalarExpression) -> Result<&'b ScalarType, Unproved> {
        let view = self.view;
        let _point = view
            .eval(value)
            .ok_or(self.missing("wide primitive evaluation"))?;
        let checked = self.wide_checked_type(value);
        #[cfg(test)]
        let checked = if tests::is_withheld(tests::Withheld::PrimitiveFact(view.body, _point)) {
            None
        } else if tests::is_withheld(tests::Withheld::ContradictoryPrimitiveFact(
            view.body, _point,
        )) {
            checked.map(|_| &ScalarType::U16)
        } else {
            checked
        };
        checked.ok_or(self.missing("wide primitive expression type"))
    }

    fn wide_binary_types(
        &self,
        operator: &BinaryOperator,
        left: &ScalarType,
        right: &ScalarType,
        result: &ScalarType,
    ) -> bool {
        use BinaryOperator::*;

        if left != right {
            return false;
        }
        if self.profile.float() && *left == ScalarType::F64 {
            return match operator {
                Subtract | Multiply | Divide => *result == ScalarType::F64,
                Equal | Less | GreaterEqual => *result == ScalarType::Bool,
                _ => false,
            };
        }
        let integer = self.profile.integer(left);
        match operator {
            And | Or => *left == ScalarType::Bool && *result == ScalarType::Bool,
            Equal | NotEqual => {
                (integer || matches!(left, ScalarType::Bool | ScalarType::Char))
                    && *result == ScalarType::Bool
            }
            Less | LessEqual | Greater | GreaterEqual => integer && *result == ScalarType::Bool,
            Add | Subtract | Multiply | Divide | Remainder | BitAnd | BitOr | BitXor
            | ShiftLeft | ShiftRight => integer && left == result,
        }
    }

    fn wide_folded_minimum_shape(
        &self,
        operand: &ScalarExpression,
        operator: &UnaryOperator,
    ) -> bool {
        matches!(self.profile, Profile::WideSigned)
            && *operator == UnaryOperator::Negate
            && matches!(operand, ScalarExpression::Integer { value, .. }
                if value == &i128::MIN.unsigned_abs().into())
    }

    fn wide_folded_minimum_identity(
        &self,
        unary: &ScalarExpression,
        operand: &ScalarExpression,
    ) -> bool {
        let view = self.view;
        // This exact signed minimum is ranged as one expression by the typechecker.
        // A positive 2^127 is not an i128. Accept the canonical folded occurrence
        // in its one parent only, never as an independently typed operand.
        if self.wide_checked_type(operand).is_some() {
            return false;
        }
        let (Some(child), Some(parent)) = (view.eval(operand), view.eval(unary)) else {
            return false;
        };
        let (Some(child_point), Some(parent_point)) =
            (view.cfg.points.get(child.0), view.cfg.points.get(parent.0))
        else {
            return false;
        };
        self.successor(child, parent)
            && child_point.scope == parent_point.scope
            && child_point.source_span.source == parent_point.source_span.source
            && matches!(
                child_point.kind,
                CfgPointKind::Evaluate(ScalarExpression::Integer { .. })
            )
            && matches!(
                parent_point.kind,
                CfgPointKind::Evaluate(ScalarExpression::Unary { .. })
            )
    }

    fn call(
        &mut self,
        value: &ScalarExpression,
        arguments: &[ScalarExpression],
        tuple: bool,
    ) -> Result<(CfgPointId, Option<[ScalarType; 2]>), Unproved> {
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
            if self
                .owner
                .is_some_and(|owner| view.lexical(argument) == Some(owner))
            {
                if actual.replace(position).is_some()
                    || !view
                        .nodes
                        .checked_expression_type(argument)
                        .is_some_and(bytes)
                {
                    return Err(self.unsupported("nonunique or untyped backing actual"));
                }
                if let Some(trace) = self.trace {
                    if !self.batch.dominates(&trace.dominance, *point) {
                        return Err(self.unsupported("backing definition at call"));
                    }
                }
            } else {
                self.expression(argument)?;
            }
        }
        if let (
            ResolvedCallTarget::Core(
                op @ (CoreOperationId::IntExtend
                | CoreOperationId::IntTrunc
                | CoreOperationId::Bitcast
                | CoreOperationId::FloatExtend
                | CoreOperationId::FloatToSintTrunc
                | CoreOperationId::SintToFloat),
            ),
            None,
            [argument],
        ) = (target, actual, arguments)
        {
            let from = view.nodes.checked_expression_type(argument);
            let to = view.nodes.checked_expression_type(value);
            #[cfg(test)]
            let from = from.filter(|_| {
                !tests::is_withheld(tests::Withheld::ConversionOperand(view.body, site))
            });
            #[cfg(test)]
            let to = to.filter(|_| {
                !tests::is_withheld(tests::Withheld::ConversionResult(view.body, site))
            });
            let from = from.ok_or(self.missing("conversion operand type"))?;
            let to = to.ok_or(self.missing("conversion result type"))?;
            if !tuple && self.profile.conversion(op, from, to) {
                return Ok((site, None));
            }
        }
        match (target, actual) {
            (
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete: ConcreteCallSelection::Declaration,
                },
                backing,
            ) if backing.is_some() || self.profile.strict() => {
                let (m, i) = self
                    .batch
                    .definition(source, *declaration_span)
                    .map_err(|failure| Unproved::Definition(view.body, site, failure))?;
                let outputs = if self.profile.strict() {
                    self.checked_call_signature((m, i), arguments, backing, tuple, value)?
                } else {
                    None
                };
                self.edges.push(Edge {
                    from: view.body,
                    site,
                    to: match backing {
                        Some(parameter) => Dependency::Backing((m, i, parameter)),
                        None => Dependency::ScalarOnly((m, i)),
                    },
                });
                Ok((site, outputs))
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

struct AllocationDominance {
    allocation: CfgPointId,
    reachable: Vec<bool>,
    bypass: Vec<bool>,
}

impl AllocationDominance {
    fn new(
        cfg: &ScalarReferenceCfg,
        allocation: CfgPointId,
        #[cfg(test)] work: &std::cell::Cell<Work>,
    ) -> Option<Self> {
        let reachable = walk(
            cfg,
            None,
            #[cfg(test)]
            work,
        )?;
        let bypass = walk(
            cfg,
            Some(allocation),
            #[cfg(test)]
            work,
        )?;
        Some(Self {
            allocation,
            reachable,
            bypass,
        })
    }

    fn dominates(&self, use_point: CfgPointId) -> bool {
        self.reachable.get(self.allocation.0) == Some(&true)
            && use_point != self.allocation
            && self.reachable.get(use_point.0) == Some(&true)
            && self.bypass.get(use_point.0) == Some(&false)
    }
}

fn walk(
    cfg: &ScalarReferenceCfg,
    cut: Option<CfgPointId>,
    #[cfg(test)] counters: &std::cell::Cell<Work>,
) -> Option<Vec<bool>> {
    #[cfg(test)]
    counters.update(|mut work| {
        work.dominance_walks += 1;
        work
    });
    let mut reached = vec![false; cfg.points.len()];
    let entry = CfgPointId(0);
    if cut == Some(entry) {
        return Some(reached);
    }
    *reached.first_mut()? = true;
    let mut pending = vec![entry];
    while let Some(id) = pending.pop() {
        #[cfg(test)]
        counters.update(|mut work| {
            work.points += 1;
            work
        });
        for &next in &cfg.points[id.0].successors {
            #[cfg(test)]
            counters.update(|mut work| {
                work.arcs += 1;
                work
            });
            let visited = reached.get_mut(next.0)?;
            if !*visited && cut != Some(next) {
                *visited = true;
                pending.push(next);
            }
        }
    }
    Some(reached)
}

#[cfg(test)]
mod tests;
