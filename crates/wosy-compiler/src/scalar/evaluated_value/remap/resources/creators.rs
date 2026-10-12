use super::*;

fn held(relations: &[CallableHolderRelation], environment: &CallableEnvironmentId, holder: &CallableHolderPlace) -> bool {
    relations.iter().any(|relation| relation.environment == *environment && relation.holders.contains(holder))
}

fn environment<'a>(cfg: &'a ScalarReferenceCfg, id: &CallableEnvironmentId) -> Option<&'a CallableEnvironmentFact> {
    cfg.environments.get(id.index).filter(|fact| fact.literal == id.literal)
}

pub(super) fn return_transfer_connected(cfg: &ScalarReferenceCfg, returned: &ProducerReturnCaseFact) -> bool {
    if returned.table.owner != cfg.producer_owner { return false; }
    let points = cfg.points.iter().filter(|point| point.id == returned.return_point).collect::<Vec<_>>();
    let [point] = points.as_slice() else { return false; };
    if !matches!(&point.kind, CfgPointKind::ReturnOutput { output, .. } if *output == returned.output) { return false; }
    let CallableHolderAction::ReturnTransfer { environment, from, to, .. } = &returned.transfer else { return false; };
    if environment != &returned.environment || from != &returned.holder || *to != CallableHolderPlace::Output(returned.output) { return false; }
    cfg.holder_edges.iter().map(|edge| (edge.from, &edge.actions, &edge.before, &edge.after))
        .chain(cfg.holder_exits.iter().map(|exit| (exit.from, &exit.actions, &exit.before, &exit.after)))
        .any(|(point, actions, before, after)| point == returned.return_point && actions.contains(&returned.transfer)
            && held(before, environment, from) && held(after, environment, to))
}

fn link(cfg: &ScalarReferenceCfg, producer: &HolderProducerRef) -> bool {
    match producer {
        HolderProducerRef::CaptureCreation { point, environment: id } => {
            environment(cfg, id).is_some_and(|fact| fact.instance == CallableEnvironmentInstance::Capture)
                && cfg.points.iter().any(|known| known.id == *point && matches!(&known.kind, CfgPointKind::Capture { environment, .. } if environment == id))
                && cfg.holder_edges.iter().any(|edge| edge.from == *point && held(&edge.after, id, &CallableHolderPlace::Temporary(*point)))
        }
        HolderProducerRef::CallCreation { point, environment: id, selection } => {
            environment(cfg, id).is_some_and(|fact| cfg.environment_imports.iter().any(|entry| entry.table == selection.source.table
                && entry.source_index == selection.source.alternative.source_index
                && entry.source_instance == selection.source.alternative.source_instance
                && entry.destination == *id && entry.destination_instance == fact.instance))
                && selection.call == *point && selection.destination == *id
                && cfg.points.iter().any(|known| known.id == *point
                    && matches!(&known.kind, CfgPointKind::Call { target, output, .. } if *target == selection.target && *output == selection.output)
                    && known.call_import_paths.iter().flatten().any(|row| row == selection))
                && cfg.holder_edges.iter().any(|edge| edge.from == *point && edge.after.iter().any(|relation| relation.environment == *id
                    && relation.holders.iter().any(|holder| match holder {
                        CallableHolderPlace::Temporary(created) => created == point,
                        CallableHolderPlace::Projection(place) => matches!(&place.binding.kind, ReferenceBindingKind::CallOutput { point: created, .. } if created == point) && place.projections == selection.projection,
                        CallableHolderPlace::Binding(_) | CallableHolderPlace::Output(_) | CallableHolderPlace::OutputProjection { .. } => false,
                    })))
        }
        HolderProducerRef::InstalledRead { point, environment: id, action } => {
            let CallableHolderAction::RetainCheckedSelected { to, .. } = action else { return false; };
            environment(cfg, id).is_some() && cfg.holder_edges.iter().any(|edge| edge.from == *point && edge.actions.contains(action) && held(&edge.after, id, to))
        }
        HolderProducerRef::GlobalCallsiteInput { call, holder, environment: id } => {
            environment(cfg, id).is_some() && cfg.global_callable_input_paths.iter().any(|path| path.call == *call
                && path.holders.iter().any(|fact| fact.holder == *holder && held(&fact.relations, id, holder)))
        }
        HolderProducerRef::Forward { point, environment: id, action, .. } => {
            environment(cfg, id).is_some() && cfg.holder_edges.iter().any(|edge| {
                if edge.from != *point || !edge.actions.contains(action) { return false; }
                match action {
                    CallableHolderAction::Retain { from, to, .. } | CallableHolderAction::Move { from, to, .. } => held(&edge.before, id, from) && held(&edge.after, id, to),
                    CallableHolderAction::RetainSelected { from, to, .. } | CallableHolderAction::MoveSelected { from, to, .. } => held(&edge.before, id, from)
                        && edge.after.iter().any(|relation| relation.environment == *id && relation.holders.iter().any(|holder| matches!(holder, CallableHolderPlace::Projection(place) if place == &to.place))),
                    _ => false,
                }
            })
        }
    }
}

fn known_foreign(cfg: &ScalarReferenceCfg, producer: &HolderProducerRef) -> bool {
    cfg.capture_input_path_facts.iter().filter(|fact| fact.owner != cfg.producer_owner)
        .flat_map(|fact| &fact.inputs).any(|input| match &input.origin {
            CaptureInputOriginFact::Value => false,
            CaptureInputOriginFact::Callable { holders } => holders.iter().any(|holder| {
                let mut current = &holder.producer;
                loop {
                    if current == producer { return true; }
                    match current {
                        HolderProducerRef::Forward { previous, .. } => current = previous,
                        _ => return false,
                    }
                }
            }),
        })
}

pub(super) fn constructor<'a>(owner: &ProducerCfgOwnerId, producer: &HolderProducerRef, forest: &'a SourceCaptureConstructionForest) -> Result<&'a ValueTableSource, ValueSchemaRemapError> {
    let invalid = || ValueSchemaRemapError::MissingProducer(owner.clone());
    let rows = forest.sources.iter().filter(|row| row.producer == *owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(invalid()); };
    let cfg = row.resource_construction.original();
    if link(cfg, producer) { return Ok(row); }
    if !known_foreign(cfg, producer) { return Err(invalid()); }
    let candidates = forest.sources.iter().filter(|candidate| link(candidate.resource_construction.original(), producer)).collect::<Vec<_>>();
    let [creator] = candidates.as_slice() else { return Err(invalid()); };
    Ok(creator)
}

pub(super) fn dependencies(owner: &ProducerCfgOwnerId, holder: &ConnectedCaptureHolderFact, forest: &SourceCaptureConstructionForest) -> Result<Vec<BoundaryResourceRef>, ValueSchemaRemapError> {
    let mut references = Vec::new();
    let mut current_owner = owner.clone();
    let mut current = &holder.producer;
    loop {
        let actual = constructor(&current_owner, current, forest).map_err(|_| ValueSchemaRemapError::MissingHolder(holder.clone()))?;
        references.extend(children::producer_references(&actual.producer, current));
        match current {
            HolderProducerRef::Forward { previous, .. } => { current_owner = actual.producer.clone(); current = previous; }
            HolderProducerRef::CallCreation { selection, .. } => {
                let makers = forest.sources.iter().filter(|row| row.producer == selection.source.table.owner).collect::<Vec<_>>();
                let [maker] = makers.as_slice() else { return Err(ValueSchemaRemapError::MissingHolder(holder.clone())); };
                let returns = maker.resource_construction.original().producer_return_cases.iter().filter(|row| row.table == selection.source.table
                    && row.return_point == selection.source.return_point && row.output == selection.source.output && row.alternative == selection.source.alternative).collect::<Vec<_>>();
                let [returned] = returns.as_slice() else { return Err(ValueSchemaRemapError::MissingHolder(holder.clone())); };
                references.push(BoundaryResourceRef { owner: maker.producer.clone(), key: BoundaryResourceKey::Holder(ConnectedCaptureHolderFact {
                    holder: returned.holder.clone(), environment: returned.environment.clone(), producer: returned.producer.clone(),
                }) });
                break;
            }
            HolderProducerRef::CaptureCreation { .. } | HolderProducerRef::InstalledRead { .. } | HolderProducerRef::GlobalCallsiteInput { .. } => break,
        }
    }
    Ok(references)
}

pub(super) fn runtime_owner<'a>(owner: &'a ProducerCfgOwnerId, runtime: &RuntimeValueId, forest: &'a SourceCaptureConstructionForest) -> Result<&'a ProducerCfgOwnerId, ValueSchemaRemapError> {
    let matches = |row: &ValueTableSource| {
        if copied::lexical(&row.producer) != &runtime.source { return false; }
        match &row.producer {
            ProducerCfgOwnerId::Module(_) => runtime.function_span == ByteSpan::new(0, 0),
            ProducerCfgOwnerId::Literal { literal, .. } => literal.range == runtime.function_span,
            ProducerCfgOwnerId::Function { declaration, body, .. } => {
                *declaration == runtime.function_span || row.resource_construction.original().backing_output_contracts.iter().any(|contract| {
                    contract.source == runtime.source && contract.declaration_span == *declaration
                        && matches!((body, &contract.selection), (ProducerFunctionBodyId::Declaration, RuntimeCallSelection::Declaration) | (ProducerFunctionBodyId::Overload(_), RuntimeCallSelection::Overload(_)))
                        && match (body, &contract.selection) { (ProducerFunctionBodyId::Overload(left), RuntimeCallSelection::Overload(right)) => left == right, _ => true }
                        && contract.sources.iter().any(|source| matches!(source, RuntimeOutputBackingSource::Created { lineage, .. } if lineage == runtime))
                })
            }
        }
    };
    let local = forest.sources.iter().filter(|row| row.producer == *owner && matches(row)).collect::<Vec<_>>();
    if local.len() == 1 { return Ok(owner); }
    let creators = forest.sources.iter().filter(|row| matches(row)).collect::<Vec<_>>();
    let [creator] = creators.as_slice() else { return Err(ValueSchemaRemapError::MissingRuntime(runtime.clone())); };
    Ok(&creator.producer)
}

pub(super) fn qualify_runtime(references: &mut [BoundaryResourceRef], forest: &SourceCaptureConstructionForest) -> Result<(), ValueSchemaRemapError> {
    for reference in references {
        if let BoundaryResourceKey::Runtime(runtime) = &reference.key { reference.owner = runtime_owner(&reference.owner, runtime, forest)?.clone(); }
    }
    Ok(())
}

pub(super) fn validate(
    owner: &ProducerCfgOwnerId,
    holder: &ConnectedCaptureHolderFact,
    forest: &SourceCaptureConstructionForest,
) -> Result<Vec<BoundaryResourceConstructionEvidence>, ValueSchemaRemapError> {
    let invalid = || ValueSchemaRemapError::MissingHolder(holder.clone());
    let mut pending = vec![(owner.clone(), holder.producer.clone())];
    let mut visited = Vec::new();
    let mut originals = Vec::new();
    while let Some((requested_owner, producer)) = pending.pop() {
        if visited.contains(&(requested_owner.clone(), producer.clone())) { continue; }
        visited.push((requested_owner.clone(), producer.clone()));
        let actual = constructor(&requested_owner, &producer, forest).map_err(|_| invalid())?;
        if !originals.contains(&actual.resource_construction) { originals.push(actual.resource_construction.clone()); }
        match &producer {
            HolderProducerRef::Forward { previous, .. } => pending.push((actual.producer.clone(), *previous.clone())),
            HolderProducerRef::CallCreation { environment: imported, selection, .. } => {
                let makers = forest.sources.iter().filter(|candidate| candidate.producer == selection.source.table.owner).collect::<Vec<_>>();
                let [maker] = makers.as_slice() else { return Err(invalid()); };
                let original = maker.resource_construction.original();
                let returns = original.producer_return_cases.iter().filter(|fact| fact.table == selection.source.table
                    && fact.return_point == selection.source.return_point && fact.output == selection.source.output
                    && fact.alternative == selection.source.alternative).collect::<Vec<_>>();
                let [returned] = returns.as_slice() else { return Err(invalid()); };
                let Some(created) = environment(original, &returned.environment) else { return Err(invalid()); };
                if returned.environment.index != returned.alternative.source_index || created.instance != returned.alternative.source_instance
                    || !return_transfer_connected(original, returned)
                    || environment(actual.resource_construction.original(), imported).is_none() {
                    return Err(invalid());
                }
                if !originals.contains(&maker.resource_construction) { originals.push(maker.resource_construction.clone()); }
                pending.push((maker.producer.clone(), returned.producer.clone()));
            }
            HolderProducerRef::CaptureCreation { .. } | HolderProducerRef::InstalledRead { .. } | HolderProducerRef::GlobalCallsiteInput { .. } => {}
        }
    }
    Ok(originals)
}
