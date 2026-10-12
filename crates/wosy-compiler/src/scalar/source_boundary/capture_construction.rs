use super::*;

impl reference::ReferenceFlowBuilder {
    pub(in crate::scalar) fn register_capture_input(&mut self, point: CfgPointId) -> Result<(), ReferenceAnalysisError> {
        let actual = &self.cfg.points[point.0];
        let source = actual.source_span.clone();
        let CfgPointKind::Capture { literal, environment, signature, captures, parameters, parameter_spans, body } = &actual.kind else { return Err(invalid(&source)); };
        let declaration = reference::source_epoch::SourceCaptureDeclaration::new(&self.cfg.producer_owner, &source, point, *literal, &environment.literal, signature, captures, parameters, parameter_spans, body).map_err(|_| invalid(&source))?;
        let requirement = SourceCaptureRequirement { event: declaration.event.clone(), output: bind_evaluated_value_id(&self.cfg, ResolvedValueOutputId { producer: self.cfg.producer_owner.clone(), occurrence: source.clone(), output_projection: Vec::new(), output: 0 }, point).map_err(|_| invalid(&source))?, child: declaration.literal_owner.clone(), signature: signature.clone(), captures: captures.clone() };
        let types = self.value_types.expressions.iter().filter(|row| row.source_span == source).collect::<Vec<_>>();
        let [types] = types.as_slice() else { return Err(invalid(&source)); };
        if types.outputs.outputs.len() != 1 || types.outputs.outputs[0].ty != requirement.signature
            || self.cfg.source_capture_inventory.phase != SourceCaptureConstructionPhase::Building
            || self.cfg.source_capture_inventory.required.iter().any(|row| row.event == requirement.event) { return Err(invalid(&source)); }
        let mut tables = self.cfg.evaluated_values.clone();
        let shape = intern_value_shape(&mut tables.shapes, &self.cfg.producer_owner, &requirement.signature, &self.structs, &self.enums, &self.generic_scope, &self.generic_declarations).map_err(|_| invalid(&source))?;
        let values = tables.values.iter().filter(|row| row.id.resolved == requirement.output.resolved).collect::<Vec<_>>();
        match values.as_slice() {
            [] => tables.values.push(EvaluatedValueFact { id: requirement.output.clone(), shape, cases: Vec::new() }),
            [row] if row.id == requirement.output && row.shape == shape => {},
            _ => return Err(invalid(&source)),
        }
        for capture in &requirement.captures {
            if self.binding_types.get(&capture.binding) != Some(&capture.ty) { return Err(invalid(&source)); }
            let shape = intern_value_shape(&mut tables.shapes, &self.cfg.producer_owner, &capture.ty, &self.structs, &self.enums, &self.generic_scope, &self.generic_declarations).map_err(|_| invalid(&source))?;
            coordinate::intern_coordinate(&mut tables, &self.cfg.producer_owner, SourceCoordinateRole::Binding(capture.binding.clone()), shape, &source)?;
            copy_policy(&capture.ty, &self.structs, &self.enums, &self.generic_parameters).map_err(|_| invalid(&source))?;
        }
        tables.source_capture_pending.push(SourceCapturePendingInput { event: requirement.event.clone(), output: requirement.output.clone(), child: requirement.child.clone() });
        self.cfg.evaluated_values = tables;
        self.cfg.source_capture_inventory.required.push(requirement);
        Ok(())
    }
}

pub(in crate::scalar) fn complete_capture_children(cfg: &mut ScalarReferenceCfg, input: &SourceCapturePreflightInput) -> Result<(), ReferenceAnalysisError> {
    validate_capture_preflight(cfg, input)?;
    let context = &input.policy_context;
    let mut productions = Vec::new();
    for requirement in &cfg.source_capture_inventory.required {
        let source = &requirement.output.resolved.occurrence;
        let children = cfg.literal_cfgs.iter().filter(|row| row.capture_point == requirement.event.point && row.reference_cfg.producer_owner == requirement.child).collect::<Vec<_>>();
        let [child] = children.as_slice() else { return Err(invalid(source)); };
        if child.reference_cfg.source_capture_inventory.phase != SourceCaptureConstructionPhase::Complete { return Err(invalid(source)); }
        let signatures = child.reference_cfg.evaluated_values.boundary_signatures.iter().filter(|row| row.owner == requirement.child).collect::<Vec<_>>();
        let [signature] = signatures.as_slice() else { return Err(invalid(source)); };
        let pending = cfg.evaluated_values.source_capture_pending.iter().filter(|row| row.event == requirement.event && row.output == requirement.output && row.child == requirement.child).collect::<Vec<_>>();
        if pending.len() != 1 || cfg.evaluated_values.source_capture_productions.iter().any(|row| row.event == requirement.event) { return Err(invalid(source)); }
        let mut inputs = Vec::new();
        for (ordinal, capture) in requirement.captures.iter().enumerate() {
            let formals = signature.ports.iter().enumerate().filter(|(_, row)| row.role == BoundaryPortRole::Capture { ordinal, binding: capture.binding.clone() }).collect::<Vec<_>>();
            let [(port, _)] = formals.as_slice() else { return Err(invalid(source)); };
            let slots = cfg.evaluated_values.source_slots.iter().filter(|row| row.role == SourceCoordinateRole::Binding(capture.binding.clone())).collect::<Vec<_>>();
            let [slot] = slots.as_slice() else { return Err(invalid(source)); };
            let policy = copy_policy(&capture.ty, &context.structs, &context.enums, &context.generic_parameters).map_err(|_| invalid(source))?;
            inputs.push(SourceCaptureInputPort { capture: capture.clone(), formal: BoundaryFormalPortRef { owner: requirement.child.clone(), port: *port }, coordinate: slot.id.clone(), shape: slot.shape.clone(), policy });
        }
        productions.push(SourceCaptureProduction { event: requirement.event.clone(), output: requirement.output.clone(), child: requirement.child.clone(), signature: requirement.signature.clone(), inputs });
    }
    let mut staged = cfg.clone();
    staged.evaluated_values.source_capture_productions.extend(productions);
    staged.evaluated_values.source_capture_pending.retain(|row| !staged.source_capture_inventory.required.iter().any(|required| required.event == row.event));
    validate_capture_preflight(&staged, input)?;
    cfg.evaluated_values = staged.evaluated_values;
    Ok(())
}

pub(in crate::scalar) fn seal_capture_tree(cfg: &mut ScalarReferenceCfg, context: &SourceCapturePolicyContext) -> Result<(), ReferenceAnalysisError> {
    let mut staged = cfg.clone();
    staged.source_capture_inventory.phase = SourceCaptureConstructionPhase::Complete;
    let input = SourceCapturePreflightInput { policy_context: context.clone(), construction_forest: capture_root_construction_forest(&staged, context).map_err(|_| invalid(&cfg.initial_points[0].source_span))?, substitutions: BTreeMap::new() };
    validate_capture_preflight(&staged, &input)?;
    cfg.source_capture_inventory.phase = SourceCaptureConstructionPhase::Complete;
    Ok(())
}
