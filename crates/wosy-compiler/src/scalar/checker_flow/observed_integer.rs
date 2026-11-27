use super::*;

type Span = wosy_syntax::SourceSpan;
use super::super::reference::{ModuleStorageRole, NamedPlaceRootIdentity};

pub(super) struct Place<'a> {
    pub(super) site: OccurrenceOrdinal,
    pub(super) declaration: &'a Span,
    pub(super) ty: &'a ScalarType,
    index: Option<(u64,OccurrenceOrdinal)>,
}

impl Place<'_> {
    pub(super) fn role(&self) -> ModuleStorageRole {
        if self.index.is_some() { ModuleStorageRole::Project } else { ModuleStorageRole::Read }
    }

    fn selector(&self, selector: Option<BigInt>) -> Option<Option<usize>> {
        match (self.index,selector) {
            (None,None) => Some(None),
            (Some((length,_)),Some(selector)) => {
                let i = u64::try_from(&selector).ok()?;
                if i >= length { return None; }
                Some(Some(usize::try_from(i).ok()?))
            }
            _ => None,
        }
    }

    fn read(&self, row: &IntegerRow, selector: Option<BigInt>) -> Option<BigInt> {
        let index = self.selector(selector)?;
        let storage = row.storage.as_ref()?;
        let leaf = match index {
            Some(i) => {
                let (ScalarType::Array { element,length,.. },cells) = storage.fixed_arrays.get(self.declaration)? else { return None; };
                if element.as_ref() != self.ty || Some(*length) != self.index.map(|(length,_)| length) { return None; }
                cells.get(i)?
            }
            None => {
                let (ty,value) = storage.scalars.get(self.declaration)?;
                if ty != self.ty { return None; }
                value
            }
        };
        let current = storage.changes.get(&(self.declaration.clone(),index)).unwrap_or(leaf);
        Some(current.singleton_value()?.clone())
    }
}

fn indexed_place<'a>(calls: &'a FunctionCallSites, at: OccurrenceOrdinal, ty: &'a ScalarType) -> Option<Place<'a>> {
    let st = &calls.structure;
    let parent = OccurrenceParent::Place(at);
    let OccurrenceNode::Place(base) = st.edges.get(&(parent,OccurrenceEdge::PlaceBase))? else { return None; };
    let OccurrenceNode::Expression(index) = st.edges.get(&(parent,OccurrenceEdge::PlaceIndex))? else { return None; };
    let Some(NamedPlaceRootIdentity::ModuleValue(declaration)) = calls.prepared_named_place_root(*base) else { return None; };
    let ScalarType::Array { element,length,.. } = st.checked_types.places.get(base)? else { return None; };
    if element.as_ref() != ty || st.checked_types.places.get(&at)? != ty
        || st.checked_types.expressions.get(index)? != &ScalarType::U64
    { return None; }
    Some(Place { site:*base,declaration,ty,index:Some((*length,*index)) })
}

fn read_place<'a>(calls: &'a FunctionCallSites, at: OccurrenceOrdinal, ty: &'a ScalarType) -> Option<Place<'a>> {
    if let Some(PlainNameIdentity::ModuleValue(declaration)) = calls.prepared_plain_name(at) {
        return Some(Place { site:at,declaration,ty,index:None });
    }
    let OccurrenceNode::Place(place) = calls.structure.edges.get(&(
        OccurrenceParent::Expression(at),OccurrenceEdge::ExpressionPlace))? else { return None; };
    indexed_place(calls,*place,ty)
}

enum Expr<'a> {
    Literal(&'a BigInt),
    Lexical(&'a ReferenceBindingId,&'a ScalarType),
    Module(Place<'a>),
    Binary(OccurrenceOrdinal,&'a BinaryOperator,OccurrenceOrdinal,&'a ScalarType),
    Extend(OccurrenceOrdinal),
    Getter,
    FromFloatCall(Vec<OccurrenceOrdinal>,&'a ScalarType),
}

fn expression(calls: &FunctionCallSites, at: OccurrenceOrdinal) -> Option<Expr<'_>> {
    let st = &calls.structure;
    let ty = st.checked_types.expressions.get(&at)?;
    if !is_integer_type(ty) { return None; }
    if let Some(value) = st.prepared_range_literals.get(&at) {
        return super::super::typecheck::integer_literal_fits_type(value,ty).then_some(Expr::Literal(value));
    }
    if let Some(PlainNameIdentity::Lexical(binding)) = calls.prepared_plain_name(at) {
        return (!st.checker_contextual_occurrences.contains(&at)).then_some(Expr::Lexical(binding,ty));
    }
    if let Some(place) = read_place(calls,at,ty) { return Some(Expr::Module(place)); }
    if let Some((l,op,r)) = st.prepared_range_binary.get(&at) {
        return Some(Expr::Binary(*l,op,*r,ty));
    }
    if observed_getter_signature(calls,at).is_some() { return Some(Expr::Getter); }
    if let Some(args) = observed_float_argument_sites(calls,at,ty) { return Some(Expr::FromFloatCall(args,ty)); }
    use super::super::{CoreOperationId,ResolvedCallTarget};
    if calls.resolved_call_site(at)?.target != ResolvedCallTarget::Core(CoreOperationId::IntExtend) { return None; }
    let sig = calls.checked_call_signatures.get(&at)?;
    if sig.parameters.as_slice() != [ScalarType::I32] || *ty != ScalarType::U64
        || !matches!(sig.outputs.outputs.as_slice(),[out] if out.ty == ScalarType::U64)
    { return None; }
    let OccurrenceNode::Expression(arg) = st.edges.get(&(
        OccurrenceParent::Expression(at),OccurrenceEdge::CallArgument(0)))? else { return None; };
    (st.checked_types.expressions.get(arg)? == &ScalarType::I32).then_some(Expr::Extend(*arg))
}

pub(super) fn sites<'a>(calls: &'a FunctionCallSites, at: OccurrenceOrdinal,
    mut place: impl FnMut(&'a Span,OccurrenceOrdinal,ModuleStorageRole), mut core: impl FnMut(OccurrenceOrdinal),
    mut local: impl FnMut(&'a ReferenceBindingId, &'a ScalarType)) -> Option<()> {
    let mut todo = vec![at];
    while let Some(at) = todo.pop() {
        match expression(calls,at)? {
            Expr::Literal(_) => {}
            Expr::Lexical(binding,ty) => local(binding,ty),
            Expr::Module(value) => {
                place(value.declaration,value.site,value.role());
                if let Some((_,i)) = value.index { todo.push(i); }
            }
            Expr::Binary(l,_,r,_) => todo.extend([r,l]),
            Expr::Extend(input) => { core(at); todo.push(input); }
            Expr::Getter => core(at),
            Expr::FromFloatCall(args,_) => {
                core(at);
                for arg in args {
                    if let Some(value) = observed_f32_array::expression(calls,arg) {
                        place(value.declaration,value.site,ModuleStorageRole::Project);
                    }
                }
            }
        }
    }
    Some(())
}

pub(super) fn evaluate(calls: &FunctionCallSites, at: OccurrenceOrdinal, roots: &IntegerRoots<'_>,
    row: &mut IntegerRow) -> Option<BigInt> {
    evaluate_in_lane(calls,at,roots,row,true)
}

pub(super) fn evaluate_pure_argument(calls: &FunctionCallSites, at: OccurrenceOrdinal, roots: &IntegerRoots<'_>,
    row: &mut IntegerRow) -> Option<BigInt> {
    evaluate_in_lane(calls,at,roots,row,false)
}

fn evaluate_in_lane(calls: &FunctionCallSites, at: OccurrenceOrdinal, roots: &IntegerRoots<'_>,
    row: &mut IntegerRow, use_current_cells: bool) -> Option<BigInt> {
    enum Step<'a> { Eval(OccurrenceOrdinal), Read(Place<'a>), Apply(&'a BinaryOperator,&'a ScalarType), Extend }
    use Step::*;
    let charge = roots.callable_history?;
    let mut todo = vec![Eval(at)];
    let mut values = Vec::<BigInt>::new();
    while let Some(step) = todo.pop() {
        charge.borrow().charge(1)?;
        match step {
            Eval(at) => match expression(calls,at)? {
                Expr::Literal(literal) => values.push(literal.clone()),
                Expr::Lexical(binding,ty) => {
                    let (axis,actual) = roots.lookup(binding)?;
                    if actual != ty { return None; }
                    values.push(row.get(axis)?.singleton_value()?.clone());
                }
                Expr::Module(place) => {
                    if !use_current_cells { return None; }
                    let index = place.index;
                    todo.push(Read(place));
                    if let Some((_,i)) = index { todo.push(Eval(i)); }
                }
                Expr::Binary(l,op,r,ty) => todo.extend([Apply(op,ty),Eval(r),Eval(l)]),
                Expr::Getter => {
                    if !use_current_cells { return None; }
                    let super::super::checker_project::ObservedExactValue::Integer(_, value) =
                        evaluate_observed_getter(calls,at,roots,row)? else { return None; };
                    values.push(value);
                }
                Expr::FromFloatCall(args,expected) => {
                    if !use_current_cells { return None; }
                    let super::super::checker_project::ObservedExactValue::Integer(ty,value)=
                        evaluate_observed_scalar_from_f32(calls,at,&args,roots,row)? else { return None; };
                    if &ty != expected { return None; }
                    values.push(value);
                }
                Expr::Extend(input) => {
                    if !use_current_cells { return None; }
                    todo.extend([Extend,Eval(input)]);
                }
            }
            Read(place) => {
                let index = if place.index.is_some() { Some(values.pop()?) } else { None };
                values.push(place.read(row,index)?);
            }
            Apply(op,ty) => {
                let right = values.pop()?;
                let left = values.pop()?;
                let (min,max) = super::super::typecheck::integer_bounds(ty)?;
                let value = AuthoredIntegerOperation::checked(op)?.apply(Cow::Owned(left),Cow::Owned(right),&min,&max)?;
                if value < min || value > max { return None; }
                values.push(value);
            }
            Extend => {
                // The prepared operation sign-extends its i32 source. The
                // unsigned destination interprets those same 64 bits as u64.
                let signed = i32::try_from(&values.pop()?).ok()?;
                values.push(BigInt::from(i64::from(signed) as u64));
            }
        }
    }
    if values.len()!=1 { return None; }
    values.pop()
}

pub(super) struct Write<'a> { pub(super) place: Place<'a>, pub(super) right: OccurrenceOrdinal }

impl Write<'_> {
    pub(super) fn apply(&self,calls: &FunctionCallSites, roots: &IntegerRoots<'_>,row: &mut IntegerRow) -> Option<()> {
        // Assignment RHS precedes place evaluation, even for self replacement.
        let value = evaluate(calls,self.right,roots,row)?;
        let index = match self.place.index {
            Some((_, site)) => Some(evaluate(calls, site, roots, row)?),
            None => None,
        };
        let index = self.place.selector(index)?;
        let storage = Rc::make_mut(row.storage.as_mut()?);
        match index {
            Some(i) => storage.overwrite_array_element(self.place.declaration,self.place.ty,i,&value),
            None => storage.overwrite_scalar(self.place.declaration,self.place.ty,&value),
        }
    }

    pub(super) fn reads<'a>(&'a self,calls: &'a FunctionCallSites,
        mut place: impl FnMut(&'a Span,OccurrenceOrdinal,ModuleStorageRole),mut core: impl FnMut(OccurrenceOrdinal)) -> Option<()> {
        sites(calls,self.right,&mut place,&mut core,|_,_| {})?;
        if let Some((_,i)) = self.place.index { sites(calls,i,place,core,|_,_| {})?; }
        Some(())
    }
}

pub(super) fn write(calls: &FunctionCallSites, position: BlockPosition) -> Option<Write<'_>> {
    let group = calls.assignments.get(&(position.block,position.item))?;
    let [rhs] = group.rhs_evaluations().as_slice() else { return None; };
    let [destination] = group.destinations().as_slice() else { return None; };
    if group.projected_outputs().len() != 0 { return None; }
    let AssignmentReplacement::Resolved(ty) = &destination.replacement else { return None; };
    if !is_integer_type(ty) || calls.structure.checked_types.expressions.get(&rhs.expression)? != ty
        || calls.structure.checked_types.places.get(&destination.destination)? != ty { return None; }
    let site = destination.destination;
    let place = if let Some(DirectDestinationIdentity::Module(declaration)) = calls.prepared_direct_destination(site) {
        Place { site,declaration,ty,index:None }
    } else { indexed_place(calls,site,ty)? };
    Some(Write { place,right:rhs.expression })
}
