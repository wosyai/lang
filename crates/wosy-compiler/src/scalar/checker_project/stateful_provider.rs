//! Execute an authenticated scalar call on the current observed row. There
//! is no speculative state, ambient entry or external effects in this lane.
use super::*;
use super::ordered_float_prefix::{self,ExactValue,ObservedStep,ObservedTrace};
use super::super::checker_flow::{ProjectedStorage,apply_observed_integer_operator,charge_call_work,observed_f32_array};
use super::super::reference::{BlockOrdinal,OccurrenceEdge,OccurrenceNode,OccurrenceParent,PlainNameIdentity};
use super::super::control::AssignmentReplacement;
use std::collections::BTreeMap;
use wosy_syntax::SourceSpan;

#[derive(Clone)]
struct Place {
    declaration: SourceSpan,
    selectors: Vec<(OccurrenceOrdinal,u64)>,
    ty: ScalarType,
    field: Option<super::super::ScalarStructFieldId>,
}

impl Place {
    fn indexed(body:&FunctionCallSites,site:OccurrenceOrdinal,ty:&ScalarType,owner:&SourceIdentity) -> Option<Self> {
        use super::super::reference::NamedPlaceRootIdentity;
        let st = &body.structure;
        if st.checked_types.places.get(&site)? != ty { return None; }
        if let Some(field) = st.projected_place_fields.get(&site).filter(|_| matches!(ty,ScalarType::I32|ScalarType::U8)) {
            let OccurrenceNode::Place(base) = *st.edges.get(&(OccurrenceParent::Place(site),OccurrenceEdge::PlaceBase))? else { return None; };
            let base_ty = st.checked_types.places.get(&base)?;
            if *base_ty != ScalarType::Struct(field.structure.clone()) { return None; }
            let (declaration, selectors) = if let Some(NamedPlaceRootIdentity::ModuleValue(declaration)) = body.prepared_named_place_root(base) {
                (declaration, Vec::new())
            } else {
                let checked=observed_f32_array::typed_provider_place(body,base,base_ty)?;
                (checked.declaration,checked.selectors)
            };
            return (&declaration.source==owner).then(|| Self { declaration:declaration.clone(), selectors, ty:ty.clone(),field:Some(field.clone()) });
        }
        let checked=observed_f32_array::typed_provider_place(body,site,ty)?;
        if !matches!(ty,ScalarType::I32|ScalarType::F32|ScalarType::U16|ScalarType::U8) || &checked.declaration.source!=owner { return None; }
        Some(Self { declaration:checked.declaration.clone(),selectors:checked.selectors,ty:ty.clone(),field:None })
    }

    fn cell(&self, indices:Vec<ExactValue>) -> Option<observed_f32_array::Cell> {
        if indices.len()!=self.selectors.len() { return None; }
        let indices=indices.into_iter().zip(self.selectors.iter().rev()).map(|(value,(_,length))| {
            let ExactValue::Integer(ScalarType::U64,value)=value else { return None; };
            let value=u64::try_from(&value).ok()?;
            (value<*length).then_some(value)
        }).collect::<Option<Box<[_]>>>()?;
        Some(observed_f32_array::Cell { declaration:self.declaration.clone(),indices })
    }
}

#[derive(Clone,Copy)]
enum Output { Unit, Scalar }

struct Activation {
    target: PreparedCallable,
    slots: HashMap<ReferenceBindingId,ExactValue>,
    rank: Option<usize>,
    value_base: usize,
    entered_at: Option<SourceSpan>,
}

enum Task {
    Block(BlockOrdinal,usize,Output),
    Eval(OccurrenceOrdinal),
    DropUnit,
    DropInteger(ScalarType),
    Bind(ReferenceBindingId,ScalarType),
    ReplaceLocal(ReferenceBindingId,ScalarType),
    Read(Place),
    WritePlace(Place),
    Write(Place,ExactValue),
    Binary(BinaryOperator),
    Not,
    Negate(ScalarType),
    Requirement(OccurrenceOrdinal),
    Branch(OccurrenceOrdinal,BlockOrdinal,Option<BlockOrdinal>,Output),
    Short(OccurrenceOrdinal,bool,OccurrenceOrdinal),
    Call(OccurrenceOrdinal,usize),
    Finish(Option<SourceSpan>,ScalarType),
}

// Unit is only an ordering token. Scalar proof values remain finite/exact.
enum ResultValue { Unit, Scalar(ExactValue) }

fn scalar(value:ResultValue) -> Option<ExactValue> {
    let ResultValue::Scalar(value)=value else { return None; };
    Some(value)
}

fn ordinary_binary(op:&BinaryOperator,l:ExactValue,r:ExactValue) -> Option<ExactValue> {
    use super::super::symbolic_integer::IntegerRelation;
    use ExactValue::*;
    match (l,r) {
        (Integer(t,l),Integer(u,r)) if t==u => {
            if let Some(relation)=IntegerRelation::from_operator(op) { return Some(Boolean(relation.accepts(l.cmp(&r)))); }
            Some(Integer(t.clone(),apply_observed_integer_operator(&t,op,l,r)?))
        }
        (Enum(l,a),Enum(r,b)) if l==r => match op {
            BinaryOperator::Equal=>Some(Boolean(a==b)),
            BinaryOperator::NotEqual=>Some(Boolean(a!=b)),
            _=>None,
        },
        (F32(l),F32(r)) => {
            if let Some(relation)=IntegerRelation::from_operator(op) {
                return Some(Boolean(relation.accepts(f32::from_bits(l.0).partial_cmp(&f32::from_bits(r.0))?)));
            }
            Some(F32(l.binary(op,r)?))
        }
        _ => None,
    }
}

fn direct_symbol<'a>(project:&'a ScalarProject,calls:&'a ProjectCallSiteResults,target:PreparedCallable)
    -> Option<(&'a ScalarFunction,&'a FunctionCallSites,ScalarType)> {
    let ScalarItem::Function(function)=project.modules.get(target.0)?.items.get(target.1)? else { return None; };
    let body=calls.function_body(target.0,target.1,None)?;
    if !function.generic_parameters.is_empty() || !body.arms.is_empty() || !body.callable_escapes.is_empty()
        || body.contextual_formal().is_some() || !body.allocation_length_calls.is_empty()
        || (has_callable_obligations(body) && !supports_observed_requirements(body,false))
    { return None; }
    let [return_slot]=body.boundary.results.as_slice() else { return None; };
    Some((function,body,return_slot.ty.clone()))
}

fn rank_argument(body:&FunctionCallSites, call:OccurrenceOrdinal, slot:usize, formal:&ReferenceBindingId) -> Option<()> {
    let OccurrenceNode::Expression(arg)=body.structure.edges.get(&(
        OccurrenceParent::Expression(call),OccurrenceEdge::CallArgument(slot)))? else { return None; };
    let (left,BinaryOperator::Subtract,right)=body.structure.prepared_range_binary.get(arg)? else { return None; };
    if body.structure.checked_types.expressions.get(arg)?!=&ScalarType::I32
        || !matches!(body.prepared_plain_name(*left),Some(PlainNameIdentity::Lexical(id)) if id==formal)
        || body.structure.prepared_range_literals.get(right)?!=&BigInt::from(1)
    { return None; }
    Some(())
}

fn rank_value(value:&ExactValue) -> Option<i32> {
    let ExactValue::Integer(ScalarType::I32,value)=value else { return None; };
    i32::try_from(value).ok()
}

fn requirement_condition(body:&FunctionCallSites,at:OccurrenceOrdinal) -> Option<(OccurrenceOrdinal,bool)> {
    let st=&body.structure;
    let directive=st.checker_directives.get(&at)?;
    if directive.operation!=CheckerOperation::Require || !directive.output.state_files.is_empty()
        || directive.output.has_module_values || !matches!(directive.output.code_value,CheckerCode::Literal(_)) { return None; }
    let OccurrenceNode::Expression(condition)=st.edges.get(&(OccurrenceParent::Expression(at),OccurrenceEdge::CallArgument(0)))?
        else { return None; };
    if st.checked_types.expressions.get(condition)?!=&ScalarType::Bool
        || body.occurrences.get(condition.0)?.span!=directive.condition { return None; }
    let mut indexed=false;
    let mut pending=vec![*condition];
    while let Some(at)=pending.pop() {
        let ty=st.checked_types.expressions.get(&at)?;
        if let Some(cut)=body.short_circuits.get(&at) {
            if *ty!=ScalarType::Bool { return None; }
            pending.extend([cut.right,cut.left]);
        } else if let Some((left,_,right))=st.prepared_range_binary.get(&at) {
            if !matches!(ty,ScalarType::Bool|ScalarType::I32) { return None; }
            pending.extend([*right,*left]);
        } else if let Some(leaf)=st.observed_bool_negations.get(&at) {
            if *ty!=ScalarType::Bool { return None; }
            pending.push(*leaf);
        } else if st.prepared_range_literals.contains_key(&at) || st.observed_bool_literals.contains_key(&at)
            || matches!(body.prepared_plain_name(at),Some(PlainNameIdentity::Lexical(_) | PlainNameIdentity::ModuleValue(_))) {
            if !matches!(ty,ScalarType::Bool|ScalarType::I32) { return None; }
        } else if let Some(OccurrenceNode::Place(place))=st.edges.get(&(OccurrenceParent::Expression(at),OccurrenceEdge::ExpressionPlace)) {
            if *ty!=ScalarType::I32 { return None; }
            observed_f32_array::typed_place(body,*place,&ScalarType::I32)?;
            indexed=true;
        } else { return None; }
    }
    Some((*condition,indexed))
}

fn supports_observed_requirements(body:&FunctionCallSites,indexed:bool) -> bool {
    body.checker_returns_unit() && body.arms.is_empty() && !body.structure.checker_directives.is_empty()
        && body.structure.checker_directives.keys().all(|&site|
            requirement_condition(body,site).is_some_and(|(_,projects)| !indexed || projects))
}

pub(super) fn supports_requirements(body:&FunctionCallSites) -> bool {
    supports_observed_requirements(body,true)
}

#[cfg(test)]
thread_local! {
    static REQUIREMENT_RESULTS: RefCell<Vec<(SourceSpan,bool)>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn take_requirement_results() -> Vec<(SourceSpan,bool)> {
    REQUIREMENT_RESULTS.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

pub(super) struct SelectedRequirement {
    pub(super) target:PreparedCallable,
    pub(super) site:OccurrenceOrdinal,
    pub(super) history:ObservedTrace,
}

#[derive(Clone,Copy)]
enum RangeBound { Formal(usize), Literal(i128) }

#[derive(Clone,Copy)]
pub(super) struct RangeRank { begin:usize, finish:RangeBound, inclusive:bool }

/// Locate the unique terminal self-edge through prepared Unit branches.
/// Only an enclosing admitted comparison can authorize the advancing axis.
pub(super) fn advancing_range_plan(body:&FunctionCallSites,target:PreparedCallable) -> Option<RangeRank> {
    let st=&body.structure;
    let ids=body.boundary.checker_parameter_bindings.as_deref()?;
    if ids.len()!=body.boundary.parameters.len()
        || (has_callable_obligations(body) && !supports_requirements(body)) { return None; }
    let mut sites=st.checker_direct_targets.iter().filter_map(|(&site,&child)| (child==target).then_some(site));
    let backedge=sites.next()?;
    if sites.next().is_some() { return None; }
    let formal=|at| match body.prepared_plain_name(at)? {
        PlainNameIdentity::Lexical(id)=>ids.iter().position(|p| p==id),
        _=>None,
    };
    let argument=|slot| match st.edges.get(&(OccurrenceParent::Expression(backedge),OccurrenceEdge::CallArgument(slot)))? {
        OccurrenceNode::Expression(at)=>Some(*at), _=>None,
    };
    let (begin,old)= (0..ids.len()).find_map(|slot| {
        let arg=argument(slot)?;
        let (old,BinaryOperator::Add,one)=st.prepared_range_binary.get(&arg)? else { return None; };
        let ty=&body.boundary.parameters.get(slot)?.ty;
        (matches!(ty,ScalarType::I32|ScalarType::U64)
            && formal(*old)==Some(slot) && st.prepared_range_literals.get(one)?==&BigInt::from(1)
            && st.checked_types.expressions.get(&arg)?==ty).then_some((slot,*old))
    })?;
    let bound=|mut condition| {
        // A positive conjunction proves its left predicate before any right
        // effects. The path must actually select its true branch below.
        while let Some(cut)=body.short_circuits.get(&condition) {
            if !cut.evaluate_right_when || cut.bypass { return None; }
            condition=cut.left;
        }
        let (left,operation,right)=st.prepared_range_binary.get(&condition)?;
        let inclusive=match operation { BinaryOperator::Less=>false,BinaryOperator::LessEqual=>true,_=>return None };
        if formal(*left)!=formal(old) || st.checked_types.expressions.get(right)?!=&body.boundary.parameters.get(begin)?.ty { return None; }
        let finish=if let Some(slot)=formal(*right) {
            (slot!=begin && formal(argument(slot)?)==Some(slot)).then_some(RangeBound::Formal(slot))?
        } else {
            RangeBound::Literal(i128::try_from(st.prepared_range_literals.get(right)?).ok()?)
        };
        Some((finish,inclusive))
    };
    if body.checker_returns_unit() {
        let mut pending=vec![(st.root,None)];
        while let Some((block,limit))=pending.pop() {
            let Some(item)=st.blocks.get(block.0)?.items.last() else { continue; };
            let BlockItemKind::Evaluation {expression}=item.kind else { continue; };
            if expression==backedge { let (finish,inclusive)=limit?; return Some(RangeRank {begin,finish,inclusive}); }
            if st.checked_types.expressions.get(&expression)!=Some(&ScalarType::Unit) { continue; }
            match body.conditionals.get(&expression) {
                Some(ConditionalRegion::Unit {condition,then_block,..})=>
                    pending.push((*then_block,bound(*condition).or(limit))),
                Some(ConditionalRegion::Value {condition,then_block,else_block,..})=>{
                    pending.push((*else_block,limit));
                    pending.push((*then_block,bound(*condition).or(limit)));
                }
                None=>{}
            }
        }
        return None;
    }
    // A value-returning recursive call may be consumed by a local
    // assignment, then return that value. The VM retains the exact
    // continuation; termination needs the enclosing true guard, not tail
    // position. Use the same typed occurrence parents as range proofs.
    let parents = super::range_envelopes::occurrence_parents(body)?;
    let mut cursor=OccurrenceParent::Expression(backedge);
    while cursor!=OccurrenceParent::Block(st.root) {
        let &(parent,edge)=parents.get(&cursor)?;
        if let (OccurrenceParent::Expression(at),OccurrenceEdge::ThenBranch)=(parent,edge) {
            let condition=match body.conditionals.get(&at)? {
                ConditionalRegion::Unit {condition,..} | ConditionalRegion::Value {condition,..}=>*condition,
            };
            if let Some((finish,inclusive))=bound(condition) { return Some(RangeRank {begin,finish,inclusive}); }
        }
        cursor=parent;
    }
    None
}

impl RangeRank {
    fn decreases(self, slots:&HashMap<ReferenceBindingId,ExactValue>,ids:&[ReferenceBindingId],args:&[ExactValue]) -> Option<()> {
        let mathematical=|value:&ExactValue| {
            let ExactValue::Integer(ty,value)=value else { return None; };
            match ty {
                ScalarType::I32=>Some(i128::from(i32::try_from(value).ok()?)),
                ScalarType::U64=>Some(i128::from(u64::try_from(value).ok()?)),
                _=>None,
            }
        };
        let begin=mathematical(slots.get(ids.get(self.begin)?)?)?;
        let next=mathematical(args.get(self.begin)?)?;
        let (finish,next_finish)=match self.finish {
            RangeBound::Formal(slot)=>(mathematical(slots.get(ids.get(slot)?)?)?,mathematical(args.get(slot)?)?),
            RangeBound::Literal(value)=>(value,value),
        };
        // For either typed axis the enclosing true guard bounds a
        // mathematical distance; checked source arithmetic already gave
        // the exact non-wrapping next argument.
        let adjustment=i128::from(self.inclusive);
        let distance=finish-begin+adjustment;
        let successor=next_finish-next+adjustment;
        (distance>0 && successor==distance-1 && finish==next_finish && begin+1==next).then_some(())
    }
}

/// The common Unit cursor keeps its established lane for ordinary scopes.
/// A mixed, directly prepared Unit with a nested U16 store needs this
/// state-returning runner. The store itself is checked again on execution.
pub(super) fn needs_unit_runner(body:&FunctionCallSites) -> bool {
    if !body.checker_returns_unit() || has_callable_obligations(body) { return false; }
    let parameters=&body.boundary.parameters;
    parameters.iter().any(|p| p.ty==ScalarType::I32)
        && parameters.iter().any(|p| p.ty==ScalarType::F32)
        && parameters.iter().all(|p| matches!(p.ty,ScalarType::I32|ScalarType::F32))
        && body.assignments.values().any(|group| group.destinations().any(|destination|
            matches!(destination.replacement,AssignmentReplacement::Resolved(ScalarType::U16))
                && observed_f32_array::typed_place(body,destination.destination,&ScalarType::U16).is_some()))
}

/// Restrict new recursive entries to a prepared chain with completed-owner
/// storage: a U16 provider handoff or an authenticated F32 backing write.
/// A syntactic edge is not an execution fact; the cursor checks each call.
pub(super) fn qualified_store_reachable(project:&ScalarProject,calls:&ProjectCallSiteResults,start:PreparedCallable,
    visits:&[InitializationVisit], completed_local_f32:bool) -> bool {
    let mut seen=HashSet::new();
    let mut pending=vec![start];
    while let Some(target)=pending.pop() {
        if !seen.insert(target) { continue; }
        let Some(body)=calls.function_body(target.0,target.1,None) else { continue; };
        if completed_local_f32 && target.0==start.0 && body.assignments.values().any(|group|
            group.destinations().any(|destination|
                matches!(destination.replacement,AssignmentReplacement::Resolved(ScalarType::F32))
                    && observed_f32_array::typed_place(body,destination.destination,&ScalarType::F32).is_some())) {
            return true;
        }
        for (&site,&child) in &body.structure.checker_direct_targets {
            if child==target { continue; }
            let Some(signature)=body.checked_call_signatures.get(&site) else { continue; };
            if !matches!(signature.outputs.outputs.as_slice(),[out] if out.ty==ScalarType::Unit)
                || authenticate_prepared_direct_edge(project,calls,target.0,body,site,signature)!=Some(child) {
                continue;
            }
            if child.0==start.0 {
                pending.push(child);
                continue;
            }
            let Some(from)=project.modules.get(target.0) else { continue; };
            let Some(to)=project.modules.get(child.0) else { continue; };
            if completed_namespace_owner(&from.source,&to.source,child.0,Some(visits))
                && calls.function_body(child.0,child.1,None).is_some_and(needs_unit_runner) { return true; }
        }
    }
    false
}

/// A reached prepared expression, not an executed foreign operation.
/// The pure operand prefix is known on this row; foreign effects and
/// results are not modeled and cannot produce a continuation row.
#[derive(Debug, PartialEq)]
pub(super) struct PendingExternalCall {
    pub(super) call: SourceSpan,
    pub(super) declaration: SourceSpan,
    pub(super) signature: CheckedCallSignature,
    pub(super) invocation: Vec<SourceSpan>,
    pub(super) argument_sites: Vec<SourceSpan>,
    pub(super) operand_prefix: Vec<ForeignOperand>,
    // Actual requirements proved on this single observed prefix. These are
    // not entry summaries and say nothing about post-call state.
    pub(super) verified_prefix: VerifiedRequirePrefix,
}

#[derive(Debug, Default, Eq, PartialEq, Hash)]
pub(super) struct VerifiedRequirePrefix {
    owners: BTreeMap<usize, SourceIdentity>,
    // Repeated evaluations are distinct events; do not deduplicate them.
    events: Vec<(usize, ByteSpan)>,
}

impl VerifiedRequirePrefix {
    fn seal(project: &ScalarProject, events: Vec<(usize, ByteSpan)>) -> Option<Self> {
        let mut owners = BTreeMap::new();
        for &(owner, _) in &events {
            if let std::collections::btree_map::Entry::Vacant(slot) = owners.entry(owner) {
                slot.insert(project.modules.get(owner)?.source.clone());
            }
        }
        Some(Self { owners, events })
    }

    #[cfg(test)]
    pub(super) fn spans(&self) -> impl Iterator<Item = SourceSpan> + '_ {
        self.events.iter().map(|(owner, range)| SourceSpan::new(self.owners[owner].clone(), *range))
    }
}

/// A proved value before the host call. An address is only a symbolic
/// allocation and element; it has no numeric address or memory contents.
#[derive(Debug, PartialEq)]
pub(super) enum ForeignOperand {
    Integer(ScalarType, BigInt),
    ArrayAddress(observed_f32_array::Cell),
}

// This projection is deliberately pure: unlike the ordinary interpreter it
// cannot execute calls, change storage, or bypass an earlier unknown operand.
fn foreign_integer(
    body: &FunctionCallSites, root: OccurrenceOrdinal, frame: &Activation,
    storage: &ProjectedStorage, owner: &SourceIdentity,
) -> Option<ExactValue> {
    enum Step { Read(OccurrenceOrdinal), Binary(BinaryOperator, ScalarType), Negate(ScalarType) }
    let st = &body.structure;
    let mut pending = vec![Step::Read(root)];
    let mut values = Vec::new();
    while let Some(step) = pending.pop() {
        match step {
            Step::Read(at) => {
                let ty = st.checked_types.expressions.get(&at)?;
                if st.checker_contextual_occurrences.contains(&at) { return None; }
                if let Some(literal) = st.prepared_range_literals.get(&at) {
                    if !integer_literal_fits_type(literal, ty) { return None; }
                    values.push(ExactValue::Integer(ty.clone(), literal.clone()));
                } else if let Some(name) = body.prepared_plain_name(at) {
                    let value = match name {
                        PlainNameIdentity::Lexical(id) => frame.slots.get(id)?.clone(),
                        PlainNameIdentity::ModuleValue(decl) if &decl.source == owner => storage.observed_atom(decl)?,
                        _ => return None,
                    };
                    if !matches!(&value, ExactValue::Integer(t,_) if t == ty) { return None; }
                    values.push(value);
                } else if let Some((l,op,r)) = st.prepared_range_binary.get(&at).or_else(|| st.prepared_integer_bitwise.get(&at)) {
                    pending.extend([Step::Binary(op.clone(),ty.clone()), Step::Read(*r), Step::Read(*l)]);
                } else if let Some(child) = st.observed_integer_negations.get(&at) {
                    pending.extend([Step::Negate(ty.clone()), Step::Read(*child)]);
                } else { return None; }
            }
            Step::Binary(op, ty) => {
                let right=values.pop()?;
                let result=ordinary_binary(&op,values.pop()?,right)?;
                if !matches!(&result,ExactValue::Integer(t,_) if *t==ty) { return None; }
                values.push(result);
            }
            Step::Negate(ty) => {
                let ExactValue::Integer(t, value)=values.pop()? else {return None;};
                let value= -value;
                if t!=ty || !integer_literal_fits_type(&value,&ty) {return None;}
                values.push(ExactValue::Integer(ty,value));
            }
        }
    }
    values.pop().filter(|_|values.is_empty())
}

fn foreign_operand(
    body: &FunctionCallSites, at: OccurrenceOrdinal, frame:&Activation,
    storage:&ProjectedStorage, owner:&SourceIdentity,
) -> Option<ForeignOperand> {
    let st=&body.structure;
    let Some(ResolvedCallTarget::Core(super::super::CoreOperationId::PointerCast))=body.resolved_call_site(at).map(|site|&site.target) else {
        let ExactValue::Integer(ty,value)=foreign_integer(body,at,frame,storage,owner)? else {return None;};
        return Some(ForeignOperand::Integer(ty,value));
    };
    let cast=body.checked_call_signatures.get(&at)?;
    if cast.overload.is_some() || cast.overload_origin.is_some() {return None;}
    let [ScalarType::RawPointer(element)]=cast.parameters.as_slice() else {return None;};
    let [out]=cast.outputs.outputs.as_slice() else {return None;};
    if !matches!(&out.ty,ScalarType::RawPointer(_)) || st.checked_types.expressions.get(&at)?!=&out.ty {return None;}
    let OccurrenceNode::Expression(arg)=*st.edges.get(&(OccurrenceParent::Expression(at),OccurrenceEdge::CallArgument(0)))? else {return None;};
    if !st.checker_raw_addresses.contains(&arg) || st.checked_types.expressions.get(&arg)?!=&cast.parameters[0] {return None;}
    let OccurrenceNode::Place(place)=*st.edges.get(&(OccurrenceParent::Expression(arg),OccurrenceEdge::ExpressionPlace))? else {return None;};
    let place=Place::indexed(body,place,element,owner)?;
    if place.field.is_some() || !storage.has_observed_aggregate(&place.declaration) {return None;}
    let indices=place.selectors.iter().rev().map(|(index,_)|foreign_integer(body,*index,frame,storage,owner)).collect::<Option<Vec<_>>>()?;
    Some(ForeignOperand::ArrayAddress(place.cell(indices)?))
}

fn pending_external_call(
    body: &FunctionCallSites, owner: &SourceIdentity, block: BlockOrdinal, frame: &Activation, storage: &ProjectedStorage,
) -> Option<PendingExternalCall> {
    let region = body.structure.blocks.get(block.0)?;
    let [item] = region.items.as_slice() else { return None; };
    if !region.unsafe_context { return None; }
    let BlockItemKind::Evaluation { expression } = item.kind else { return None; };
    let ResolvedCallTarget::ExternCallable { source, declaration_span } =
        &body.resolved_call_site(expression)?.target else { return None; };
    let signature = body.checked_call_signatures.get(&expression)?;
    if signature.overload.is_some() || signature.overload_origin.is_some() { return None; }
    let mut operand_prefix = Vec::new();
    let argument_sites = signature.parameters.iter().enumerate().map(|(slot, ty)| {
        let OccurrenceNode::Expression(at) = body.structure.edges.get(&(
            OccurrenceParent::Expression(expression), OccurrenceEdge::CallArgument(slot),
        ))? else { return None; };
        if !super::super::scalar_type_equal(body.structure.checked_types.expressions.get(at)?, ty) {
            return None;
        }
        if operand_prefix.len()==slot {
            if let Some(value)=foreign_operand(body,*at,frame,storage,owner) {operand_prefix.push(value);}
        }
        Some(SourceSpan::new(owner.clone(), body.occurrences.get(at.0)?.span))
    }).collect::<Option<Vec<_>>>()?;
    Some(PendingExternalCall {
        call: SourceSpan::new(owner.clone(), body.occurrences.get(expression.0)?.span),
        declaration: SourceSpan::new(source.clone(), *declaration_span),
        signature: signature.clone(),
        invocation: Vec::new(),
        argument_sites, operand_prefix,
        verified_prefix: VerifiedRequirePrefix::default(),
    })
}

/// Outcome of this concrete activation prefix only. A false require is
/// already decided even when ordered whole-root selection is not. It
/// never supplies a return value, a writeback, or an admitted diagnostic.
#[derive(Debug, PartialEq)]
pub(super) enum ProviderStop {
    External(PendingExternalCall),
    RefutedRow {
        directive: SourceSpan,
        invocation: Vec<SourceSpan>,
        verified_prefix: VerifiedRequirePrefix,
    },
}

#[derive(Default)]
pub(super) struct Dispatch<'a> {
    stop: Option<ProviderStop>,
    declarations:HashMap<PreparedCallable,(&'a ScalarFunction,&'a FunctionCallSites,ScalarType)>,
    pure_plans:HashMap<PreparedCallable,Option<(ordered_float_prefix::PrefixCertificate,ordered_float_prefix::FunctionEffects)>>,
    latest_pure:HashMap<PreparedCallable,(Vec<u32>,ExactValue,Option<Rc<ObservedTrace>>)>,
    range_ranks:HashMap<PreparedCallable,Option<RangeRank>>,
    requirement_conditions:HashMap<(PreparedCallable,OccurrenceOrdinal),Option<OccurrenceOrdinal>>,
}

impl Dispatch<'_> {
    pub(super) fn take_stop(&mut self) -> Option<ProviderStop> {
        self.stop.take()
    }
}

/// This is capability selection only. The actual call still needs a current,
/// compiler-owned row and every executed location must be authenticated.
pub(super) fn has_nominal_integer_store(body:&FunctionCallSites,owner:&SourceIdentity)->bool {
    body.assignments.values().any(|group|group.destinations().any(|destination|
        matches!(&destination.replacement,AssignmentReplacement::Resolved(ty)
            if matches!(ty,ScalarType::I32|ScalarType::U8)
                && Place::indexed(body,destination.destination,ty,owner).is_some_and(|place|place.field.is_some()))))
}

/// Find the prepared direct-call paths that can execute a nominal store in
/// this owner. This selects an evaluator, never a proof root or a reason to
/// assume an edge is taken. Reverse closure scans each local edge once so
/// every wider call in the observed startup can share the same decision.
pub(super) fn local_nominal_store_routes(
    project:&ScalarProject,calls:&ProjectCallSiteResults,owner:usize,
) -> Option<HashSet<PreparedCallable>> {
    let module=project.modules.get(owner)?;
    let mut incoming=HashMap::<_,Vec<_>>::new();
    let mut covered=HashSet::new();
    let mut pending=Vec::new();
    for (item,declaration) in module.items.iter().enumerate() {
        if !matches!(declaration,ScalarItem::Function(_)) { continue; }
        let Some(body)=calls.function_body(owner,item,None) else { continue; };
        let target=(owner,item);
        if has_nominal_integer_store(body,&module.source) {
            covered.insert(target);
            pending.push(target);
        }
        for (&site,&child) in &body.structure.checker_direct_targets {
            if child.0!=owner { continue; }
            let Some(signature)=body.checked_call_signatures.get(&site) else { continue; };
            if authenticate_prepared_direct_edge(project,calls,owner,body,site,signature)==Some(child) {
                incoming.entry(child).or_default().push(target);
            }
        }
    }
    while let Some(target)=pending.pop() {
        for parent in incoming.remove(&target).unwrap_or_default() {
            if covered.insert(parent) { pending.push(parent); }
        }
    }
    Some(covered)
}

pub(super) fn run<'a>(
    project:&'a ScalarProject,calls:&'a ProjectCallSiteResults,target:PreparedCallable,args:Vec<ExactValue>,
    caller_storage:&mut ProjectedStorage, trace:bool,dispatch:&mut Dispatch<'a>,
    ranges:&HashMap<PreparedCallable,RangeRank>, visits:&[InitializationVisit], allow_single_failure:bool, reached_requirement:&std::cell::Cell<bool>,
    read_array:impl Fn(&ProjectedStorage,&observed_f32_array::Cell,&ScalarType)->Option<(ExactValue,usize)>,
    read_integer_field:impl Fn(&ProjectedStorage,&super::super::checker_flow::IntegerFieldCell,&ScalarType)->Option<(ExactValue,usize)>,
) -> Option<(Option<ExactValue>,Option<ObservedTrace>,usize,Option<SelectedRequirement>)> {
    // Keep each activation transactional. No partial state escapes if a
    // later operation is unsupported or a selected requirement stops it.
    dispatch.stop = None;
    let mut next_storage=caller_storage.clone();
    let storage=&mut next_storage;
    let mut active=HashSet::new();
    let mut latest_segment=HashMap::<PreparedCallable,(Rc<ObservedTrace>,usize)>::new();
    let mut values=Vec::new();
    let mut stack=Vec::<Activation>::new();
    let mut tasks=Vec::new();
    let mut work=0usize;
    let mut verified_prefix = Vec::new();
    let mut traces=trace.then(|| vec![Vec::new()]);
    let mut segments=Vec::new();
    let get_symbol=|table:&mut HashMap<_,_>,target| -> Option<_> {
        use std::collections::hash_map::Entry;
        match table.entry(target) {
            Entry::Vacant(slot)=>Some(slot.insert(direct_symbol(project,calls,target)?).clone()),
            Entry::Occupied(slot)=>Some(slot.get().clone()),
        }
    };
    fn enter(body:&FunctionCallSites,target:PreparedCallable,args:Vec<ExactValue>,rank:Option<usize>,value_base:usize,entered_at:Option<SourceSpan>) -> Option<Activation> {
        let ids=body.boundary.checker_parameter_bindings.as_ref()?;
        if ids.len()!=args.len() || args.len()!=body.boundary.parameters.len()
            || !body.boundary.parameters.iter().map(|p| p.ty.clone()).eq(args.iter().map(ExactValue::ty))
        { return None; }
        Some(Activation { target, slots:ids.iter().cloned().zip(args).collect(),rank,value_base,entered_at })
    }
    let (_,body,ret)=get_symbol(&mut dispatch.declarations,target)?;
    stack.push(enter(body,target,args,None,0,None)?);
    active.insert(target);
    let output=if ret==ScalarType::Unit { Output::Unit } else { Output::Scalar };
    tasks.extend([Task::Finish(None,ret),Task::Block(body.structure.root,0,output)]);
    while let Some(task)=tasks.pop() {
        charge_call_work(&mut work,1)?;
        let frame=stack.last_mut()?;
        let (function,body,_) = get_symbol(&mut dispatch.declarations,frame.target)?;
        let owner=&project.modules.get(frame.target.0)?.source;
        let st=&body.structure;
        let edge=|at,edge| match st.edges.get(&(OccurrenceParent::Expression(at),edge))? {
            OccurrenceNode::Expression(found)=>Some(*found),
            _=>None,
        };
        let span=|at:OccurrenceOrdinal| Some(SourceSpan::new(owner.clone(),body.occurrences.get(at.0)?.span));
        match task {
            Task::Block(block,index,output) => {
                let region=st.blocks.get(block.0)?;
                if region.unsafe_context { return None; }
                let Some(item)=region.items.get(index) else {
                    if !matches!(output,Output::Unit) { return None; }
                    values.push(ResultValue::Unit);
                    continue;
                };
                if let BlockItemKind::FinalOutput(slot)=item.kind {
                    if !matches!(output,Output::Scalar) || slot.output!=0 || index+1!=region.items.len() { return None; }
                    tasks.push(Task::Eval(slot.expression));
                    continue;
                }
                tasks.push(Task::Block(block,index+1,output));
                match &item.kind {
                    BlockItemKind::Binding(binding) => {
                        let BindingInitialization::Outputs(outputs)=&binding.initialization else { return None; };
                        let [slot]=binding.receivers() else { return None; };
                        if !outputs.secondary.is_empty() || slot.contextual_storage.is_some()
                            || st.checked_types.expressions.get(&outputs.primary)? != &slot.ty
                        { return None; }
                        let id=ReferenceBindingId { source:owner.clone(),function_span:function.span,
                            declaration_span:slot.span,block_span:region.span,kind:super::super::ReferenceBindingKind::Declared };
                        tasks.extend([Task::Bind(id,slot.ty.clone()),Task::Eval(outputs.primary)]);
                    }
                    BlockItemKind::Evaluation { expression } => {
                        if st.checker_directives.contains_key(expression) {
                            let condition=(*dispatch.requirement_conditions.entry((frame.target,*expression))
                                .or_insert_with(|| requirement_condition(body,*expression).map(|(condition,_)| condition)))?;
                            tasks.extend([Task::DropUnit,Task::Requirement(*expression),Task::Eval(condition)]);
                        } else {
                            let ty=st.checked_types.expressions.get(expression)?;
                            let discard=match ty {
                                ScalarType::Unit=>Task::DropUnit,
                                ScalarType::I32 if st.checker_direct_targets.contains_key(expression)=>Task::DropInteger(ty.clone()),
                                _=>return None,
                            };
                            tasks.extend([discard,Task::Eval(*expression)]);
                        }
                    }
                    BlockItemKind::Assignment { group } => {
                        let assignment=body.assignments.get(&(group.block,group.item))?;
                        let [rhs]=assignment.rhs_evaluations().as_slice() else { return None; };
                        let [destination]=assignment.destinations().as_slice() else { return None; };
                        let AssignmentReplacement::Resolved(ty)=&destination.replacement else { return None; };
                        if assignment.projected_outputs().len()!=0 || st.checked_types.expressions.get(&rhs.expression)?!=ty
                            || st.checked_types.places.get(&destination.destination)?!=ty { return None; }
                        match body.prepared_direct_destination(destination.destination) {
                            Some(DirectDestinationIdentity::Existing(id)) => {
                                let formals=body.boundary.checker_parameter_bindings.as_ref()?;
                                if formals.contains(id) || frame.slots.get(id)?.ty()!=*ty { return None; }
                                tasks.extend([Task::ReplaceLocal(id.clone(),ty.clone()),Task::Eval(rhs.expression)]);
                            }
                            direct => {
                                let p=if let Some(DirectDestinationIdentity::Module(declaration))=direct {
                                    if &declaration.source!=owner || !matches!(ty,ScalarType::I32|ScalarType::Bool) { return None; }
                                    Place { declaration:declaration.clone(),selectors:Vec::new(),ty:ty.clone(),field:None }
                                } else { Place::indexed(body,destination.destination,ty,owner)? };
                                tasks.extend([Task::WritePlace(p),Task::Eval(rhs.expression)]);
                            }
                        }
                    }
                    _=>return None,
                }
            }
            Task::DropUnit => if !matches!(values.pop()?,ResultValue::Unit) { return None; },
            Task::DropInteger(ty) => if scalar(values.pop()?)?.ty()!=ty { return None; },
            Task::ReplaceLocal(id,ty) => {
                let value=scalar(values.pop()?)?;
                let slot=frame.slots.get_mut(&id)?;
                if value.ty()!=ty || slot.ty()!=ty { return None; }
                *slot=value;
            }
            Task::Bind(id,ty) => {
                let value=scalar(values.pop()?)?;
                if value.ty()!=ty || frame.slots.insert(id,value).is_some() { return None; }
            }
            Task::Eval(at) => {
                let ty=st.checked_types.expressions.get(&at)?;
                if let Some(OccurrenceNode::Block(block)) = st.edges.get(&(
                    OccurrenceParent::Expression(at), OccurrenceEdge::BlockBody,
                )) {
                    if let Some(mut pending) = pending_external_call(body, owner, *block, frame, storage) {
                        pending.invocation.extend(stack.iter().filter_map(|activation| activation.entered_at.clone()));
                        pending.verified_prefix = VerifiedRequirePrefix::seal(project, verified_prefix)?;
                        dispatch.stop = Some(ProviderStop::External(pending));
                        return None;
                    }
                }
                if st.checker_contextual_occurrences.contains(&at) { return None; }
                if let Some(value)=st.prepared_range_literals.get(&at) {
                    if !integer_literal_fits_type(value,ty) { return None; }
                    values.push(ResultValue::Scalar(ExactValue::Integer(ty.clone(),value.clone())));
                } else if let Some(bits)=st.projected_f32_literals.get(&at) {
                    if *ty!=ScalarType::F32 { return None; }
                    values.push(ResultValue::Scalar(ExactValue::F32(super::super::checker_flow::projected_f32::ExactF32::checked_bits(*bits)?)));
                } else if let Some(value)=st.observed_bool_literals.get(&at) {
                    values.push(ResultValue::Scalar(ExactValue::Boolean(*value)));
                } else if let Some((id,tag))=st.observed_enum_tags.get(&at) {
                    if !matches!(ty,ScalarType::Enum(expected) if expected==id) { return None; }
                    values.push(ResultValue::Scalar(ExactValue::Enum(id.clone(),*tag)));
                } else if let Some(name)=body.prepared_plain_name(at) {
                    let value=match name {
                        PlainNameIdentity::Lexical(id)=>frame.slots.get(id)?.clone(),
                        PlainNameIdentity::ModuleValue(decl) if &decl.source==owner=>storage.observed_atom(decl)?,
                        _=>return None,
                    };
                    if &value.ty()!=ty { return None; }
                    values.push(ResultValue::Scalar(value));
                } else if let Some(OccurrenceNode::Place(place))=st.edges.get(&(OccurrenceParent::Expression(at),OccurrenceEdge::ExpressionPlace)) {
                    let place=Place::indexed(body,*place,ty,owner)?;
                    charge_call_work(&mut work,place.selectors.len())?;
                    tasks.push(Task::Read(place.clone()));
                    tasks.extend(place.selectors.iter().map(|(site,_)| Task::Eval(*site)));
                } else if let Some(region)=body.conditionals.get(&at) {
                    let (at,yes,no,output)=match *region {
                        ConditionalRegion::Unit { condition,then_block,.. } if *ty==ScalarType::Unit=>(condition,then_block,None,Output::Unit),
                        ConditionalRegion::Value { condition,then_block,else_block,.. }=>
                            (condition,then_block,Some(else_block),if *ty==ScalarType::Unit { Output::Unit } else { Output::Scalar }),
                        _=>return None,
                    };
                    tasks.extend([Task::Branch(at,yes,no,output),Task::Eval(at)]);
                } else if let Some(cut)=body.short_circuits.get(&at) {
                    if *ty!=ScalarType::Bool || cut.evaluate_right_when==cut.bypass { return None; }
                    tasks.extend([Task::Short(cut.left,cut.evaluate_right_when,cut.right),Task::Eval(cut.left)]);
                } else if let Some((l,op,r))=st.prepared_range_binary.get(&at).or_else(|| st.prepared_integer_bitwise.get(&at)).or_else(|| st.observed_enum_comparisons.get(&at)) {
                    tasks.extend([Task::Binary(op.clone()),Task::Eval(*r),Task::Eval(*l)]);
                } else if let Some((l,op,r))=st.projected_f32_comparisons.get(&at).or_else(|| st.projected_f32_math.get(&at)) {
                    tasks.extend([Task::Binary(op.clone()),Task::Eval(*r),Task::Eval(*l)]);
                } else if let Some(operand)=st.observed_bool_negations.get(&at) {
                    tasks.extend([Task::Not,Task::Eval(*operand)]);
                } else if let Some(operand)=st.observed_integer_negations.get(&at) {
                    tasks.extend([Task::Negate(ty.clone()),Task::Eval(*operand)]);
                } else {
                    let signature=body.checked_call_signatures.get(&at)?;
                    if !matches!(signature.outputs.outputs.as_slice(),[out] if &out.ty==ty) { return None; }
                    tasks.push(Task::Call(at,signature.parameters.len()));
                    charge_call_work(&mut work,signature.parameters.len())?;
                    for (i,ty) in signature.parameters.iter().enumerate().rev() {
                        let arg=edge(at,OccurrenceEdge::CallArgument(i))?;
                        if st.checked_types.expressions.get(&arg)?!=ty { return None; }
                        tasks.push(Task::Eval(arg));
                    }
                }
            }
            Task::Read(place) => {
                let args=values.split_off(values.len().checked_sub(place.selectors.len())?);
                let indices=args.into_iter().map(scalar).collect::<Option<_>>()?;
                let cell=place.cell(indices)?;
                let (value,cost)=if let Some(field)=place.field {
                    let cell=super::super::checker_flow::IntegerFieldCell { cell, structure:field.structure,field:field.index };
                    read_integer_field(storage,&cell,&place.ty)?
                } else { read_array(storage,&cell,&place.ty)? };
                charge_call_work(&mut work,cost)?;
                values.push(ResultValue::Scalar(value));
            }
            Task::WritePlace(place) => {
                let rhs=scalar(values.pop()?)?;
                charge_call_work(&mut work,place.selectors.len())?;
                tasks.push(Task::Write(place.clone(),rhs));
                tasks.extend(place.selectors.iter().map(|(site,_)| Task::Eval(*site)));
            }
            Task::Write(place,value) => {
                if value.ty()!=place.ty { return None; }
                let args=values.split_off(values.len().checked_sub(place.selectors.len())?);
                let indices=args.into_iter().map(scalar).collect::<Option<_>>()?;
                let cell=place.cell(indices)?;
                if let Some(field) = place.field {
                    let ExactValue::Integer(ty,value) = value else { return None; };
                    let cell=super::super::checker_flow::IntegerFieldCell { cell,structure:field.structure,field:field.index };
                    storage.overwrite_integer_field(cell,ty,i32::try_from(&value).ok()?)?;
                    continue;
                }
                match value {
                    ExactValue::F32(value) if !cell.indices.is_empty()=>storage.overwrite_f32_array_element(cell,value)?,
                    ExactValue::Integer(ScalarType::I32,value) if cell.indices.is_empty()=>
                        storage.overwrite_scalar(&cell.declaration,&ScalarType::I32,&value)?,
                    ExactValue::Integer(ScalarType::I32,value)=>
                        storage.overwrite_i32_array_element(cell,i32::try_from(&value).ok()?)?,
                    ExactValue::Boolean(value) if cell.indices.is_empty()=>
                        storage.overwrite_scalar(&cell.declaration,&ScalarType::Bool,&BigInt::from(u8::from(value)))?,
                    ExactValue::Integer(ScalarType::U8,value) if !cell.indices.is_empty()=>
                        storage.overwrite_u8_array_element(cell,u8::try_from(&value).ok()?)?,
                    ExactValue::Integer(ScalarType::U16,value) if !cell.indices.is_empty()=>
                        storage.overwrite_u16_array_element(cell,u16::try_from(&value).ok()?)?,
                    _=>return None,
                }
            }
            Task::Binary(op) => {
                let r=scalar(values.pop()?)?;
                let l=scalar(values.pop()?)?;
                values.push(ResultValue::Scalar(ordinary_binary(&op,l,r)?));
            }
            Task::Not => {
                let ExactValue::Boolean(value)=scalar(values.pop()?)? else { return None; };
                values.push(ResultValue::Scalar(ExactValue::Boolean(!value)));
            }
            Task::Negate(expected) => {
                let ExactValue::Integer(ty,value)=scalar(values.pop()?)? else { return None; };
                if ty!=expected { return None; }
                let value= -value;
                if !integer_literal_fits_type(&value,&ty) { return None; }
                values.push(ResultValue::Scalar(ExactValue::Integer(ty,value)));
            }
            Task::Requirement(site) => {
                reached_requirement.set(true);
                let ExactValue::Boolean(accepted)=scalar(values.pop()?)? else { return None; };
                #[cfg(test)]
                {
                    let authored=span(site)?;
                    REQUIREMENT_RESULTS.with(|log| log.borrow_mut().push((authored,accepted)));
                }
                if accepted {
                    verified_prefix.push((frame.target.0, body.occurrences.get(site.0)?.span));
                    // Require is not a scope-narrowing operation. Its valid
                    // actual state simply follows the next prepared item.
                    values.push(ResultValue::Unit);
                    continue;
                }
                if !allow_single_failure {
                    dispatch.stop = Some(ProviderStop::RefutedRow {
                        directive: span(site)?,
                        invocation: stack.iter().filter_map(|activation| activation.entered_at.clone()).collect(),
                        verified_prefix: VerifiedRequirePrefix::seal(project, verified_prefix)?,
                    });
                    return None;
                }
                let owner=frame.target;
                let mut paths=traces.take()?;
                if paths.len()!=stack.len() { return None; }
                // Preserve active entries as open; never forge a return on
                // the failure prefix. Completed nested segments stay shared.
                for activation in stack.iter().skip(1).rev() {
                    let events=paths.pop()?;
                    let nested=segments.len();
                    segments.push(events);
                    paths.last_mut()?.push(ObservedStep::Entered {site:activation.entered_at.clone()?,nested});
                }
                let root=paths.pop()?;
                let history=ObservedTrace::new(root,segments,&mut work)?;
                return Some((None,None,work,Some(SelectedRequirement {target:owner,site,history})));
            }
            Task::Branch(at,yes,no,output) => {
                let ExactValue::Boolean(accepted)=scalar(values.pop()?)? else { return None; };
                if let Some(trace)=&mut traces {
                    charge_call_work(&mut work,1)?;
                    trace.last_mut()?.push(ObservedStep::Branch {site:span(at)?,accepted});
                }
                if let Some(block)=if accepted { Some(yes) } else { no } { tasks.push(Task::Block(block,0,output)); }
                else if matches!(output,Output::Unit) { values.push(ResultValue::Unit); }
                else { return None; }
            }
            Task::Short(at,conjunction,right) => {
                let ExactValue::Boolean(left)=scalar(values.pop()?)? else { return None; };
                if let Some(trace)=&mut traces {
                    charge_call_work(&mut work,1)?;
                    trace.last_mut()?.push(ObservedStep::Branch {site:span(at)?,accepted:left});
                }
                if left==conjunction { tasks.push(Task::Eval(right)); }
                else { values.push(ResultValue::Scalar(ExactValue::Boolean(left))); }
            }
            Task::Call(site,n) => {
                let args=values.split_off(values.len().checked_sub(n)?).into_iter().map(scalar).collect::<Option<Vec<_>>>()?;
                let signature=body.checked_call_signatures.get(&site)?;
                let [out]=signature.outputs.outputs.as_slice() else { return None; };
                if !signature.parameters.iter().cloned().eq(args.iter().map(ExactValue::ty)) { return None; }
                if body.resolved_call_site(site)?.target==ResolvedCallTarget::Core(super::super::CoreOperationId::SintToFloat) {
                    let [ExactValue::Integer(ScalarType::I32,value)]=args.as_slice() else { return None; };
                    if out.ty!=ScalarType::F32 { return None; }
                    let value=super::super::checker_flow::projected_f32::ExactF32::signed_i32(i32::try_from(value).ok()?);
                    values.push(ResultValue::Scalar(ExactValue::F32(value)));
                    continue;
                }
                if body.resolved_call_site(site)?.target==ResolvedCallTarget::Core(super::super::CoreOperationId::IntExtend) {
                    let [ExactValue::Integer(input,value)]=args.as_slice() else { return None; };
                    let value=match (input,&out.ty) {
                        (ScalarType::I32,ScalarType::U64)=>BigInt::from(i64::from(i32::try_from(value).ok()?) as u64),
                        (ScalarType::I32,ScalarType::I64)=>BigInt::from(i64::from(i32::try_from(value).ok()?)),
                        (ScalarType::U8,ScalarType::I32)=>BigInt::from(i32::from(u8::try_from(value).ok()?)),
                        _=>return None,
                    };
                    values.push(ResultValue::Scalar(ExactValue::Integer(out.ty.clone(),value)));
                    continue;
                }
                if body.resolved_call_site(site)?.target==ResolvedCallTarget::Core(super::super::CoreOperationId::IntTrunc) {
                    let [ExactValue::Integer(input,value)]=args.as_slice() else { return None; };
                    let value=match (input,&out.ty) {
                        (ScalarType::I32,ScalarType::U16)=>BigInt::from(i32::try_from(value).ok()? as u16),
                        (ScalarType::I32,ScalarType::U8)=>BigInt::from(i32::try_from(value).ok()? as u8),
                        (ScalarType::I64,ScalarType::I32)=>BigInt::from(i64::try_from(value).ok()? as i32),
                        (ScalarType::U64,ScalarType::I32)=>BigInt::from(u64::try_from(value).ok()? as i32),
                        _=>return None,
                    };
                    values.push(ResultValue::Scalar(ExactValue::Integer(out.ty.clone(),value)));
                    continue;
                }
                let child=authenticate_prepared_direct_edge(project,calls,frame.target.0,body,site,signature)?;
                if child.0!=frame.target.0 && !completed_namespace_owner(owner,&project.modules.get(child.0)?.source,child.0,Some(visits)) {
                    return None;
                }
                // Reuse the independently sealed pure engine for precisely
                // scoped F32-input functions. Its certificate forbids all
                // state reads/writes and opaque calls; never try it for find.
                if child!=frame.target && !args.is_empty() && signature.parameters.iter().all(|ty| *ty==ScalarType::F32)
                    && out.ty!=ScalarType::Unit {
                    charge_call_work(&mut work,n)?;
                    let bits=args.iter().map(|value| match value { ExactValue::F32(v)=>Some(v.0),_=>None })
                        .collect::<Option<Vec<_>>>()?;
                    let cached=dispatch.latest_pure.get(&child).filter(|(key,_,_)| key==&bits)
                        .map(|(_,value,trace)| (value.clone(),trace.clone()));
                    let (value,inner)=if let Some(cached)=cached { cached } else {
                        let (plan,function)=dispatch.pure_plans.entry(child).or_insert_with(||
                            ordered_float_prefix::plan_observed_math_call(project,calls,child)).as_ref()?;
                        let locals=function.formals.iter().zip(args).map(|((id,_),value)| (id.clone(),value)).collect();
                        let (mut returned,inner)=ordered_float_prefix::evaluate_observed_prefix(plan,Some(function),locals,
                            |_,_,_| None,trace,&mut work,|func,at| {
                                if func.0!=child.0 { return None; }
                                let owner_body=calls.function_body(func.0,func.1,None)?;
                                let source=&project.modules.get(func.0)?.source;
                                Some(SourceSpan::new(source.clone(),owner_body.occurrences.get(at.0)?.span))
                            })?;
                        if returned.len()!=1 { return None; }
                        let value=returned.pop()?;
                        let inner=inner.map(Rc::new);
                        dispatch.latest_pure.insert(child,(bits,value.clone(),inner.clone()));
                        (value,inner)
                    };
                    if value.ty()!=out.ty { return None; }
                    if let (Some(stacks),Some(inner))=(&mut traces,inner) {
                        let nested=if let Some((_,id))=latest_segment.get(&child).filter(|(known,_)| Rc::ptr_eq(known,&inner)) {
                            *id
                        } else {
                            let base=segments.len();
                            let offset_steps=|steps:&[ObservedStep]| -> Option<Vec<ObservedStep>> {
                                steps.iter().map(|step| Some(match step {
                                    ObservedStep::Call {site,nested}=>ObservedStep::Call {site:site.clone(),nested:base.checked_add(*nested)?},
                                    _=>step.clone(),
                                })).collect()
                            };
                            charge_call_work(&mut work,inner.physical_cost)?;
                            for block in &inner.segments { segments.push(offset_steps(block)?); }
                            let nested=segments.len();
                            segments.push(offset_steps(&inner.root)?);
                            latest_segment.insert(child,(inner,nested));
                            nested
                        };
                        charge_call_work(&mut work,1)?;
                        stacks.last_mut()?.push(ObservedStep::Call {site:span(site)?,nested});
                    }
                    values.push(ResultValue::Scalar(value));
                    continue;
                }
                let rank=if child==frame.target {
                    let bindings=body.boundary.checker_parameter_bindings.as_deref()?;
                    let advancing=ranges.get(&child).copied().or_else(||
                        *dispatch.range_ranks.entry(child).or_insert_with(|| advancing_range_plan(body,child)));
                    if let Some(rank)=advancing {
                        rank.decreases(&frame.slots,bindings,&args)?;
                        None
                    } else {
                    if out.ty==ScalarType::Unit || !matches!(tasks.last(),Some(Task::Finish(..))) { return None; }
                    let use_rank=|slot| -> Option<usize> {
                        let id=bindings.get(slot)?;
                        rank_argument(body,site,slot,id)?;
                        let prior=rank_value(frame.slots.get(id)?)?;
                        let next=rank_value(args.get(slot)?)?;
                        (next>=0 && next.checked_add(1)?==prior).then_some(slot)
                    };
                    Some(match frame.rank { Some(slot)=>use_rank(slot)?, None=>(0..n).find_map(use_rank)? })
                    }
                } else {
                    if !active.insert(child) { return None; }
                    None
                };
                let (_,callee,result)=get_symbol(&mut dispatch.declarations,child)?;
                let output=if result==ScalarType::Unit { Output::Unit } else { Output::Scalar };
                if let Some(traces)=&mut traces { traces.push(Vec::new()); }
                let entered=span(site)?;
                tasks.extend([Task::Finish(Some(entered.clone()),result),Task::Block(callee.structure.root,0,output)]);
                charge_call_work(&mut work,n)?;
                stack.push(enter(callee,child,args,rank,values.len(),Some(entered))?);
            }
            Task::Finish(site,ty) => {
                if values.len()!=frame.value_base.checked_add(1)? { return None; }
                match values.last()? {
                    ResultValue::Unit if ty==ScalarType::Unit=>{},
                    ResultValue::Scalar(value) if value.ty()==ty=>{},
                    _=>return None,
                }
                let closed=stack.pop()?;
                if stack.last().is_none_or(|parent| parent.target!=closed.target) { active.remove(&closed.target); }
                if let (Some(site),Some(traces))=(site,&mut traces) {
                    let trace=traces.pop()?;
                    charge_call_work(&mut work,trace.len().saturating_add(1))?;
                    let nested=segments.len();
                    segments.push(trace);
                    traces.last_mut()?.push(ObservedStep::Call {site,nested});
                }
            }
        }
    }
    if !stack.is_empty() || !active.is_empty() || values.len()!=1 { return None; }
    let trace=if let Some(mut traces)=traces {
        let root=traces.pop()?;
        if !traces.is_empty() { return None; }
        let mut trace=ObservedTrace::new(root,segments,&mut work)?;
        if !verified_prefix.is_empty() {
            trace.attach_requirements(VerifiedRequirePrefix::seal(project,verified_prefix)?);
        }
        Some(trace)
    } else { None };
    let value=match values.pop()? { ResultValue::Scalar(v)=>Some(v),ResultValue::Unit=>None };
    *caller_storage=next_storage;
    Some((value,trace,work,None))
}
