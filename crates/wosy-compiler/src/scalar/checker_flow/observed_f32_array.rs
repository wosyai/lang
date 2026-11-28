use super::*;

type Span = wosy_syntax::SourceSpan;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(in super::super) struct Cell {
    pub(in super::super) declaration: Span,
    pub(in super::super) indices: Box<[u64]>,
}

pub(in super::super) type Read<'a> = dyn Fn(&ProjectedStorage, &Cell) -> Option<(projected_f32::ExactF32,usize)> + 'a;

pub(in super::super) struct Place<'a> {
    pub(in super::super) declaration: &'a Span,
    pub(super) site: OccurrenceOrdinal,
    pub(in super::super) selectors: Vec<(OccurrenceOrdinal, u64)>,
}

impl Place<'_> {
    pub(super) fn address(&self, calls: &FunctionCallSites, roots: &IntegerRoots<'_>, row: &mut IntegerRow) -> Option<Cell> {
        let mut indices = Vec::with_capacity(self.selectors.len());
        for &(index, length) in self.selectors.iter().rev() {
            roots.callable_history?.borrow().charge(1)?;
            // The planner checked u64 literals or exact lexical u64 slots.
            // No storage reads, calls, implicit conversions or dependent index effects.
            let value = if let Some(value) = calls.structure.prepared_range_literals.get(&index) {
                u64::try_from(value).ok()?
            } else {
                let PlainNameIdentity::Lexical(binding) = calls.prepared_plain_name(index)? else { return None; };
                let (axis, ty) = roots.lookup(binding)?;
                if ty != &ScalarType::U64 { return None; }
                u64::try_from(row.get(axis)?.singleton_value()?).ok()?
            };
            if value >= length { return None; }
            indices.push(value);
        }
        Some(Cell { declaration: self.declaration.clone(), indices: indices.into_boxed_slice() })
    }
}

pub(super) fn read(calls: &FunctionCallSites, place: Place<'_>, roots: &IntegerRoots<'_>, row: &mut IntegerRow)
    -> Option<projected_f32::ExactF32> {
    let cell = place.address(calls,roots,row)?;
    let (value,work) = roots.observed_f32_array_read?(row.storage.as_ref()?,&cell)?;
    roots.callable_history?.borrow().charge(work)?;
    Some(value)
}

pub(in super::super) fn typed_place<'a>(calls: &'a FunctionCallSites, cursor: OccurrenceOrdinal, ty: &ScalarType) -> Option<Place<'a>> {
    typed_index_path(calls,cursor,ty,false)
}

/// The stateful cursor evaluates each checked index using its ordinary effect
/// continuation. The simpler float-row reader still needs lexical/literal
/// selectors because it never dispatches index expression calls.
pub(in super::super) fn typed_provider_place<'a>(calls: &'a FunctionCallSites, cursor: OccurrenceOrdinal, ty: &ScalarType) -> Option<Place<'a>> {
    typed_index_path(calls,cursor,ty,true)
}

fn typed_index_path<'a>(calls: &'a FunctionCallSites, mut cursor: OccurrenceOrdinal, ty: &ScalarType, dispatch_indices:bool) -> Option<Place<'a>> {
    use super::super::reference::NamedPlaceRootIdentity;
    let structure = &calls.structure;
    if structure.checked_types.places.get(&cursor)? != ty { return None; }
    let mut selectors = Vec::new();
    loop {
        let parent = OccurrenceParent::Place(cursor);
        let Some(index) = structure.edges.get(&(parent,OccurrenceEdge::PlaceIndex)) else { break; };
        let OccurrenceNode::Expression(index) = *index else { return None; };
        let OccurrenceNode::Place(base) = *structure.edges.get(&(parent,OccurrenceEdge::PlaceBase))? else { return None; };
        let ScalarType::Array { element,length,.. } = structure.checked_types.places.get(&base)? else { return None; };
        if !super::super::scalar_type_equal(element,structure.checked_types.places.get(&cursor)?)
            || structure.checked_types.expressions.get(&index)? != &ScalarType::U64
            || !(dispatch_indices || structure.prepared_range_literals.contains_key(&index)
                || matches!(calls.prepared_plain_name(index),Some(PlainNameIdentity::Lexical(_))))
        { return None; }
        selectors.push((index,*length));
        cursor = base;
    }
    if selectors.is_empty() { return None; }
    let NamedPlaceRootIdentity::ModuleValue(declaration) = calls.prepared_named_place_root(cursor)? else { return None; };
    Some(Place { declaration,site:cursor,selectors })
}

pub(super) fn expression(calls: &FunctionCallSites, at: OccurrenceOrdinal) -> Option<Place<'_>> {
    if calls.structure.checked_types.expressions.get(&at)? != &ScalarType::F32 { return None; }
    let OccurrenceNode::Place(destination) = calls.structure.edges.get(&(
        OccurrenceParent::Expression(at),OccurrenceEdge::ExpressionPlace))? else { return None; };
    typed_place(calls,*destination,&ScalarType::F32)
}

pub(super) fn assignment(calls: &FunctionCallSites, position: BlockPosition) -> Option<(Place<'_>,OccurrenceOrdinal)> {
    let group = calls.assignments.get(&(position.block,position.item))?;
    let [rhs] = group.rhs_evaluations().as_slice() else { return None; };
    let [destination] = group.destinations().as_slice() else { return None; };
    if group.projected_outputs().len() != 0 || !matches!(destination.replacement,AssignmentReplacement::Resolved(ScalarType::F32))
        || calls.structure.checked_types.expressions.get(&rhs.expression)? != &ScalarType::F32
    { return None; }
    Some((typed_place(calls,destination.destination,&ScalarType::F32)?,rhs.expression))
}
