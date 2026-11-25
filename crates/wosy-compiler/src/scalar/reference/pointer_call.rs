//! Authenticate a concrete caller occurrence before instantiating a point
//! from the already checked callee effect. The receipt ends at that call.
use super::*;
use super::pointer_entry::EntryPointeeWrite;

#[derive(Debug)]
#[allow(dead_code)] // The ordered caller continuation is a separate consumer.
pub(crate) struct CallerPointeeEffect<'a> {
    call: &'a SourceSpan,
    place: &'a ReferencePlaceId,
    loan: &'a ReferenceLoanId,
    width: &'a ScalarType,
    entry_epoch: CfgPointId,
    position: usize,
    entry: BigInt,
    exit: BigInt,
}

#[allow(dead_code)]
impl<'a> CallerPointeeEffect<'a> {
    pub(crate) fn call(&self) -> &'a SourceSpan { self.call }
    pub(crate) fn place(&self) -> &'a ReferencePlaceId { self.place }
    pub(crate) fn loan(&self) -> &'a ReferenceLoanId { self.loan }
    pub(crate) fn width(&self) -> &'a ScalarType { self.width }
    pub(crate) fn entry_epoch(&self) -> CfgPointId { self.entry_epoch }
    pub(crate) fn position(&self) -> usize { self.position }
    pub(crate) fn entry(&self) -> &BigInt { &self.entry }
    pub(crate) fn exit(&self) -> &BigInt { &self.exit }
}

pub(super) fn checked<'a>(
    source: &SourceIdentity, declaration: ByteSpan, function: &ScalarFunction,
    body: &'a FunctionCallSites, cfg: &'a ScalarReferenceCfg, facts: &CallablePointFacts,
    callee: EntryPointeeWrite<'a>, claimed: &BigInt, wanted: Option<OccurrenceOrdinal>,
) -> Option<CallerPointeeEffect<'a>> {
    if callee.offset() != claimed || &callee.formal().source != source || cfg.points.len() > 32
        || !(1..=4).contains(&cfg.facts.len()) || !cfg.allocations.is_empty() || function.body.unsafe_context
        || !function.generic_parameters.is_empty() || !body.arms.is_empty()
        || !body.boundary.parameters.is_empty() || !body.callable_escapes.is_empty()
        || !body.allocation_length_calls.is_empty() || !body.structure.module_storage_uses.is_empty()
    {
        return None;
    }
    let structure = &body.structure;
    let root = structure.root;
    struct Cell<'a> {
        binding: &'a ReferenceBindingId,
        entry: BigInt,
        observed: bool,
    }
    let width = callee.width();
    let (min, max) = super::super::typecheck::integer_bounds(width)?;
    let mut cells: Vec<Cell<'_>> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut points = Vec::new();
    let mut previous = CfgPointId(0);
    let mut position = 0usize;
    for item in structure.blocks.get(root.0)?.items.iter().take(11) {
        if let BlockItemKind::Binding(region) = &item.kind {
            let [receiver] = region.receivers() else { return None; };
            let BindingInitialization::Outputs(outputs) = &region.initialization else { return None; };
            if cells.len() == 4 || !outputs.secondary.is_empty() || receiver.contextual_storage.is_some()
                || &receiver.ty != width || structure.checked_types.expressions.get(&outputs.primary)? != width
            {
                return None;
            }
            let BindingDefinition::Initialized { bind, value } = *facts.definitions.get(&(root, item.position, 0))?
                else { return None; };
            if facts.evaluations.get(&outputs.primary) != Some(&value) || !(previous.0 < value.0 && value.0 < bind.0) {
                return None;
            }
            let CfgPointKind::Evaluate(ScalarExpression::Integer { value: entry, .. }) = &cfg.points.get(value.0)?.kind
                else { return None; };
            let CfgPointKind::Bind { binding } = &cfg.points.get(bind.0)?.kind else { return None; };
            if entry < &min || entry > &max || &binding.source != source || binding.function_span != function.span
                || binding.declaration_span != receiver.span || cells.iter().any(|cell| cell.binding == binding)
            {
                return None;
            }
            points.extend([value, bind]);
            previous = bind;
            cells.push(Cell { binding, entry: entry.clone(), observed: false });
            continue;
        }
        if let BlockItemKind::Assignment { group } = item.kind {
            // The initial escrow remains the literal declaration. A later
            // ordinary overwrite must replace, not refine, the old epoch.
            let (destination, ty, rhs, place) =
                super::super::checker_flow::checked_scalar_assignment(body, group)?;
            let cell = cells.iter_mut().find(|cell| cell.binding == destination && cell.observed)?;
            if ty != width || structure.checked_types.expressions.get(&rhs)? != width
                || structure.checked_types.places.get(&place)? != width
            {
                return None;
            }
            let literal = structure.checker_integer_literals.get(&rhs)?;
            if literal < &min || literal > &max { return None; }
            let rhs_point = *facts.evaluations.get(&rhs)?;
            let install = cfg.points.get(rhs_point.0.checked_add(1)?)?;
            if previous.0 >= rhs_point.0
                || !matches!(&cfg.points.get(rhs_point.0)?.kind,
                    CfgPointKind::Evaluate(ScalarExpression::Integer { value, .. }) if value == literal)
                || !matches!(&install.kind, CfgPointKind::Assign { target, rhs_point: exact, previous_origins, previous_loans }
                    if *exact == rhs_point && &target.binding == cell.binding && &target.source == source
                        && target.projections.is_empty() && previous_origins.is_empty() && previous_loans.is_empty())
                || install.source_span.range != body.occurrences.get(place.0)?.span
            {
                return None;
            }
            points.extend([rhs_point, install.id]);
            previous = install.id;
            cell.entry = literal.clone();
            continue;
        }
        let BlockItemKind::Evaluation { expression: call } = item.kind else { return None; };
        if position == 4 { return None; }
        let call_point = cfg.points.get(cfg.primary_call_point(call)?.0)?;
        if facts.evaluations.get(&call) != Some(&call_point.id) { return None; }
        let CfgPointKind::Call { expression, output: 0, target, argument_points } = &call_point.kind else { return None; };
        let ResolvedCallTarget::ModuleCallable { source: owner, declaration_span, concrete: ConcreteCallSelection::Declaration } = target
            else { return None; };
        let site = body.resolved_call_site(call)?;
        if owner != source || *declaration_span != declaration || &site.target != target || site.source_span != call_point.source_span {
            return None;
        }
        let signature = body.checked_call_signatures.get(&call)?;
        if signature.overload.is_some() || signature.overload_origin.is_some()
            || !matches!(signature.outputs.outputs.as_slice(), [output] if output.ty == ScalarType::Unit)
            || !matches!(signature.parameters.as_slice(),
                [ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner }] if inner.as_ref() == callee.width())
        {
            return None;
        }
        let ScalarExpression::Call { receiver: None, arguments, type_arguments, .. } = expression else { return None; };
        let [ScalarExpression::CheckedAddress {
            mutability: ScalarReferenceMutability::Mutable, place: ScalarPlace::Name { .. }, ..
        }] = arguments.as_slice() else { return None; };
        if !type_arguments.is_empty() { return None; }
        let OccurrenceNode::Expression(arg) = structure.edges.get(&(
            OccurrenceParent::Expression(call), OccurrenceEdge::CallArgument(0),
        ))? else { return None; };
        let [borrow] = argument_points.as_slice() else { return None; };
        if facts.evaluations.get(arg) != Some(borrow) || structure.checked_types.expressions.get(arg)? != &signature.parameters[0] {
            return None;
        }
        let borrow_point = cfg.points.get(borrow.0)?;
        let CfgPointKind::Borrow { place, loan } = &borrow_point.kind else { return None; };
        let origins = borrow_point.possible_origins.as_ref()?;
        if origins.len() != 1 { return None; }
        let origin = origins.iter().next()?;
        let ReferenceOrigin::Fresh { allocation, loan: issued } = cfg.facts.get(origin.0)? else { return None; };
        let cell = cells.iter_mut().find(|cell| cell.binding == &place.binding)?;
        if !seen.insert(*origin)
            || &allocation.source != source || allocation.function_span != function.span
            || allocation.declaration_span != cell.binding.declaration_span || allocation.creation_span != borrow_point.source_span.range
            || &place.source != source || !place.projections.is_empty()
            || &loan.origin_place != place || loan != issued || loan.parent.is_some()
            || loan.mode != ScalarReferenceMutability::Mutable
            || !(previous.0 < borrow.0 && borrow.0 < call_point.id.0)
        {
            return None;
        }
        let exit = super::super::typecheck::wrapping_integer_sum(&cell.entry, claimed, &min, &max)?;
        points.extend([*borrow, call_point.id]);
        previous = call_point.id;
        if wanted.is_some_and(|wanted| wanted != call) {
            cell.entry = exit;
            cell.observed = true;
            position += 1;
            continue;
        }
        // All state changes on the authenticated prefix have been paired
        // with an ordered source assignment or an exact verified call.
        for (index, point) in cfg.points.get(..=call_point.id.0)?.iter().enumerate() {
            if point.id.0 != index || &point.source_span.source != source
                || (index < call_point.id.0 && point.successors.as_slice() != [CfgPointId(index + 1)])
            {
                return None;
            }
            match &point.kind {
                CfgPointKind::Entry if index == 0 => {}
                CfgPointKind::ScopeEnter if point.scope == ReferenceScopeId(0) => {}
                CfgPointKind::Evaluate(_) | CfgPointKind::Bind { .. } | CfgPointKind::Borrow { .. }
                    | CfgPointKind::Call { .. } | CfgPointKind::Assign { .. } if points.contains(&point.id) => {}
                _ => return None,
            }
        }
        return Some(CallerPointeeEffect {
            call: &call_point.source_span, place, loan, width,
            entry_epoch: call_point.id, position, entry: cell.entry.clone(), exit,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analyze_scalar_build, derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project};

    #[test]
    fn instantiate_pointee_effect_at_distinct_concrete_caller_occurrences() {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r5".into());
        let program = r#"%%start
            unit(*!u8) increment = fn(p) { *p = *p +@ 1; };
            unit(*!u8) unrelated = fn(p) { *p = *p +@ 1; };
            unit() first = fn { u8 cell = 255; increment(&!cell); };
            unit() second = fn { u8 cell = 255; increment(&!cell); };
            unit() other = fn { u8 cell = 255; unrelated(&!cell); };
            unit() later = fn { u8 cell = 255; cell = 7; increment(&!cell); };
            unit() alias = fn { u8 cell = 255; *!u8 p = &!cell; increment(p); };
            first(); second();
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let parsed = parse_source(source.clone(), program.to_owned(), &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let prepared = prepare_scalar_project(
                vec![(derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())],
                vec![source.clone()], ScalarPreparationInputs { target_layout },
            ).unwrap();
            let analysis = analyze_scalar_build(&prepared);
            assert!(analysis.diagnostics().is_empty(), "{:?}", analysis.diagnostics());
            let apply = |caller, callee, claimed| analysis.pointer_caller_effect(0, caller, callee, &BigInt::from(claimed));
            let first = apply(2, 0, 1).expect("prepared first call");
            let second = apply(3, 0, 1).expect("prepared second call");
            assert_ne!(first.call(), second.call());
            assert_ne!(first.place().binding, second.place().binding);
            for receipt in [first, second] {
                assert_eq!(receipt.call().source, source);
                assert_eq!(receipt.place(), &receipt.loan().origin_place);
                assert_eq!(receipt.width(), &ScalarType::U8);
                assert_eq!(receipt.entry(), &BigInt::from(255));
                assert_eq!(receipt.exit(), &BigInt::from(0));
                assert!(receipt.entry_epoch().0 > 0);
                let range = receipt.call().range;
                assert_eq!(&program[range.start as usize..range.end as usize], "increment(&!cell)");
            }
            assert!(apply(2, 0, 2).is_none()); // unestablished authored amount
            assert!(apply(4, 0, 1).is_none()); // same signature/value, wrong canonical target
            assert!(apply(5, 0, 1).is_none()); // lexical write before invocation
            assert!(apply(6, 0, 1).is_none()); // indirect pointer binding is outside the receipt
        }
    }
}
