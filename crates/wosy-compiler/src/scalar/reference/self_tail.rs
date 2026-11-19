use super::*;

pub(super) fn select(
    source: &SourceIdentity,
    function: &ScalarFunction,
    calls: &FunctionCallSites,
) -> Vec<OccurrenceOrdinal> {
    let boundary = &calls.boundary;
    let [result] = boundary.results.as_slice() else {
        return Vec::new();
    };
    if calls.call_occurrences.is_empty()
        || !function.overload_arms.is_empty()
        || !boundary.generics.is_empty()
        || !primitive_scalar(&result.ty)
        || !boundary
            .parameters
            .iter()
            .all(|slot| primitive_scalar(&slot.ty))
    {
        return Vec::new();
    }

    let resolved: HashMap<_, _> = calls
        .call_occurrences
        .iter()
        .map(|row| (row.ordinal, &calls.sites[row.site].target))
        .collect();
    let mut pending = vec![calls.structure.root];
    let mut candidates = Vec::new();
    while let Some(block) = pending.pop() {
        let mut outputs = calls.structure.blocks[block.0].final_outputs();
        let (Some(slot), None) = (outputs.next(), outputs.next()) else {
            continue;
        };
        let producer = slot.expression;
        if let Some(ConditionalRegion::Value {
            then_block,
            else_block,
            ..
        }) = calls.conditionals.get(&producer)
        {
            pending.extend([*else_block, *then_block]);
        } else if let Some(OccurrenceNode::Block(body)) = calls.structure.edges.get(&(
            OccurrenceParent::Expression(producer),
            OccurrenceEdge::BlockBody,
        )) {
            pending.push(*body);
        } else if matches!(
            resolved.get(&producer),
            Some(ResolvedCallTarget::ModuleCallable {
                source: target_source,
                declaration_span,
                concrete: ConcreteCallSelection::Declaration,
            }) if target_source == source && *declaration_span == function.name_span
        ) {
            candidates.push(producer);
        }
    }
    candidates.sort_unstable_by_key(|ordinal| ordinal.0);
    candidates
}

fn primitive_scalar(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::Bool
            | ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128
            | ScalarType::F32
            | ScalarType::F64
            | ScalarType::Char
    )
}
