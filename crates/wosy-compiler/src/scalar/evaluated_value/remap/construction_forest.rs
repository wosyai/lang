use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::scalar) struct SourceCaptureSupplementalAssociation {
    observation: SourceEventId,
    production: SourceEventId,
}

fn original_source(owner: &ProducerCfgOwnerId) -> &SourceIdentity {
    match owner {
        ProducerCfgOwnerId::Module(source) | ProducerCfgOwnerId::Function { source, .. } => source,
        ProducerCfgOwnerId::Literal { enclosing, .. } => original_source(enclosing),
    }
}

fn productive(source: &ValueTableSource, point: CfgPointId) -> bool {
    source.top_level.points.iter().all(|row| row.point != point || matches!(row.execution, ScalarTopLevelPointExecution::Connected { .. }))
}

fn same_constructor(a: &ReferenceFlowPoint, b: &ReferenceFlowPoint) -> bool {
    let (CfgPointKind::Capture { literal: x, environment: e, signature: s, captures: c, parameters: p, parameter_spans: q, body: f }, CfgPointKind::Capture { literal: y, environment: h, signature: t, captures: d, parameters: r, parameter_spans: u, body: g }) = (&a.kind, &b.kind) else { return false; };
    a.source_span == b.source_span && x == y && e.literal == h.literal && s == t && c == d && p == r && q == u && f == g
        && ScalarCaptureOccurrence::for_literal(*x, f) == ScalarCaptureOccurrence::for_literal(*y, g)
}

impl ValueSchemaRemap {
    pub(in crate::scalar) fn validate_construction_forest(forest: &SourceCaptureConstructionForest) -> Result<Vec<SourceCaptureSupplementalAssociation>, ValueSchemaRemapError> {
        if forest.roots.is_empty() && forest.sources.is_empty() && forest.pending.is_empty() { return Err(ValueSchemaRemapError::EmptyConstructionForest); }
        if forest.phase == SourceCaptureConstructionPhase::Complete && !forest.pending.is_empty() { return Err(ValueSchemaRemapError::MissingProducer(forest.pending[0].clone())); }
        for root in &forest.roots {
            if forest.roots.iter().filter(|row| row.owner == root.owner).count() != 1 || original_source(&root.owner) != &root.declaration.source
                || forest.sources.iter().any(|source| source.children.iter().any(|link| link.child == root.owner)) {
                return Err(ValueSchemaRemapError::MissingProducer(root.owner.clone()));
            }
            let pending = forest.pending.iter().filter(|owner| **owner == root.owner).count();
            let sources = forest.sources.iter().filter(|source| source.producer == root.owner).collect::<Vec<_>>();
            match (pending, sources.as_slice()) {
                (1, []) if forest.phase == SourceCaptureConstructionPhase::Building => {}
                (0, [source]) => {
                    let entries = source.initial_points.iter().filter(|row| matches!(row.kind, CfgPointKind::Entry)).collect::<Vec<_>>();
                    match entries.as_slice() {
                        [entry] if entry.source_span == root.declaration => {}
                        [] if matches!(&root.owner, ProducerCfgOwnerId::Module(_)) && root.declaration.range == ByteSpan::new(0, 0) => {}
                        _ => return Err(ValueSchemaRemapError::MissingProducer(root.owner.clone())),
                    }
                }
                _ => return Err(ValueSchemaRemapError::MissingProducer(root.owner.clone())),
            }
        }
        for pending in &forest.pending {
            if forest.roots.iter().filter(|root| root.owner == *pending).count() != 1 || forest.pending.iter().filter(|owner| *owner == pending).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(pending.clone())); }
        }
        Self::validate_capture_sources(&forest.sources)?;
        name_resolution::validate_original_forest(&forest.sources)?;
        formal_graph::validate_tables(&forest.sources.iter().map(|row| (&row.producer, &row.tables)).collect::<Vec<_>>())?;
        for source in &forest.sources {
            if forest.phase == SourceCaptureConstructionPhase::Complete && source.capture_inventory.phase != SourceCaptureConstructionPhase::Complete { return Err(ValueSchemaRemapError::MissingProducer(source.producer.clone())); }
            for point in &source.initial_points {
                if point.source_span.source != *original_source(&source.producer) { return Err(ValueSchemaRemapError::MissingPoint { producer: source.producer.clone(), point: point.id }); }
            }
            let mut owner = &source.producer;
            let mut visited = Vec::new();
            while !forest.roots.iter().any(|row| row.owner == *owner) {
                if visited.contains(owner) { return Err(ValueSchemaRemapError::MissingProducer(source.producer.clone())); }
                visited.push(owner.clone());
                let parents = forest.sources.iter().filter(|row| row.children.iter().filter(|link| link.child == *owner).count() == 1).collect::<Vec<_>>();
                let [parent] = parents.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(source.producer.clone())); };
                owner = &parent.producer;
            }
            for child in &source.children {
                if forest.sources.iter().filter(|row| row.producer == child.child).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(child.child.clone())); }
            }
        }
        let mut associations = Vec::new();
        for source in &forest.sources {
            for observation in &source.initial_points {
                if !matches!(observation.kind, CfgPointKind::Capture { .. }) || productive(source, observation.id) { continue; }
                let event = SourceEventId { owner: source.producer.clone(), point: observation.id };
                let candidates = forest.sources.iter().flat_map(|source| source.initial_points.iter().filter(|point| productive(source, point.id) && same_constructor(observation, point)).map(move |point| (source, point))).collect::<Vec<_>>();
                match candidates.as_slice() {
                    [(source, point)] => {
                        let CfgPointKind::Capture { literal, body, .. } = &point.kind else { unreachable!(); };
                        let occurrence = ScalarCaptureOccurrence::for_literal(*literal, body);
                        if source.capture_points.iter().filter(|row| row.point == point.id && row.occurrence == occurrence && row.role == ScalarCapturePointRole::Capture).count() != 1 { return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner, point: event.point }); }
                        associations.push(SourceCaptureSupplementalAssociation { observation: event, production: SourceEventId { owner: source.producer.clone(), point: point.id } });
                    }
                    [] if forest.phase == SourceCaptureConstructionPhase::Building => {
                        let pending = forest.roots.iter().filter(|root| forest.pending.contains(&root.owner) && root.declaration.source == observation.source_span.source && root.declaration.range.start <= observation.source_span.range.start && root.declaration.range.end >= observation.source_span.range.end).collect::<Vec<_>>();
                        let [_] = pending.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner, point: event.point }); };
                    }
                    _ => return Err(ValueSchemaRemapError::MissingPoint { producer: event.owner, point: event.point }),
                }
            }
        }
        Ok(associations)
    }

    pub(super) fn validate_forest_sources(&self, source: &[ValueTableSource], forest: &SourceCaptureConstructionForest) -> Result<(), ValueSchemaRemapError> {
        Self::validate_construction_forest(forest)?;
        if source.is_empty() { return Err(ValueSchemaRemapError::EmptyTransportSources); }
        for original in source {
            let rows = forest.sources.iter().filter(|row| row.producer == original.producer).collect::<Vec<_>>();
            let [row] = rows.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); };
            if *row != original || source.iter().filter(|row| row.producer == original.producer).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(original.producer.clone())); }
            for child in &original.children {
                if source.iter().filter(|row| row.producer == child.child).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(child.child.clone())); }
            }
        }
        Ok(())
    }
}
