//! Internal proof receipt, issued only from validated reference-flow facts.
//! No pointer is loaded or evaluated here: the entry value is a typed symbolic
//! atom identified by its exclusive formal loan and call-entry epoch.
use super::*;

#[derive(Debug)]
#[allow(dead_code)] // Consumed by the Unit proof route, not by ordinary emission.
pub(crate) struct EntryPointeeWrite<'a> {
    formal: &'a ReferenceBindingId,
    loan: &'a ReferenceLoanId,
    width: &'a ScalarType,
    entry: CfgPointId,
    read: &'a SourceSpan,
    write: SourceSpan,
    offset: BigInt,
}

#[allow(dead_code)]
impl<'a> EntryPointeeWrite<'a> {
    pub(crate) fn formal(&self) -> &'a ReferenceBindingId { self.formal }
    pub(crate) fn loan(&self) -> &'a ReferenceLoanId { self.loan }
    pub(crate) fn width(&self) -> &'a ScalarType { self.width }
    pub(crate) fn entry(&self) -> CfgPointId { self.entry }
    pub(crate) fn read(&self) -> &'a SourceSpan { self.read }
    pub(crate) fn write(&self) -> &SourceSpan { &self.write }
    pub(crate) fn offset(&self) -> &BigInt { &self.offset }
}

pub(super) fn checked<'a>(
    source: &SourceIdentity, function: &ScalarFunction, body: &'a FunctionCallSites,
    cfg: &'a ScalarReferenceCfg, facts: &'a CallablePointFacts,
) -> Option<EntryPointeeWrite<'a>> {
    // Only successive straight-line integer load/modify/stores on one loan are admitted.
    // Cap before inspecting the path; there is no new reference analysis.
    if cfg.points.len() > 64 || cfg.facts.len() != 1 || !cfg.allocations.is_empty()
        || function.body.unsafe_context || !function.generic_parameters.is_empty()
        || !body.arms.is_empty() || !body.call_occurrences.is_empty()
        || !body.callable_escapes.is_empty() || !body.allocation_length_calls.is_empty()
        || !body.conditionals.is_empty() || !body.short_circuits.is_empty()
        || !(1..=8).contains(&body.assignments.len()) || body.structure.blocks.len() != 1
        || !body.structure.module_storage_uses.is_empty()
        || !matches!(body.boundary.results.as_slice(), [DeclaredResultSlot { ordinal: 0, ty: ScalarType::Unit, .. }])
        || !body.boundary.root_outputs.is_empty()
    {
        return None;
    }
    let [slot] = body.boundary.parameters.as_slice() else { return None; };
    let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner } = &slot.ty
        else { return None; };
    if slot.ordinal != 0 || !is_integer_type(inner) || facts.formals.len() != 1 { return None; }
    let formal = facts.formals.get(&0)?;
    if &formal.source != source || formal.function_span != function.span { return None; }
    let [ReferenceOrigin::BorrowedFrom { parameter: 0, loan }] = cfg.facts.as_slice() else { return None; };
    if loan.mode != ScalarReferenceMutability::Mutable || loan.parent.is_some()
        || !loan.origin_place.projections.is_empty() || &loan.origin_place.binding != formal
        || &loan.origin_place.source != source || loan.creation_span != slot.span
    {
        return None;
    }
    let structure = &body.structure;
    let items = &structure.blocks.get(structure.root.0)?.items;
    if items.len() != body.assignments.len() { return None; }
    let writes = cfg.points.iter().filter(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
        .collect::<Vec<_>>();
    if writes.len() != items.len() { return None; }
    let origin = ReferenceOriginId(0);
    let has_only_origin = |point: &ReferenceFlowPoint| {
        point.possible_origins.as_ref().is_some_and(|origins| origins.len() == 1 && origins.contains(&origin))
    };
    let bounds = super::super::typecheck::integer_bounds(inner)?;
    let mut offset_sum = BigInt::from(0);
    let mut claimed = vec![false; cfg.points.len()];
    let mut previous_write = 0;
    let mut first_reader = None;
    let mut first_read = None;
    let mut last_write = None;

    // Zip genuine source assignments and ordered converged write points.
    // Write i consumes the current value after write i−1, not a second
    // snapshot; composing typed modular translations preserves the original entry epoch.
    for (item, write) in items.iter().zip(&writes) {
        let BlockItemKind::Assignment { group } = &item.kind else { return None; };
        let assignment = body.assignments.get(&(group.block, group.item))?;
        let [rhs] = assignment.rhs_evaluations().as_slice() else { return None; };
        let [destination] = assignment.destinations().as_slice() else { return None; };
        if rhs.position != 0 || destination.position != 0
            || structure.checked_types.places.get(&destination.destination)? != inner.as_ref()
            || structure.checked_types.expressions.get(&rhs.expression)? != inner.as_ref()
        {
            return None;
        }
        let expression = |parent, edge| match structure.edges.get(&(parent, edge))? {
            OccurrenceNode::Expression(ordinal) => Some(*ordinal),
            _ => None,
        };
        let parent = OccurrenceParent::Expression(rhs.expression);
        let read_ordinal = expression(parent, OccurrenceEdge::BinaryLeft)?;
        let offset_ordinal = expression(parent, OccurrenceEdge::BinaryRight)?;
        if structure.checked_types.expressions.get(&read_ordinal)? != inner.as_ref()
            || structure.checked_types.expressions.get(&offset_ordinal)? != inner.as_ref()
        {
            return None;
        }
        let OccurrenceNode::Place(read_place) = structure.edges.get(&(
            OccurrenceParent::Expression(read_ordinal), OccurrenceEdge::ExpressionPlace,
        ))? else { return None; };
        let pointer_for = |place| expression(OccurrenceParent::Place(place), OccurrenceEdge::DereferencePointer);
        let read_pointer = pointer_for(*read_place)?;
        let write_pointer = pointer_for(destination.destination)?;
        for pointer in [read_pointer, write_pointer] {
            if structure.checked_types.expressions.get(&pointer)? != &slot.ty
                || body.prepared_plain_name(pointer) != Some(&PlainNameIdentity::Lexical(formal.clone()))
            {
                return None;
            }
        }
        let read = cfg.points.get(facts.evaluations.get(&read_ordinal)?.0)?;
        let rhs = cfg.points.get(facts.evaluations.get(&rhs.expression)?.0)?;
        let literal = cfg.points.get(facts.evaluations.get(&offset_ordinal)?.0)?;
        let CfgPointKind::Evaluate(ScalarExpression::Binary {
            operator: BinaryOperator::WrappingAdd, left, right, ..
        }) = &rhs.kind else { return None; };
        let ScalarExpression::Dereference { place: ScalarPlace::Dereference { .. }, .. } = left.as_ref()
            else { return None; };
        let ScalarExpression::Integer { value: offset, .. } = right.as_ref() else { return None; };
        if !matches!(&literal.kind, CfgPointKind::Evaluate(expression) if expression == right.as_ref())
            || !super::super::typecheck::integer_literal_fits_type(offset, inner)
        {
            return None;
        }
        let CfgPointKind::ReadPlace {
            place: ScalarPlace::Dereference { .. }, pointer_point: Some(reader), candidates, reads_origin: false,
        } = &read.kind else { return None; };
        if facts.evaluations.get(&read_pointer) != Some(reader) { return None; }
        let [read_target] = candidates.as_slice() else { return None; };
        // ReadPlace carries the explicit dereference projection; AssignThrough
        // carries its base place plus the dereferenced loan separately.
        if &read_target.binding != formal || &read_target.source != source || !has_only_origin(read)
            || !matches!(read_target.projections.as_slice(),
                [ReferencePlaceProjection::Dereference(parent)] if parent.as_ref() == loan)
        {
            return None;
        }
        let CfgPointKind::AssignThrough {
            place: ScalarPlace::Dereference { span: write_place, .. }, pointer_point: writer, rhs_point, candidates,
            previous_origins, previous_loans,
        } = &write.kind else { return None; };
        let [candidate] = candidates.as_slice() else { return None; };
        if *rhs_point != rhs.id || !previous_origins.is_empty() || !previous_loans.is_empty()
            || candidate.origin != origin || &candidate.loan != loan
            || !reference_places_same(&candidate.place, &loan.origin_place)
            || candidate.place.source != *source
            || !(previous_write < reader.0 && previous_write < writer.0 && reader.0 < read.id.0
                && read.id.0 < literal.id.0 && literal.id.0 < rhs.id.0 && rhs.id.0 < write.id.0 && writer.0 < write.id.0)
            || facts.evaluations.get(&write_pointer) != Some(writer)
            || write.source_span.range != destination.target_span
            || body.occurrences.get(destination.destination.0)?.span != *write_place
            || body.occurrences.get(read_ordinal.0)?.span != read.source_span.range
        {
            return None;
        }
        for point in [reader, writer] {
            let point = cfg.points.get(point.0)?;
            if !matches!(&point.kind, CfgPointKind::Read { binding } if binding == formal)
                || !has_only_origin(point)
            {
                return None;
            }
        }
        for point in [*reader, *writer, read.id, literal.id, rhs.id, write.id] {
            if std::mem::replace(claimed.get_mut(point.0)?, true) { return None; }
        }
        offset_sum = super::super::typecheck::wrapping_integer_sum(&offset_sum, offset, &bounds.0, &bounds.1)?;
        first_reader.get_or_insert(reader.0.min(writer.0));
        first_read.get_or_insert(&read.source_span);
        last_write = Some(SourceSpan::new(write.source_span.source.clone(), *write_place));
        previous_write = write.id.0;
    }
    let mut bind = None;
    let mut next_write = 0;
    let mut captured = false;
    for (ordinal, point) in cfg.points.iter().enumerate() {
        let next = cfg.points.get(ordinal + 1).map(|point| point.id);
        if point.id.0 != ordinal || &point.source_span.source != source
            || point.successors.iter().copied().ne(next)
        {
            return None;
        }
        if claimed[ordinal] {
            if matches!(point.kind, CfgPointKind::AssignThrough { .. }) {
                next_write += 1;
                captured = false;
            }
            continue;
        }
        match &point.kind {
            CfgPointKind::Entry if ordinal == 0 => {}
            CfgPointKind::ScopeEnter | CfgPointKind::ScopeExit => {}
            CfgPointKind::Bind { binding } if binding == formal && has_only_origin(point) && bind.is_none() => {
                bind = Some(ordinal);
            }
            CfgPointKind::CaptureAssignmentPrevious { writes: recorded } if !captured => {
                if recorded.as_slice() != [writes.get(next_write)?.id] { return None; }
                captured = true;
            }
            _ => return None,
        }
    }
    if !matches!(cfg.points.last()?, ReferenceFlowPoint { kind: CfgPointKind::ScopeExit, scope: ReferenceScopeId(0), .. })
        || bind? >= first_reader?
    {
        return None;
    }
    Some(EntryPointeeWrite { formal, loan, width: inner, entry: CfgPointId(0),
        read: first_read?, write: last_write?, offset: offset_sum })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analyze_scalar_build, derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project};

    #[test]
    fn wrapping_pointee_entry_receipt_comes_from_the_exclusive_formal_loan() {
        let check = |signature: &str, code: &str, offset: Option<u8>| {
            let text = format!("%%start unit({signature}) step = fn(p) {{ {code} }}; %%end");
            let source = SourceIdentity::new("project".into(), "package".into(), "src/pointer.w".into(), "rev7".into());
            let parsed = parse_source(source.clone(), text.clone(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
                let prepared = prepare_scalar_project(
                    vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
                    vec![source.clone()], ScalarPreparationInputs { target_layout },
                ).unwrap();
                let validated = analyze_scalar_build(&prepared);
                let receipt = validated.pointer_entry_wrapping_write(0, 0);
                if let Some(offset) = offset {
                    assert!(validated.diagnostics().is_empty(), "{:?}", validated.diagnostics());
                    let receipt = receipt.expect("the prepared exact projected read/write must be authenticated");
                    assert_eq!(receipt.width(), &ScalarType::U32);
                    assert_eq!(receipt.offset(), &BigInt::from(offset));
                    assert_eq!(receipt.entry(), CfgPointId(0));
                    assert_eq!(receipt.formal().source, source);
                    assert_eq!(receipt.formal(), &receipt.loan().origin_place.binding);
                    assert_eq!(receipt.read().source, source);
                    assert_eq!(receipt.write().source, source);
                    let snippet = |span: &SourceSpan| &text[span.range.start as usize..span.range.end as usize];
                    assert_eq!(snippet(receipt.read()), "*p");
                    assert_eq!(snippet(receipt.write()), "*p");
                    assert_ne!(receipt.read(), receipt.write());
                } else {
                    assert!(receipt.is_none(), "{signature}: {code}");
                }
            }
        };
        check("*!u32", "*p = *p +@ 1;", Some(1));
        check("*!u32", "*p = *p +@ 7;", Some(7));
        check("*!u32", "*p = *p + 1;", None);
        check("*!u32", "*p = *p +@ 1; *p = *p +@ 2;", Some(3));
        check("*!u32", "*p = *p +@ 4294967295; *p = *p +@ 2;", Some(1));
        check("*!u32", "*p = *p +@ 1; *p = *p + 2;", None);
        check("*!u32", "if (*p == 0) { *p = *p +@ 1; };", None);
        check("*u32", "if (*p == 0) { };", None);
    }
}
