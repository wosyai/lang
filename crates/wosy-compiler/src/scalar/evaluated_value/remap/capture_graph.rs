use super::*;

fn capture_error(owner: &ProducerCfgOwnerId, point: CfgPointId) -> ValueSchemaRemapError {
    ValueSchemaRemapError::MissingPoint { producer: owner.clone(), point }
}

fn generic_names(ty: &ScalarType, names: &mut BTreeSet<String>) {
    match ty {
        ScalarType::Named { name, .. } => { names.insert(name.clone()); }
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => generic_names(inner, names),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => generic_names(element, names),
        ScalarType::Callable { parameters, outputs } => { for parameter in parameters { generic_names(parameter, names); } for output in &outputs.outputs { generic_names(&output.ty, names); } }
        _ => {}
    }
}

fn canonical_type(ty: &ScalarType, context: &SourceCapturePolicyContext) -> bool {
    match ty {
        ScalarType::Struct(id) => context.structs.iter().filter(|row| row.id == *id).count() == 1,
        ScalarType::Enum(id) => context.enums.iter().filter(|row| row.id == *id).count() == 1,
        ScalarType::Named { name, .. } => context.generic_parameters.contains(name),
        ScalarType::Qualified { .. } | ScalarType::Error => false,
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => canonical_type(inner, context),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => canonical_type(element, context),
        ScalarType::Callable { parameters, outputs } => parameters.iter().all(|ty| canonical_type(ty, context)) && outputs.outputs.iter().all(|output| canonical_type(&output.ty, context)),
        _ => true,
    }
}

impl ValueSchemaRemap {
    pub(super) fn validate_capture_sources(sources: &[ValueTableSource]) -> Result<(), ValueSchemaRemapError> {
        for source in sources {
            if sources.iter().filter(|row| row.producer == source.producer).count() != 1 {
                return Err(ValueSchemaRemapError::MissingProducer(source.producer.clone()));
            }
            Self::validate_header(&source.tables, &source.producer)?;
            for signature in &source.tables.boundary_signatures {
                let entries = source.initial_points.iter().filter(|row| row.id == signature.entry.point && matches!(row.kind, CfgPointKind::Entry)).collect::<Vec<_>>();
                let [_] = entries.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySignature(source.producer.clone())); };
            }
            let context = &source.policy_context;
            for declaration in &context.structs {
                if context.structs.iter().filter(|row| row.id == declaration.id).count() != 1 {
                    return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone()));
                }
            }
            for declaration in &context.enums {
                if context.enums.iter().filter(|row| row.id == declaration.id).count() != 1 {
                    return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone()));
                }
            }
            let mut enclosing = &source.producer;
            while let ProducerCfgOwnerId::Literal { enclosing: parent, .. } = enclosing { enclosing = parent; }
            if enclosing != &context.generic_scope
            { return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone())); }
            if matches!(context.generic_scope, ProducerCfgOwnerId::Module(_)) && !context.generic_parameters.is_empty() {
                return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone()));
            }
            if !template::valid_context(context) { return Err(ValueSchemaRemapError::MissingProducer(context.generic_scope.clone())); }
            template::validate_shapes(&source.tables.shapes, &source.producer, &context.generic_scope, &context.generic_declarations)
                .map_err(|_| ValueSchemaRemapError::MissingProducer(source.producer.clone()))?;
            let mut required_events = Vec::new();
            for point in &source.initial_points {
                let CfgPointKind::Capture { literal, environment, signature, captures, parameters, parameter_spans, body } = &point.kind else { continue; };
                if source.initial_points.iter().filter(|row| row.id == point.id).count() != 1 {
                    return Err(capture_error(&source.producer, point.id));
                }
                let execution = source.top_level.points.iter().filter(|row| row.point == point.id).collect::<Vec<_>>();
                let productive = match (&source.producer, execution.as_slice()) {
                    (ProducerCfgOwnerId::Module(_), [row]) => matches!(row.execution, ScalarTopLevelPointExecution::Connected { .. }),
                    (ProducerCfgOwnerId::Module(_), _) => return Err(capture_error(&source.producer, point.id)),
                    (_, []) => true,
                    (_, [row]) => matches!(row.execution, ScalarTopLevelPointExecution::Connected { .. }),
                    _ => return Err(capture_error(&source.producer, point.id)),
                };
                let declaration = reference::source_epoch::SourceCaptureDeclaration::new(
                    &source.producer, &point.source_span, point.id, *literal, &environment.literal,
                    signature, captures, parameters, parameter_spans, body,
                ).map_err(|_| capture_error(&source.producer, point.id))?;
                if !productive { continue; }
                if !canonical_type(signature, context) || captures.iter().any(|capture| !canonical_type(&capture.ty, context) || !canonical_type(&capture.place_type, context)) {
                    return Err(capture_error(&source.producer, point.id));
                }
                let occurrence = ScalarCaptureOccurrence::for_literal(*literal, body);
                let registrations = source.capture_points.iter().filter(|row| row.point == point.id).collect::<Vec<_>>();
                let [registration] = registrations.as_slice() else { return Err(capture_error(&source.producer, point.id)); };
                if registration.occurrence != occurrence || registration.role != ScalarCapturePointRole::Capture { return Err(capture_error(&source.producer, point.id)); }
                let output = EvaluatedValueId { resolved: ResolvedValueOutputId { producer: source.producer.clone(), occurrence: point.source_span.clone(), output_projection: Vec::new(), output: 0 }, point: point.id };
                let values = source.tables.values.iter().filter(|row| row.id == output).collect::<Vec<_>>();
                let [value] = values.as_slice() else { return Err(capture_error(&source.producer, point.id)); };
                Self::capture_shape(&source.tables, &value.shape, signature)?;
                let requirements = source.capture_inventory.required.iter().filter(|row| row.event == declaration.event).collect::<Vec<_>>();
                let [requirement] = requirements.as_slice() else { return Err(capture_error(&source.producer, point.id)); };
                if requirement.output != output || requirement.child != declaration.literal_owner || requirement.signature != *signature || requirement.captures != *captures {
                    return Err(capture_error(&source.producer, point.id));
                }
                required_events.push(declaration.event.clone());
                let productions = source.tables.source_capture_productions.iter().filter(|row| row.event == declaration.event).collect::<Vec<_>>();
                let pending = source.tables.source_capture_pending.iter().filter(|row| row.event == declaration.event).collect::<Vec<_>>();
                let links = source.children.iter().filter(|row| row.event == declaration.event).collect::<Vec<_>>();
                for capture in captures {
                    let coordinates = source.tables.source_slots.iter().filter(|row| row.role == SourceCoordinateRole::Binding(capture.binding.clone())).collect::<Vec<_>>();
                    let [coordinate] = coordinates.as_slice() else { return Err(capture_error(&source.producer, point.id)); };
                    Self::capture_shape(&source.tables, &coordinate.shape, &capture.ty)?;
                    copy_policy(&capture.ty, &context.structs, &context.enums, &context.generic_parameters).map_err(|_| capture_error(&source.producer, point.id))?;
                }
                match (productions.as_slice(), pending.as_slice(), links.as_slice()) {
                    ([], [pending], links) if source.capture_inventory.phase == SourceCaptureConstructionPhase::Building => {
                        if pending.output != output || pending.child != declaration.literal_owner {
                            return Err(capture_error(&source.producer, point.id));
                        }
                        match links {
                            [] => {}
                            [link] if link.child == declaration.literal_owner => {
                                if sources.iter().filter(|row| row.producer == link.child).count() != 1 { return Err(ValueSchemaRemapError::MissingProducer(link.child.clone())); }
                            }
                            _ => return Err(capture_error(&source.producer, point.id)),
                        }
                    }
                    ([production], [], [link]) => {
                        if link.child != declaration.literal_owner || production.output != output || production.child != link.child || production.signature != *signature || production.inputs.len() != captures.len() {
                            return Err(capture_error(&source.producer, point.id));
                        }
                        let children = sources.iter().filter(|row| row.producer == link.child).collect::<Vec<_>>();
                        let [child] = children.as_slice() else { return Err(ValueSchemaRemapError::MissingProducer(link.child.clone())); };
                        if source.capture_inventory.phase == SourceCaptureConstructionPhase::Complete && child.capture_inventory.phase != SourceCaptureConstructionPhase::Complete {
                            return Err(ValueSchemaRemapError::MissingProducer(link.child.clone()));
                        }
                        let signatures = child.tables.boundary_signatures.iter().filter(|row| row.owner == link.child).collect::<Vec<_>>();
                        let [child_signature] = signatures.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySignature(link.child.clone())); };
                        let ScalarType::Callable { outputs, parameters: parameter_types } = signature else { return Err(capture_error(&source.producer, point.id)); };
                        if child_signature.ports.iter().filter(|row| matches!(row.role, BoundaryPortRole::Parameter { .. })).count() != declaration.parameters.len()
                            || child_signature.ports.iter().filter(|row| matches!(row.role, BoundaryPortRole::Output(_))).count() != outputs.outputs.len() {
                            return Err(ValueSchemaRemapError::MissingBoundarySignature(link.child.clone()));
                        }
                        for (ordinal, ty) in parameter_types.iter().enumerate() {
                            let binding = &declaration.parameters[ordinal].binding;
                            let ports = child_signature.ports.iter().filter(|row| row.role == BoundaryPortRole::Parameter { ordinal, binding: binding.clone() }).collect::<Vec<_>>();
                            let [port] = ports.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySignature(link.child.clone())); };
                            Self::capture_shape(&child.tables, &port.shape, ty)?;
                        }
                        for (ordinal, output) in outputs.outputs.iter().enumerate() {
                            let ports = child_signature.ports.iter().filter(|row| row.role == BoundaryPortRole::Output(ordinal)).collect::<Vec<_>>();
                            let [port] = ports.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySignature(link.child.clone())); };
                            Self::capture_shape(&child.tables, &port.shape, &output.ty)?;
                        }
                        if child_signature.ports.iter().filter(|row| matches!(row.role, BoundaryPortRole::Capture { .. })).count() != captures.len() {
                            return Err(ValueSchemaRemapError::MissingBoundarySignature(link.child.clone()));
                        }
                        for (ordinal, (capture, input)) in captures.iter().zip(&production.inputs).enumerate() {
                            let policy = copy_policy(&capture.ty, &context.structs, &context.enums, &context.generic_parameters).map_err(|_| capture_error(&source.producer, point.id))?;
                            let coordinates = source.tables.source_slots.iter().filter(|row| row.id == input.coordinate && row.role == SourceCoordinateRole::Binding(capture.binding.clone()) && row.shape == input.shape).collect::<Vec<_>>();
                            let [_] = coordinates.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundarySlot(input.coordinate.clone())); };
                            Self::capture_shape(&source.tables, &input.shape, &capture.ty)?;
                            let formals = child_signature.ports.iter().enumerate().filter(|(_, row)| row.role == BoundaryPortRole::Capture { ordinal, binding: capture.binding.clone() }).collect::<Vec<_>>();
                            let [(port, formal)] = formals.as_slice() else { return Err(ValueSchemaRemapError::MissingBoundaryFormal(input.formal.clone())); };
                            if input.capture != *capture || input.policy != policy || input.formal != (BoundaryFormalPortRef { owner: link.child.clone(), port: *port }) {
                                return Err(capture_error(&source.producer, point.id));
                            }
                            Self::capture_shape(&child.tables, &formal.shape, &capture.ty)?;
                            for instance in source.tables.source_capture_policy_instances.iter().filter(|row| row.event == declaration.event && row.formal == input.formal) {
                                let mut demanded = BTreeSet::new();
                                generic_names(&capture.ty, &mut demanded);
                                if instance.substitutions.keys().any(|name| !context.generic_parameters.contains(name)) || demanded.iter().any(|name| !context.generic_parameters.contains(name) || !instance.substitutions.contains_key(name)) {
                                    return Err(capture_error(&source.producer, point.id));
                                }
                                let ty = substitute_type(&capture.ty, &instance.substitutions);
                                let mut remaining = BTreeSet::new();
                                generic_names(&ty, &mut remaining);
                                if !remaining.is_empty() { return Err(capture_error(&source.producer, point.id)); }
                                let policy = copy_policy(&ty, &context.structs, &context.enums, &BTreeSet::new()).map_err(|_| capture_error(&source.producer, point.id))?;
                                if instance.ty != ty || instance.policy != policy || source.tables.source_capture_policy_instances.iter().filter(|row| row.event == instance.event && row.formal == instance.formal && row.substitutions == instance.substitutions).count() != 1 {
                                    return Err(capture_error(&source.producer, point.id));
                                }
                            }
                        }
                    }
                    _ => return Err(capture_error(&source.producer, point.id)),
                }
            }
            for requirement in &source.capture_inventory.required {
                if required_events.iter().filter(|event| **event == requirement.event).count() != 1 { return Err(capture_error(&source.producer, requirement.event.point)); }
            }
            for registration in &source.capture_points {
                if registration.role != ScalarCapturePointRole::Capture || source.capture_points.iter().filter(|row| row.point == registration.point).count() != 1 || source.initial_points.iter().filter(|row| {
                    if row.id != registration.point { return false; }
                    let CfgPointKind::Capture { literal, body, .. } = &row.kind else { return false; };
                    registration.occurrence == ScalarCaptureOccurrence::for_literal(*literal, body)
                }).count() != 1 {
                    return Err(capture_error(&source.producer, registration.point));
                }
            }
            for row in &source.tables.source_capture_productions {
                if !required_events.contains(&row.event) { return Err(capture_error(&source.producer, row.event.point)); }
            }
            for row in &source.tables.source_capture_pending {
                if !required_events.contains(&row.event) { return Err(capture_error(&source.producer, row.event.point)); }
            }
            for row in &source.tables.source_capture_policy_instances {
                if source.tables.source_capture_productions.iter().filter(|production| production.event == row.event && production.inputs.iter().any(|input| input.formal == row.formal)).count() != 1 { return Err(capture_error(&source.producer, row.event.point)); }
            }
            for link in &source.children {
                if !required_events.contains(&link.event) { return Err(capture_error(&source.producer, link.event.point)); }
            }
            if let ProducerCfgOwnerId::Literal { enclosing, capture_point, .. } = &source.producer {
                let parents = sources.iter().filter(|row| &row.producer == enclosing.as_ref() && row.children.iter().any(|link| link.child == source.producer && link.event.point == *capture_point)).collect::<Vec<_>>();
                if parents.len() > 1 { return Err(ValueSchemaRemapError::MissingProducer(source.producer.clone())); }
            }
        }
        Ok(())
    }

    pub(super) fn capture_shape(tables: &EvaluatedValueTables, id: &ValueShapeId, ty: &ScalarType) -> Result<(), ValueSchemaRemapError> {
        let shapes = tables.shapes.iter().filter(|row| row.id == *id).collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else { return Err(ValueSchemaRemapError::MissingShape(id.clone())); };
        if shape.ty != *ty { return Err(ValueSchemaRemapError::MissingShape(id.clone())); }
        Ok(())
    }

    pub(super) fn capture_binding(&self, owner: &ProducerCfgOwnerId, capture: &closure::ClosureCapture) -> Result<closure::ClosureCapture, ValueSchemaRemapError> {
        let mut mapped = capture.clone();
        mapped.binding = self.binding(owner, &capture.binding)?;
        Ok(mapped)
    }

    pub(super) fn capture_inventory(&self, inventory: &SourceCaptureInventory) -> Result<SourceCaptureInventory, ValueSchemaRemapError> {
        Ok(SourceCaptureInventory { phase: inventory.phase.clone(), required: inventory.required.iter().map(|row| {
            Ok(SourceCaptureRequirement { event: self.source_event(&row.event)?, output: self.value(&row.output)?, child: self.producer(&row.child)?, signature: row.signature.clone(), captures: row.captures.iter().map(|capture| self.capture_binding(&row.event.owner, capture)).collect::<Result<Vec<_>, ValueSchemaRemapError>>()? })
        }).collect::<Result<Vec<_>, ValueSchemaRemapError>>()? })
    }

    pub(super) fn capture_productions(&self, tables: &EvaluatedValueTables, correspondence: &BoundaryArenaCorrespondence) -> Result<Vec<SourceCaptureProduction>, ValueSchemaRemapError> {
        tables.source_capture_productions.iter().map(|row| {
            Ok(SourceCaptureProduction { event: self.source_event(&row.event)?, output: self.value(&row.output)?, child: self.producer(&row.child)?, signature: row.signature.clone(), inputs: row.inputs.iter().map(|input| {
                Ok(SourceCaptureInputPort { capture: self.capture_binding(&row.event.owner, &input.capture)?, formal: correspondence.formal(&input.formal)?, coordinate: correspondence.slot(&input.coordinate)?, shape: self.shape(&input.shape)?, policy: input.policy })
            }).collect::<Result<Vec<_>, ValueSchemaRemapError>>()? })
        }).collect()
    }
}
