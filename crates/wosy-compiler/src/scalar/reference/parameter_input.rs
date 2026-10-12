use super::*;

fn invalid(source: &SourceSpan) -> ReferenceAnalysisError {
    ReferenceAnalysisError::MissingContext {
        source_span: source.clone(),
        reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
    }
}

impl ReferenceFlowBuilder {
    pub(super) fn introduce_nonchecked_parameters(&mut self, signature: &ScalarType) -> Result<(), ReferenceAnalysisError> {
        let ScalarType::Callable { parameters, .. } = signature else {
            return Err(invalid(&self.cfg.points[0].source_span));
        };
        if parameters.len() != self.cfg.reference_parameters.len() {
            return Err(invalid(&self.cfg.points[0].source_span));
        }
        for (binding, ty) in self.cfg.reference_parameters.clone().iter().zip(parameters) {
            if self.binding_types.get(binding) != Some(ty) {
                return Err(invalid(&SourceSpan::new(binding.source.clone(), binding.declaration_span)));
            }
            self.register_source_binding(binding, ty)?;
            if !matches!(ty, ScalarType::CheckedReference { .. }) {
                if self.cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Bind { binding: actual } if actual == binding)) {
                    return Err(invalid(&SourceSpan::new(binding.source.clone(), binding.declaration_span)));
                }
                self.point(CfgPointKind::Bind { binding: binding.clone() }, binding.declaration_span, ReferenceScopeId(0), BTreeSet::new());
            }
        }
        Ok(())
    }
}

pub(in crate::scalar) fn introductions(cfg: &ScalarReferenceCfg) -> Result<Vec<CfgPointId>, ReferenceAnalysisError> {
    let mut points = Vec::new();
    for (ordinal, binding) in cfg.reference_parameters.iter().enumerate() {
        let source = SourceSpan::new(binding.source.clone(), binding.declaration_span);
        let fail = || invalid(&source);
        if cfg.reference_parameters.iter().filter(|actual| *actual == binding).count() != 1
            || binding.kind != ReferenceBindingKind::Declared {
            return Err(fail());
        }
        let signatures = cfg.evaluated_values.boundary_signatures.iter().filter(|row| row.owner == cfg.producer_owner).collect::<Vec<_>>();
        let [signature] = signatures.as_slice() else { return Err(fail()); };
        let ports = signature.ports.iter().filter(|port| port.role == BoundaryPortRole::Parameter { ordinal, binding: binding.clone() }).collect::<Vec<_>>();
        let [port] = ports.as_slice() else { return Err(fail()); };
        if signature.ports.iter().filter(|port| matches!(port.role, BoundaryPortRole::Parameter { .. })).count() != cfg.reference_parameters.len() {
            return Err(fail());
        }
        let shapes = cfg.evaluated_values.source_binding_shapes.iter().filter(|row| row.binding == *binding).collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else { return Err(fail()); };
        if shape.shape != port.shape || shape.shape.producer != cfg.producer_owner {
            return Err(fail());
        }
        let introductions = cfg.points.iter().filter(|point| matches!(&point.kind, CfgPointKind::Bind { binding: actual } if actual == binding)).collect::<Vec<_>>();
        let [point] = introductions.as_slice() else { return Err(fail()); };
        let entries = cfg.points.iter().filter(|point| point.id == signature.entry.point && matches!(point.kind, CfgPointKind::Entry)).collect::<Vec<_>>();
        let [entry] = entries.as_slice() else { return Err(fail()); };
        if point.source_span != source || point.scope != entry.scope || point.id.0 <= entry.id.0
            || binding.function_span != entry.source_span.range
            || binding.source != entry.source_span.source
            || cfg.points.iter().any(|read| read.id.0 <= point.id.0 && matches!(&read.kind, CfgPointKind::Read { binding: actual } if actual == binding))
            || cfg.points.iter().any(|body| body.id.0 < point.id.0 && matches!(body.kind, CfgPointKind::ScopeEnter))
            || cfg.binding_points.iter().any(|row| row.point == point.id)
            || cfg.evaluated_values.binding_inputs.iter().any(|row| row.consumer == point.id)
            || cfg.evaluated_values.values.iter().any(|row| row.id.point == point.id) {
            return Err(fail());
        }
        let schemas = cfg.evaluated_values.shapes.iter().filter(|schema| schema.id == port.shape).collect::<Vec<_>>();
        let [schema] = schemas.as_slice() else { return Err(fail()); };
        let installs = cfg.checked_installs.iter().filter(|install| install.point == point.id).collect::<Vec<_>>();
        if let ScalarType::CheckedReference { mutability, .. } = &schema.ty {
            let [install] = installs.as_slice() else { return Err(fail()); };
            let ReferenceCheckedInstallInput::Parameter { binding: actual, origin } = &install.input else { return Err(fail()); };
            let ReferenceOrigin::BorrowedFrom { parameter, loan } = cfg.facts.get(origin.0).ok_or_else(fail)? else { return Err(fail()); };
            let mut referent = binding.clone();
            referent.kind = ReferenceBindingKind::BorrowedReferent { parameter: ordinal };
            if actual != binding || *parameter != ordinal || loan.mode != *mutability
                || loan.creation_source != binding.source || loan.creation_span != binding.declaration_span
                || loan.parent.is_some() || loan.origin_place.binding != referent
                || loan.origin_place.source != binding.source || loan.origin_place.function_span != binding.function_span
                || loan.origin_place.declaration_span != binding.declaration_span || loan.origin_place.block_span != binding.block_span
                || !loan.origin_place.projections.is_empty() || point.possible_origins != Some(BTreeSet::from([*origin])) {
                return Err(fail());
            }
        } else if !installs.is_empty() {
            return Err(fail());
        }
        points.push(point.id);
    }
    let mut expected_order = Vec::new();
    for checked in [true, false] {
        for (binding, point) in cfg.reference_parameters.iter().zip(&points) {
            let shape = cfg.evaluated_values.source_binding_shapes.iter().find(|shape| shape.binding == *binding).ok_or_else(|| invalid(&cfg.points[0].source_span))?;
            let schema = cfg.evaluated_values.shapes.iter().find(|schema| schema.id == shape.shape).ok_or_else(|| invalid(&cfg.points[0].source_span))?;
            if matches!(schema.ty, ScalarType::CheckedReference { .. }) == checked {
                expected_order.push(*point);
            }
        }
    }
    if !expected_order.windows(2).all(|pair| pair[0].0 < pair[1].0) {
        return Err(invalid(&cfg.points[0].source_span));
    }
    Ok(points)
}

pub(in crate::scalar) fn validate_owners(cfg: &ScalarReferenceCfg) -> Result<(), ReferenceAnalysisError> {
    let points = introductions(cfg)?;
    for (binding, point) in cfg.reference_parameters.iter().zip(points) {
        let source = SourceSpan::new(binding.source.clone(), binding.declaration_span);
        let owners = cfg.evaluated_values.owner_capabilities.iter().filter(|row| row.role == OwnerRole::Parameter(binding.clone())).collect::<Vec<_>>();
        let [owner] = owners.as_slice() else { return Err(invalid(&source)); };
        if owner.id.producer != cfg.producer_owner || owner.id.introduction != point
            || owner.introduction != (SourceEventId { owner: cfg.producer_owner.clone(), point })
            || owner.endpoints.is_empty() || !owner.endpoints.iter().all(|endpoint| endpoint.kind == OwnerEndpointKind::ParameterExit { binding: binding.clone() }) {
            return Err(invalid(&source));
        }
    }
    if cfg.evaluated_values.owner_capabilities.iter().filter(|row| matches!(row.role, OwnerRole::Parameter(_))).count() != cfg.reference_parameters.len() {
        return Err(invalid(&cfg.points[0].source_span));
    }
    Ok(())
}

pub(super) fn callable(state: &ReferenceEdgeState, binding: &ReferenceBindingId) -> BTreeSet<CallableEnvironmentId> {
    state.callable_bindings.get(binding).into_iter().flatten().cloned().collect()
}

pub(super) fn holders(state: &mut ReferenceEdgeState, binding: &ReferenceBindingId) {
    state.holder_sources = state.holder_relations.iter().flat_map(|row| &row.holders).filter(|holder| match holder {
        CallableHolderPlace::Binding(actual) => actual == binding,
        CallableHolderPlace::Projection(place) => place.binding == *binding,
        _ => false,
    }).cloned().collect();
    state.holder_value = state.holder_relations.iter().filter(|row| row.holders.iter().any(|holder| state.holder_sources.contains(holder))).cloned().collect();
}

#[cfg(test)]
mod tests;
