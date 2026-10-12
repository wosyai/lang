use super::*;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar::evaluated_value::remap) enum OriginalCalleePath {
    Captured { environment: CallableEnvironmentId, fact: CallableEnvironmentFact, path: CaptureInputPathFact, holder: ConnectedCaptureHolderFact, originals: Vec<BoundaryResourceConstructionEvidence>, dependencies: Vec<BoundaryResourceRef>, terminal: Vec<(SourceEventId, ScalarType)> },
    Capture { owner: ProducerCfgOwnerId, environment: CallableEnvironmentId, fact: CallableEnvironmentFact, point: ReferenceFlowPoint },
    Imported { owner: ProducerCfgOwnerId, environment: CallableEnvironmentId, fact: CallableEnvironmentFact, import: EnvironmentImportFact, call: ReferenceFlowPoint, selection_path: Vec<ProducerCallSelectionFact>, returned: ProducerReturnCaseFact, originals: Vec<BoundaryResourceConstructionEvidence>, terminal: Vec<(SourceEventId, ScalarType)> },
    Parameter { owner: ProducerCfgOwnerId, declaration: ProducerCfgOwnerId, slot: SourceSlotFact, shape: ResolvedValueShape },
}

fn original<'a>(forest: &'a SourceCaptureConstructionForest, owner: &ProducerCfgOwnerId) -> Result<&'a ScalarReferenceCfg, ValueSchemaRemapError> {
    let rows = forest.sources.iter().filter(|row| row.producer == *owner && row.resource_construction.original().producer_owner == *owner).collect::<Vec<_>>();
    let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(owner.clone())); };
    Ok(row.resource_construction.original())
}

pub(in crate::scalar::evaluated_value::remap) fn paths(forest: &SourceCaptureConstructionForest, cfg: &ScalarReferenceCfg, call: &ReferenceFlowPoint, target: &ResolvedCallTarget) -> Result<Vec<OriginalCalleePath>, ValueSchemaRemapError> {
    let invalid = || ValueSchemaRemapError::MissingPoint { producer: cfg.producer_owner.clone(), point: call.id };
    let current = cfg.points.iter().filter(|point| point.id == call.id && point.kind == call.kind).collect::<Vec<_>>();
    let [current] = current.as_slice() else { return Err(invalid()); };
    let binding = match target { ResolvedCallTarget::LocalCallable(binding) => binding, ResolvedCallTarget::CallableField { receiver_place, .. } => &receiver_place.binding, _ => return Err(invalid()) };
    let mut paths = Vec::new();
    if current.possible_callee_environments.is_empty() {
        let mut declaration = cfg.producer_owner.clone();
        loop {
            let parameter = original(forest, &declaration)?;
            if parameter.reference_parameters.contains(binding) { break; }
            let ProducerCfgOwnerId::Literal { enclosing, capture_point, .. } = &declaration else { return Err(invalid()); };
            let parent = original(forest, enclosing)?;
            if parent.literal_cfgs.iter().filter(|child| child.capture_point == *capture_point && child.reference_cfg.producer_owner == declaration).count() != 1 { return Err(invalid()); }
            declaration = (**enclosing).clone();
        }
        for slot in &cfg.evaluated_values.source_slots {
            if !matches!(&slot.role, SourceCoordinateRole::Boundary(BoundaryPortRole::Parameter { binding: actual, .. } | BoundaryPortRole::Capture { binding: actual, .. }) if actual == binding) { continue; }
            let shapes = cfg.evaluated_values.shapes.iter().filter(|shape| shape.id == slot.shape).collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(slot.shape.clone())); };
            paths.push(OriginalCalleePath::Parameter { owner: cfg.producer_owner.clone(), declaration: declaration.clone(), slot: slot.clone(), shape: (*shape).clone() });
        }
        if paths.len() != 1 { return Err(invalid()); }
        return Ok(paths);
    }
    for environment in &current.possible_callee_environments {
        let Some(fact) = cfg.environments.get(environment.index) else { return Err(invalid()); };
        if fact.literal != environment.literal { return Err(invalid()); }
        let start = paths.len();
        for path in &cfg.capture_input_path_facts {
            let parent = original(forest, &path.owner)?;
            if !parent.capture_input_path_facts.contains(path) { return Err(invalid()); }
            for input in &path.inputs {
                if input.binding != *binding { continue; }
                let CaptureInputOriginFact::Callable { holders } = &input.origin else { continue; };
                for holder in holders {
                    if holder.environment != *environment { continue; }
                    let Some(parent_fact) = parent.environments.get(environment.index) else { return Err(invalid()); };
                    if parent_fact.literal != fact.literal || parent_fact.instance != fact.instance { return Err(invalid()); }
                    let parent_imports = parent.environment_imports.iter().filter(|import| import.destination == *environment).collect::<Vec<_>>();
                    let inherited_imports = cfg.environment_imports.iter().filter(|import| import.destination == *environment).collect::<Vec<_>>();
                    if parent_imports != inherited_imports { return Err(invalid()); }
                    let originals = creators::validate(&path.owner, holder, forest)?;
                    let dependencies = creators::dependencies(&path.owner, holder, forest)?;
                    let terminal = terminal(forest, &path.owner, &holder.producer)?;
                    paths.push(OriginalCalleePath::Captured { environment: environment.clone(), fact: fact.clone(), path: path.clone(), holder: holder.clone(), originals, dependencies, terminal });
                }
            }
        }
        if paths.len() != start { continue; }
        match fact.instance {
            CallableEnvironmentInstance::Capture => {
                let points = cfg.initial_points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Capture { environment: actual, .. } if actual == environment)).collect::<Vec<_>>();
                let [point] = points.as_slice() else { return Err(invalid()); };
                if !cfg.holder_edges.iter().any(|edge| edge.from == point.id && edge.after.iter().any(|relation| relation.environment == *environment && relation.holders.contains(&CallableHolderPlace::Temporary(point.id)))) { return Err(invalid()); }
                paths.push(OriginalCalleePath::Capture { owner: cfg.producer_owner.clone(), environment: environment.clone(), fact: fact.clone(), point: (*point).clone() });
            }
            CallableEnvironmentInstance::CallOutput { .. } => {
                let imports = cfg.environment_imports.iter().filter(|import| import.destination == *environment && import.destination_instance == fact.instance).collect::<Vec<_>>();
                let [import] = imports.as_slice() else { return Err(invalid()); };
                let maker = original(forest, &import.table.owner)?;
                let Some(source) = maker.environments.get(import.source_index) else { return Err(invalid()); };
                if source.literal != environment.literal || source.instance != import.source_instance { return Err(invalid()); }
                for point in &cfg.points {
                    for selection_path in &point.call_import_paths {
                        for selection in selection_path {
                            if selection.destination != *environment || selection.source.table != import.table || selection.source.alternative.source_index != import.source_index || selection.source.alternative.source_instance != import.source_instance { continue; }
                            if selection.call != point.id || !matches!(&point.kind, CfgPointKind::Call { target, output, .. } if *target == selection.target && *output == selection.output) { return Err(invalid()); }
                            if let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } = &selection.target {
                                if !matches!(&selection.source.table.owner, ProducerCfgOwnerId::Function { source: actual, declaration, body } if actual == source && declaration == declaration_span && match (body, concrete) { (ProducerFunctionBodyId::Declaration, ConcreteCallSelection::Declaration) => true, (ProducerFunctionBodyId::Overload(a), ConcreteCallSelection::Overload(b)) => *a == b.arm_index, _ => false }) { return Err(invalid()); }
                            }
                            let returns = maker.producer_return_cases.iter().filter(|returned| returned.table == selection.source.table && returned.return_point == selection.source.return_point && returned.output == selection.source.output && returned.alternative == selection.source.alternative).collect::<Vec<_>>();
                            let [returned] = returns.as_slice() else { return Err(invalid()); };
                            if !creators::return_transfer_connected(maker, returned) { return Err(invalid()); }
                            let holder = ConnectedCaptureHolderFact { holder: returned.holder.clone(), environment: returned.environment.clone(), producer: returned.producer.clone() };
                            let originals = creators::validate(&maker.producer_owner, &holder, forest)?;
                            let terminal = terminal(forest, &maker.producer_owner, &returned.producer)?;
                            paths.push(OriginalCalleePath::Imported { owner: cfg.producer_owner.clone(), environment: environment.clone(), fact: fact.clone(), import: (*import).clone(), call: point.clone(), selection_path: selection_path.clone(), returned: (*returned).clone(), originals, terminal });
                        }
                    }
                }
                if paths.len() == start { return Err(invalid()); }
            }
        }
    }
    Ok(paths)
}

pub(in crate::scalar::evaluated_value::remap) fn signatures(paths: &[OriginalCalleePath]) -> Vec<(SourceEventId, ScalarType)> {
    let mut signatures = Vec::new();
    for path in paths {
        match path {
            OriginalCalleePath::Capture { owner, point, .. } => {
                if let CfgPointKind::Capture { signature, .. } = &point.kind { signatures.push((SourceEventId { owner: owner.clone(), point: point.id }, signature.clone())); }
            }
            OriginalCalleePath::Captured { terminal, .. } | OriginalCalleePath::Imported { terminal, .. } => signatures.extend(terminal.iter().cloned()),
            OriginalCalleePath::Parameter { .. } => {},
        }
    }
    let mut unique = Vec::new();
    for signature in signatures { if !unique.contains(&signature) { unique.push(signature); } }
    unique
}

fn terminal(forest: &SourceCaptureConstructionForest, owner: &ProducerCfgOwnerId, producer: &HolderProducerRef) -> Result<Vec<(SourceEventId, ScalarType)>, ValueSchemaRemapError> {
    let mut current_owner = owner.clone();
    let mut current = producer.clone();
    let mut visited = Vec::new();
    loop {
        if visited.contains(&(current_owner.clone(), current.clone())) { return Err(ValueSchemaRemapError::MissingProducer(current_owner)); }
        visited.push((current_owner.clone(), current.clone()));
        let actual = creators::constructor(&current_owner, &current, forest)?;
        let cfg = actual.resource_construction.original();
        match current {
            HolderProducerRef::CaptureCreation { point, environment } => {
                let points = cfg.initial_points.iter().filter(|row| row.id == point && matches!(&row.kind, CfgPointKind::Capture { environment: original, .. } if *original == environment)).collect::<Vec<_>>();
                let [point] = points.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(actual.producer.clone())); };
                let CfgPointKind::Capture { signature, .. } = &point.kind else { unreachable!(); };
                return Ok(vec![(SourceEventId { owner: actual.producer.clone(), point: point.id }, signature.clone())]);
            }
            HolderProducerRef::Forward { previous, .. } => { current_owner = actual.producer.clone(); current = *previous; }
            HolderProducerRef::CallCreation { selection, .. } => {
                if let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } = &selection.target {
                    let valid = matches!(&selection.source.table.owner, ProducerCfgOwnerId::Function { source: actual, declaration, body } if actual == source && declaration == declaration_span && match (body, concrete) {
                        (ProducerFunctionBodyId::Declaration, ConcreteCallSelection::Declaration) => true,
                        (ProducerFunctionBodyId::Overload(a), ConcreteCallSelection::Overload(b)) => *a == b.arm_index,
                        _ => false,
                    });
                    if !valid { return Err(ValueSchemaRemapError::MissingProducer(selection.source.table.owner.clone())); }
                }
                let maker = original(forest, &selection.source.table.owner)?;
                let rows = maker.producer_return_cases.iter().filter(|row| row.table == selection.source.table && row.return_point == selection.source.return_point && row.output == selection.source.output && row.alternative == selection.source.alternative).collect::<Vec<_>>();
                let [returned] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(maker.producer_owner.clone())); };
                current_owner = maker.producer_owner.clone(); current = returned.producer.clone();
            }
            HolderProducerRef::InstalledRead { .. } | HolderProducerRef::GlobalCallsiteInput { .. } => return Ok(Vec::new()),
        }
    }
}

fn environment(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &CallableEnvironmentId) -> Result<CallableEnvironmentId, ValueSchemaRemapError> {
    if remap.producers.iter().any(|pair| pair.0 == *owner) { remap.checked_environment(owner, source) } else { Ok(source.clone()) }
}

pub(in crate::scalar::evaluated_value::remap) fn paired(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &[OriginalCalleePath], destination: &[OriginalCalleePath]) -> Result<bool, ValueSchemaRemapError> {
    if source.len() != destination.len() { return Ok(false); }
    let mut used = Vec::new();
    for path in source {
        let mut matches = Vec::new();
        for (index, actual) in destination.iter().enumerate() {
            let valid = match (path, actual) {
                (OriginalCalleePath::Captured { environment: a, path: p, holder: x, .. }, OriginalCalleePath::Captured { environment: b, path: q, holder: y, .. }) => {
                    let holder = if remap.producers.iter().any(|pair| pair.0 == p.owner) { remap.checked_holder(&p.owner, x)? } else { x.clone() };
                    environment(remap, owner, a)? == *b && qualified_input_owner(remap, &p.owner)? == q.owner && holder == *y && capture_path(remap, p, q)?
                }
                (OriginalCalleePath::Capture { owner: a, environment: x, point: p, .. }, OriginalCalleePath::Capture { owner: b, environment: y, point: q, .. }) => qualified_input_owner(remap, a)? == *b && environment(remap, owner, x)? == *y && remap.boundary_point(&SourceEventId { owner: a.clone(), point: p.id })?.point == q.id,
                (OriginalCalleePath::Imported { environment: a, import: x, call: p, selection_path: s, returned: r, .. }, OriginalCalleePath::Imported { environment: b, import: y, call: q, selection_path: t, returned: u, .. }) => environment(remap, owner, a)? == *b && qualified_input_owner(remap, &x.table.owner)? == y.table.owner && remap.boundary_point(&SourceEventId { owner: owner.clone(), point: p.id })?.point == q.id && selection_path(remap, owner, s, t)? && r.output == u.output && environment(remap, &x.table.owner, &r.environment)? == u.environment && r.alternative.source_index == x.source_index && u.alternative.source_index == y.source_index && instance(remap, &x.table.owner, &x.source_instance, &y.source_instance)? && instance(remap, owner, &x.destination_instance, &y.destination_instance)?,
                (OriginalCalleePath::Parameter { owner: a, declaration: x_owner, slot: x, shape: p }, OriginalCalleePath::Parameter { owner: b, declaration: y_owner, slot: y, shape: q }) => qualified_input_owner(remap, a)? == *b && qualified_input_owner(remap, x_owner)? == *y_owner && remap.boundary_role(a, match &x.role { SourceCoordinateRole::Boundary(role) => role, _ => return Ok(false) })? == *match &y.role { SourceCoordinateRole::Boundary(role) => role, _ => return Ok(false) } && remap.boundary_shape(&p.id)? == q.id,
                _ => false,
            };
            if valid { matches.push(index); }
        }
        let [index] = matches.as_slice() else { return Ok(false); };
        if used.contains(index) { return Ok(false); }
        used.push(*index);
    }
    Ok(true)
}

fn binding(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &ReferenceBindingId) -> Result<ReferenceBindingId, ValueSchemaRemapError> {
    if remap.producers.iter().any(|row| row.0 == *owner) { remap.binding(owner, source) } else { Ok(source.clone()) }
}

fn instance(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &CallableEnvironmentInstance, destination: &CallableEnvironmentInstance) -> Result<bool, ValueSchemaRemapError> {
    match (source, destination) {
        (CallableEnvironmentInstance::Capture, CallableEnvironmentInstance::Capture) => Ok(true),
        (CallableEnvironmentInstance::CallOutput { call: a, output: x, function_span: p }, CallableEnvironmentInstance::CallOutput { call: b, output: y, function_span: q }) => Ok(a.range == b.range && x == y && p == q && a.source == *copied::lexical(owner) && b.source == *copied::lexical(&qualified_input_owner(remap, owner)?)),
        _ => Ok(false),
    }
}

fn capture_path(remap: &ValueSchemaRemap, source: &CaptureInputPathFact, destination: &CaptureInputPathFact) -> Result<bool, ValueSchemaRemapError> {
    let event = SourceEventId { owner: source.owner.clone(), point: source.capture };
    let event = if remap.producers.iter().any(|row| row.0 == source.owner) { remap.boundary_point(&event)? } else { event };
    if event.owner != destination.owner || event.point != destination.capture || source.capture_span.range != destination.capture_span.range || source.inputs.len() != destination.inputs.len() { return Ok(false); }
    for (a, b) in source.inputs.iter().zip(&destination.inputs) {
        if binding(remap, &source.owner, &a.binding)? != b.binding { return Ok(false); }
        match (&a.origin, &b.origin) {
            (CaptureInputOriginFact::Value, CaptureInputOriginFact::Value) => {},
            (CaptureInputOriginFact::Callable { holders: a }, CaptureInputOriginFact::Callable { holders: b }) => {
                if a.len() != b.len() { return Ok(false); }
                for (a, b) in a.iter().zip(b) {
                    let mapped = if remap.producers.iter().any(|row| row.0 == source.owner) { remap.checked_holder(&source.owner, a)? } else { a.clone() };
                    if mapped != *b { return Ok(false); }
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn selection_path(remap: &ValueSchemaRemap, owner: &ProducerCfgOwnerId, source: &[ProducerCallSelectionFact], destination: &[ProducerCallSelectionFact]) -> Result<bool, ValueSchemaRemapError> {
    if source.len() != destination.len() { return Ok(false); }
    for (a, b) in source.iter().zip(destination) {
        let actual_maker = qualified_input_owner(remap, &a.source.table.owner)?;
        let returned = SourceEventId { owner: a.source.table.owner.clone(), point: a.source.return_point };
        let returned = if remap.producers.iter().any(|row| row.0 == a.source.table.owner) { remap.boundary_point(&returned)? } else { returned };
        if actual_maker != b.source.table.owner || returned.point != b.source.return_point || a.output != b.output || a.source.output != b.source.output
            || environment(remap, owner, &a.destination)? != b.destination || !instance(remap, &a.source.table.owner, &a.source.alternative.source_instance, &b.source.alternative.source_instance)? { return Ok(false); }
    }
    Ok(true)
}
