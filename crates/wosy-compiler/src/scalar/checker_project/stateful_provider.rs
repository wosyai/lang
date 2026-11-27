//! Execute an authenticated scalar call on the current observed row. There
//! is no speculative state, ambient entry or external effects in this lane.
use super::*;
use super::ordered_float_prefix::{self,ExactValue,ObservedStep,ObservedTrace};
use super::super::checker_flow::{ProjectedStorage,apply_observed_integer_operator,charge_call_work,observed_f32_array};
use super::super::reference::{BlockOrdinal,OccurrenceEdge,OccurrenceNode,OccurrenceParent,PlainNameIdentity};
use super::super::control::AssignmentReplacement;
use wosy_syntax::SourceSpan;

#[derive(Clone)]
struct Place {
    declaration: SourceSpan,
    selectors: Vec<(OccurrenceOrdinal,u64)>,
    ty: ScalarType,
}

impl Place {
    fn indexed(body:&FunctionCallSites,site:OccurrenceOrdinal,ty:&ScalarType,owner:&SourceIdentity) -> Option<Self> {
        let checked=observed_f32_array::typed_place(body,site,ty)?;
        if !matches!(ty,ScalarType::I32|ScalarType::F32|ScalarType::U16) || &checked.declaration.source!=owner { return None; }
        Some(Self { declaration:checked.declaration.clone(),selectors:checked.selectors,ty:ty.clone() })
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
}

enum Task {
    Block(BlockOrdinal,usize,Output),
    Eval(OccurrenceOrdinal),
    DropUnit,
    Bind(ReferenceBindingId,ScalarType),
    Read(Place),
    WritePlace(Place),
    Write(Place,ExactValue),
    Binary(BinaryOperator),
    Not,
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
        || body.contextual_formal().is_some() || !body.allocation_length_calls.is_empty() || has_callable_obligations(body)
        || body.structure.blocks.iter().any(|block| block.unsafe_context)
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

#[derive(Default)]
pub(super) struct Dispatch<'a> {
    declarations:HashMap<PreparedCallable,(&'a ScalarFunction,&'a FunctionCallSites,ScalarType)>,
    pure_plans:HashMap<PreparedCallable,Option<(ordered_float_prefix::PrefixCertificate,ordered_float_prefix::FunctionEffects)>>,
    latest_pure:HashMap<PreparedCallable,(Vec<u32>,ExactValue,Option<Rc<ObservedTrace>>)>,
}

pub(super) fn run<'a>(
    project:&'a ScalarProject,calls:&'a ProjectCallSiteResults,target:PreparedCallable,args:Vec<ExactValue>,
    storage:&mut ProjectedStorage, trace:bool,dispatch:&mut Dispatch<'a>,
    read_array:impl Fn(&ProjectedStorage,&observed_f32_array::Cell,&ScalarType)->Option<(ExactValue,usize)>,
) -> Option<(Option<ExactValue>,Option<ObservedTrace>,usize)> {
    let owner=&project.modules.get(target.0)?.source;
    let mut active=HashSet::new();
    let mut latest_segment=HashMap::<PreparedCallable,(Rc<ObservedTrace>,usize)>::new();
    let mut values=Vec::new();
    let mut stack=Vec::<Activation>::new();
    let mut tasks=Vec::new();
    let mut work=0usize;
    let mut traces=trace.then(|| vec![Vec::new()]);
    let mut segments=Vec::new();
    let get_symbol=|table:&mut HashMap<_,_>,target| -> Option<_> {
        use std::collections::hash_map::Entry;
        match table.entry(target) {
            Entry::Vacant(slot)=>Some(slot.insert(direct_symbol(project,calls,target)?).clone()),
            Entry::Occupied(slot)=>Some(slot.get().clone()),
        }
    };
    fn enter(body:&FunctionCallSites,target:PreparedCallable,args:Vec<ExactValue>,rank:Option<usize>,value_base:usize) -> Option<Activation> {
        let ids=body.boundary.checker_parameter_bindings.as_ref()?;
        if ids.len()!=args.len() || args.len()!=body.boundary.parameters.len()
            || !body.boundary.parameters.iter().map(|p| p.ty.clone()).eq(args.iter().map(ExactValue::ty))
        { return None; }
        Some(Activation { target, slots:ids.iter().cloned().zip(args).collect(),rank,value_base })
    }
    let (_,body,ret)=get_symbol(&mut dispatch.declarations,target)?;
    stack.push(enter(body,target,args,None,0)?);
    active.insert(target);
    let output=if ret==ScalarType::Unit { Output::Unit } else { Output::Scalar };
    tasks.extend([Task::Finish(None,ret),Task::Block(body.structure.root,0,output)]);
    while let Some(task)=tasks.pop() {
        charge_call_work(&mut work,1)?;
        let frame=stack.last_mut()?;
        let (function,body,_) = get_symbol(&mut dispatch.declarations,frame.target)?;
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
                        if st.checked_types.expressions.get(expression)?!=&ScalarType::Unit { return None; }
                        tasks.extend([Task::DropUnit,Task::Eval(*expression)]);
                    }
                    BlockItemKind::Assignment { group } => {
                        let assignment=body.assignments.get(&(group.block,group.item))?;
                        let [rhs]=assignment.rhs_evaluations().as_slice() else { return None; };
                        let [destination]=assignment.destinations().as_slice() else { return None; };
                        let AssignmentReplacement::Resolved(ty)=&destination.replacement else { return None; };
                        if assignment.projected_outputs().len()!=0 || st.checked_types.expressions.get(&rhs.expression)?!=ty
                            || st.checked_types.places.get(&destination.destination)?!=ty { return None; }
                        let p=if let Some(DirectDestinationIdentity::Module(declaration))=body.prepared_direct_destination(destination.destination) {
                            if &declaration.source!=owner || !matches!(ty,ScalarType::I32|ScalarType::Bool) { return None; }
                            Place { declaration:declaration.clone(),selectors:Vec::new(),ty:ty.clone() }
                        } else { Place::indexed(body,destination.destination,ty,owner)? };
                        tasks.extend([Task::WritePlace(p),Task::Eval(rhs.expression)]);
                    }
                    _=>return None,
                }
            }
            Task::DropUnit => if !matches!(values.pop()?,ResultValue::Unit) { return None; },
            Task::Bind(id,ty) => {
                let value=scalar(values.pop()?)?;
                if value.ty()!=ty || frame.slots.insert(id,value).is_some() { return None; }
            }
            Task::Eval(at) => {
                let ty=st.checked_types.expressions.get(&at)?;
                if st.checker_contextual_occurrences.contains(&at) { return None; }
                if let Some(value)=st.prepared_range_literals.get(&at) {
                    if !integer_literal_fits_type(value,ty) { return None; }
                    values.push(ResultValue::Scalar(ExactValue::Integer(ty.clone(),value.clone())));
                } else if let Some(bits)=st.projected_f32_literals.get(&at) {
                    if *ty!=ScalarType::F32 { return None; }
                    values.push(ResultValue::Scalar(ExactValue::F32(super::super::checker_flow::projected_f32::ExactF32::checked_bits(*bits)?)));
                } else if let Some(value)=st.observed_bool_literals.get(&at) {
                    values.push(ResultValue::Scalar(ExactValue::Boolean(*value)));
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
                } else if let Some((l,op,r))=st.prepared_range_binary.get(&at).or_else(|| st.prepared_i32_bitwise.get(&at)) {
                    tasks.extend([Task::Binary(op.clone()),Task::Eval(*r),Task::Eval(*l)]);
                } else if let Some((l,op,r))=st.projected_f32_comparisons.get(&at).or_else(|| st.projected_f32_math.get(&at)) {
                    tasks.extend([Task::Binary(op.clone()),Task::Eval(*r),Task::Eval(*l)]);
                } else if let Some(operand)=st.observed_bool_negations.get(&at) {
                    tasks.extend([Task::Not,Task::Eval(*operand)]);
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
                let (value,cost)=read_array(storage,&cell,&place.ty)?;
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
                match value {
                    ExactValue::F32(value) if !cell.indices.is_empty()=>storage.overwrite_f32_array_element(cell,value)?,
                    ExactValue::Integer(ScalarType::I32,value) if cell.indices.is_empty()=>
                        storage.overwrite_scalar(&cell.declaration,&ScalarType::I32,&value)?,
                    ExactValue::Integer(ScalarType::I32,value)=>
                        storage.overwrite_i32_array_element(cell,i32::try_from(&value).ok()?)?,
                    ExactValue::Boolean(value) if cell.indices.is_empty()=>
                        storage.overwrite_scalar(&cell.declaration,&ScalarType::Bool,&BigInt::from(u8::from(value)))?,
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
                if body.resolved_call_site(site)?.target==ResolvedCallTarget::Core(super::super::CoreOperationId::IntExtend) {
                    let [ExactValue::Integer(ScalarType::I32,value)]=args.as_slice() else { return None; };
                    if out.ty!=ScalarType::U64 { return None; }
                    let value=i64::from(i32::try_from(value).ok()?) as u64;
                    values.push(ResultValue::Scalar(ExactValue::Integer(ScalarType::U64,value.into())));
                    continue;
                }
                if body.resolved_call_site(site)?.target==ResolvedCallTarget::Core(super::super::CoreOperationId::IntTrunc) {
                    let [ExactValue::Integer(ScalarType::I32,value)]=args.as_slice() else { return None; };
                    if out.ty!=ScalarType::U16 { return None; }
                    let value=i32::try_from(value).ok()? as u16;
                    values.push(ResultValue::Scalar(ExactValue::Integer(ScalarType::U16,value.into())));
                    continue;
                }
                let child=authenticate_prepared_direct_edge(project,calls,target.0,body,site,signature)?;
                if child.0!=target.0 { return None; }
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
                                if func.0!=target.0 { return None; }
                                let owner_body=calls.function_body(func.0,func.1,None)?;
                                Some(SourceSpan::new(owner.clone(),owner_body.occurrences.get(at.0)?.span))
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
                    if !matches!(tasks.last(),Some(Task::Finish(..))) { return None; }
                    let bindings=body.boundary.checker_parameter_bindings.as_deref()?;
                    let use_rank=|slot| -> Option<usize> {
                        let id=bindings.get(slot)?;
                        rank_argument(body,site,slot,id)?;
                        let prior=rank_value(frame.slots.get(id)?)?;
                        let next=rank_value(args.get(slot)?)?;
                        (next>=0 && next.checked_add(1)?==prior).then_some(slot)
                    };
                    Some(match frame.rank { Some(slot)=>use_rank(slot)?, None=>(0..n).find_map(use_rank)? })
                } else {
                    if !active.insert(child) { return None; }
                    None
                };
                let (_,callee,result)=get_symbol(&mut dispatch.declarations,child)?;
                let output=if result==ScalarType::Unit { Output::Unit } else { Output::Scalar };
                if let Some(traces)=&mut traces { traces.push(Vec::new()); }
                tasks.extend([Task::Finish(Some(span(site)?),result),Task::Block(callee.structure.root,0,output)]);
                charge_call_work(&mut work,n)?;
                stack.push(enter(callee,child,args,rank,values.len())?);
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
        Some(ObservedTrace::new(root,segments,&mut work)?)
    } else { None };
    let value=match values.pop()? { ResultValue::Scalar(v)=>Some(v),ResultValue::Unit=>None };
    Some((value,trace,work))
}
