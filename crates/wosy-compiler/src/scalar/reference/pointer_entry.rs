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

pub(super) fn checked_effects<'a>(
    source: &SourceIdentity, function: &ScalarFunction, body: &'a FunctionCallSites,
    cfg: &'a ScalarReferenceCfg, facts: &'a CallablePointFacts,
) -> Option<Vec<EntryPointeeWrite<'a>>> {
    if !body.conditionals.is_empty() || body.structure.blocks.len() != 1 { return None; }
    let items = &body.structure.blocks.get(body.structure.root.0)?.items;
    if items.len() != body.assignments.len() || cfg.points.len() > 64 { return None; }
    let path = cfg.points.iter().map(|point| point.id).collect::<Vec<_>>();
    checked_path(source, function, body, cfg, facts, items, &path, None)
}

/// Exact prepared single Boolean conditional. Lane zero is false, lane one
/// true; both original CFG paths are validated, not just the claimed lane.
#[cfg(feature = "internal-checker")]
pub(super) fn checked_bool_effects<'a>(
    source: &SourceIdentity, function: &ScalarFunction, body: &'a FunctionCallSites,
    cfg: &'a ScalarReferenceCfg, facts: &'a CallablePointFacts,
) -> Option<[EntryPointeeWrite<'a>; 2]> {
    let structure = &body.structure;
    let [pointer, control] = body.boundary.parameters.as_slice() else { return None; };
    if !matches!(&pointer.ty, ScalarType::CheckedReference { .. }) || control.ty != ScalarType::Bool
        || cfg.points.len() > 64 || body.conditionals.len() != 1 || structure.blocks.len() != 3
        || body.short_circuits.len() != 0
    {
        return None;
    }
    let [item] = structure.blocks.get(structure.root.0)?.items.as_slice() else { return None; };
    let BlockItemKind::Evaluation { expression } = item.kind else { return None; };
    let ConditionalRegion::Value { condition, then_block, else_block, .. } = *body.conditionals.get(&expression)?
        else { return None; };
    let binding = facts.formals.get(&1)?;
    if control.ordinal != 1 || &binding.source != source || binding.function_span != function.span
        || structure.checked_types.expressions.get(&condition) != Some(&ScalarType::Bool)
        || !matches!(body.prepared_plain_name(condition), Some(PlainNameIdentity::Lexical(seen)) if seen == binding)
    {
        return None;
    }
    let condition_point = cfg.points.get(facts.evaluations.get(&condition)?.0)?;
    let [branch_id] = condition_point.successors.as_slice() else { return None; };
    let branch = cfg.points.get(branch_id.0)?;
    let CfgPointKind::Branch { condition: test @ ScalarExpression::Name { .. } } = &branch.kind else { return None; };
    if !matches!(&condition_point.kind, CfgPointKind::Read { binding: read } if read == binding)
        && !matches!(&condition_point.kind, CfgPointKind::Evaluate(expression) if expression == test)
    {
        return None;
    }
    // ReferenceFlowBuilder links then first and else second. Enforce that
    // correspondence independently by binding each arm's assignment RHS
    // occurrence to the points reached through its actual successor.
    let [when_true, when_false] = branch.successors.as_slice() else { return None; };
    if when_true == when_false { return None; }
    let follow = |branch_target| {
        let mut path = Vec::new();
        let mut current = CfgPointId(0);
        loop {
            if path.len() >= cfg.points.len() { return None; }
            path.push(current);
            let point = cfg.points.get(current.0)?;
            if point.id != current { return None; }
            current = if current == *branch_id { branch_target }
                else { match point.successors.as_slice() {
                    [] => break,
                    [next] => *next,
                    _ => return None,
                }};
        }
        Some(path)
    };
    let false_path = follow(*when_false)?;
    let true_path = follow(*when_true)?;
    let mut joins = cfg.points.iter().filter(|point| matches!(point.kind, CfgPointKind::Join));
    let join_id = joins.next()?.id;
    if joins.next().is_some() { return None; }
    if !false_path.contains(&join_id) || !true_path.contains(&join_id)
        || (0..cfg.points.len()).any(|index| !false_path.contains(&CfgPointId(index)) && !true_path.contains(&CfgPointId(index)))
    {
        return None;
    }
    let extras = (condition_point.id, *branch_id, join_id);
    let arm = |block: BlockOrdinal, path: &[CfgPointId]| {
        let items = &structure.blocks.get(block.0)?.items;
        checked_path(source, function, body, cfg, facts, items, path, Some(extras))?.pop()
    };
    let false_items = &structure.blocks.get(else_block.0)?.items;
    let true_items = &structure.blocks.get(then_block.0)?.items;
    if false_items.len().checked_add(true_items.len())? != body.assignments.len() { return None; }
    Some([arm(else_block, &false_path)?, arm(then_block, &true_path)?])
}

fn checked_path<'a>(
    source: &SourceIdentity, function: &ScalarFunction, body: &'a FunctionCallSites,
    cfg: &'a ScalarReferenceCfg, facts: &'a CallablePointFacts,
    items: &[BlockItem], path: &[CfgPointId], guard: Option<(CfgPointId, CfgPointId, CfgPointId)>,
) -> Option<Vec<EntryPointeeWrite<'a>>> {
    // Only successive straight-line integer load/modify/stores on exact formal loans are admitted.
    // Cap before inspecting the path; there is no new reference analysis.
    if cfg.points.len() > 64 || !cfg.allocations.is_empty()
        || function.body.unsafe_context || !function.generic_parameters.is_empty()
        || !body.arms.is_empty() || !body.call_occurrences.is_empty()
        || !body.callable_escapes.is_empty() || !body.allocation_length_calls.is_empty()
        || !body.short_circuits.is_empty() || !(1..=8).contains(&body.assignments.len())
        || !body.structure.module_storage_uses.is_empty()
        || !matches!(body.boundary.results.as_slice(), [DeclaredResultSlot { ordinal: 0, ty: ScalarType::Unit, .. }])
        || !body.boundary.root_outputs.is_empty()
    {
        return None;
    }
    struct Formal<'a> {
        binding: &'a ReferenceBindingId,
        loan: &'a ReferenceLoanId,
        origin: ReferenceOriginId,
        slot_type: &'a ScalarType,
        width: &'a ScalarType,
        bounds: (BigInt, BigInt),
        offset: BigInt,
        binding_point: Option<usize>,
        first_reader: Option<usize>,
        first_read: Option<&'a SourceSpan>,
        last_write: Option<SourceSpan>,
    }
    let parameters = &body.boundary.parameters;
    let loan_count = parameters.len().checked_sub(usize::from(guard.is_some()))?;
    if !(1..=2).contains(&loan_count) || facts.formals.len() != parameters.len()
        || cfg.facts.len() != loan_count
    {
        return None;
    }
    let mut formals: Vec<Formal<'_>> = Vec::with_capacity(parameters.len());
    for (index, slot) in parameters.iter().take(loan_count).enumerate() {
        let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner } = &slot.ty
            else { return None; };
        let formal = facts.formals.get(&index)?;
        if slot.ordinal != index || !is_integer_type(inner) || &formal.source != source
            || formal.function_span != function.span
            || formals.iter().any(|prior| prior.binding == formal)
        {
            return None;
        }
        let (origin, loan) = cfg.facts.iter().enumerate().find_map(|(origin, fact)| match fact {
            ReferenceOrigin::BorrowedFrom { parameter, loan } if *parameter == index =>
                Some((ReferenceOriginId(origin), loan)),
            _ => None,
        })?;
        if loan.mode != ScalarReferenceMutability::Mutable || loan.parent.is_some()
            || !loan.origin_place.projections.is_empty() || &loan.origin_place.binding != formal
            || &loan.origin_place.source != source || loan.creation_span != slot.span
            || formals.iter().any(|prior| prior.origin == origin || prior.loan == loan)
        {
            return None;
        }
        formals.push(Formal { binding: formal, loan, origin, slot_type: &slot.ty, width: inner,
            bounds: super::super::typecheck::integer_bounds(inner)?, offset: BigInt::from(0),
            binding_point: None, first_reader: None, first_read: None, last_write: None });
    }
    let structure = &body.structure;
    let writes = path.iter().map(|id| cfg.points.get(id.0)).collect::<Option<Vec<_>>>()?
        .into_iter().filter(|point| matches!(point.kind, CfgPointKind::AssignThrough { .. }))
        .collect::<Vec<_>>();
    if writes.len() != items.len() { return None; }
    let has_only_origin = |point: &ReferenceFlowPoint, origin| {
        point.possible_origins.as_ref().is_some_and(|origins| origins.len() == 1 && origins.contains(&origin))
    };
    let mut claimed = vec![false; cfg.points.len()];
    let mut previous_write = 0;

    // Zip genuine source assignments and ordered converged write points.
    // Write i consumes the current value after write i−1, not a second
    // snapshot; composing typed modular translations preserves the original entry epoch.
    for (item, write) in items.iter().zip(&writes) {
        let BlockItemKind::Assignment { group } = &item.kind else { return None; };
        let assignment = body.assignments.get(&(group.block, group.item))?;
        let [rhs] = assignment.rhs_evaluations().as_slice() else { return None; };
        let [destination] = assignment.destinations().as_slice() else { return None; };
        let expression = |parent, edge| match structure.edges.get(&(parent, edge))? {
            OccurrenceNode::Expression(ordinal) => Some(*ordinal),
            _ => None,
        };
        let parent = OccurrenceParent::Expression(rhs.expression);
        let read_ordinal = expression(parent, OccurrenceEdge::BinaryLeft)?;
        let offset_ordinal = expression(parent, OccurrenceEdge::BinaryRight)?;
        let OccurrenceNode::Place(read_place) = structure.edges.get(&(
            OccurrenceParent::Expression(read_ordinal), OccurrenceEdge::ExpressionPlace,
        ))? else { return None; };
        let pointer_for = |place| expression(OccurrenceParent::Place(place), OccurrenceEdge::DereferencePointer);
        let read_pointer = pointer_for(*read_place)?;
        let write_pointer = pointer_for(destination.destination)?;
        let PlainNameIdentity::Lexical(binding) = body.prepared_plain_name(write_pointer)? else { return None; };
        let effect = formals.iter_mut().find(|effect| effect.binding == binding)?;
        let (formal, loan, origin, width) = (effect.binding, effect.loan, effect.origin, effect.width);
        if rhs.position != 0 || destination.position != 0
            || structure.checked_types.places.get(&destination.destination)? != width
            || [rhs.expression, read_ordinal, offset_ordinal].into_iter()
                .any(|operand| structure.checked_types.expressions.get(&operand) != Some(width))
        {
            return None;
        }
        for pointer in [read_pointer, write_pointer] {
            if structure.checked_types.expressions.get(&pointer)? != effect.slot_type
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
            || !super::super::typecheck::integer_literal_fits_type(offset, width)
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
        if &read_target.binding != formal || &read_target.source != source || !has_only_origin(read, origin)
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
                || !has_only_origin(point, origin)
            {
                return None;
            }
        }
        for point in [*reader, *writer, read.id, literal.id, rhs.id, write.id] {
            if !path.contains(&point) || std::mem::replace(claimed.get_mut(point.0)?, true) { return None; }
        }
        effect.offset = super::super::typecheck::wrapping_integer_sum(&effect.offset, offset, &effect.bounds.0, &effect.bounds.1)?;
        effect.first_reader.get_or_insert(reader.0.min(writer.0));
        effect.first_read.get_or_insert(&read.source_span);
        effect.last_write = Some(SourceSpan::new(write.source_span.source.clone(), *write_place));
        previous_write = write.id.0;
    }
    let mut next_write = 0;
    let mut captured = false;
    for (index, id) in path.iter().copied().enumerate() {
        let point = cfg.points.get(id.0)?;
        let ordinal = id.0;
        let next = path.get(index + 1).copied();
        if point.id != id || &point.source_span.source != source { return None; }
        if guard.is_some_and(|(_, branch, _)| id == branch) {
            if !matches!(point.kind, CfgPointKind::Branch { .. }) || !next.is_some_and(|next| point.successors.contains(&next)) {
                return None;
            }
            continue;
        }
        if point.successors.iter().copied().ne(next) { return None; }
        if guard.is_some_and(|(condition, _, join)| id == condition || id == join) { continue; }
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
            CfgPointKind::Bind { binding } => {
                let effect = formals.iter_mut().find(|effect| effect.binding == binding)?;
                if !has_only_origin(point, effect.origin) || effect.binding_point.replace(ordinal).is_some() {
                    return None;
                }
            }
            CfgPointKind::CaptureAssignmentPrevious { writes: recorded } if !captured => {
                if recorded.as_slice() != [writes.get(next_write)?.id] { return None; }
                captured = true;
            }
            _ => return None,
        }
    }
    if !matches!(cfg.points.get(path.last()?.0)?, ReferenceFlowPoint { kind: CfgPointKind::ScopeExit, scope: ReferenceScopeId(0), .. }) {
        return None;
    }
    formals.into_iter().map(|effect| {
        if effect.binding_point? >= effect.first_reader? { return None; }
        Some(EntryPointeeWrite { formal: effect.binding, loan: effect.loan, width: effect.width, entry: CfgPointId(0),
            read: effect.first_read?, write: effect.last_write?, offset: effect.offset })
    }).collect()
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
