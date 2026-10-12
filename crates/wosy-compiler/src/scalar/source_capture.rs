use super::*;

impl SourceEpoch {
    pub(super) fn construct_captures(
        &mut self,
        cfgs: &[&ScalarReferenceCfg],
        owners: &[ProducerCfgOwnerId],
        structs: &[ScalarStruct],
        enums: &[ScalarEnum],
        contexts: &[SourceCapturePolicyContext],
    ) -> Result<(), SourceEpochError> {
        for cfg in cfgs {
            let context = Self::lexical_context(&cfg.producer_owner, contexts)?;
            for point in &cfg.initial_points {
                let CfgPointKind::Capture {
                    literal,
                    environment,
                    signature,
                    captures,
                    parameters,
                    parameter_spans,
                    body,
                } = &point.kind
                else {
                    continue;
                };
                let declaration = SourceCaptureDeclaration::new(
                    &cfg.producer_owner,
                    &point.source_span,
                    point.id,
                    *literal,
                    &environment.literal,
                    signature,
                    captures,
                    parameters,
                    parameter_spans,
                    body,
                )?;
                let event = declaration.event;
                let literal_owner = declaration.literal_owner;
                if !owners.contains(&literal_owner) {
                    return Err(SourceEpochError::MissingLexicalOwner {
                        event,
                        owner: literal_owner,
                    });
                }
                let signature = self.capture_shape(
                    &event,
                    &cfg.producer_owner,
                    declaration.signature,
                    structs,
                    enums, context,
                )?;
                let mut inputs = Vec::new();
                for (ordinal, capture) in declaration.captures.iter().enumerate() {
                    let storage = if matches!(capture.ty, ScalarType::CheckedReference { .. }) {
                        let storage =
                            cfg.checked_bindings.get(&capture.binding).ok_or_else(|| {
                                SourceEpochError::MissingCheckedCaptureStorage {
                                    event: event.clone(),
                                    binding: capture.binding.clone(),
                                }
                            })?;
                        if storage.binding != capture.binding {
                            return Err(SourceEpochError::ForeignCheckedCaptureStorage {
                                event,
                                binding: capture.binding.clone(),
                                storage_binding: storage.binding.clone(),
                            });
                        }
                        SourceCaptureStorage::CheckedSlot(capture.binding.clone())
                    } else {
                        SourceCaptureStorage::Binding(capture.binding.clone())
                    };
                    let value_shape = self.capture_shape(
                        &event,
                        &cfg.producer_owner,
                        &capture.ty,
                        structs,
                        enums, context,
                    )?;
                    let place_shape = self.capture_shape(
                        &event,
                        &cfg.producer_owner,
                        &capture.place_type,
                        structs,
                        enums, context,
                    )?;
                    let formal_shape =
                        self.capture_shape(&event, &literal_owner, &capture.ty, structs, enums, context)?;
                    inputs.push(SourceCapturePort {
                        ordinal,
                        name: capture.name.clone(),
                        binding: capture.binding.clone(),
                        value_shape,
                        place_shape,
                        formal_shape,
                        use_span: capture.use_span.clone(),
                        place: capture.place.clone(),
                        storage,
                    });
                }
                let mut formal_parameters = Vec::new();
                for parameter in declaration.parameters {
                    let shape =
                        self.capture_shape(&event, &literal_owner, parameter.ty, structs, enums, context)?;
                    formal_parameters.push(SourceParameterPort {
                        ordinal: parameter.ordinal,
                        name: parameter.name.clone(),
                        binding: parameter.binding,
                        shape,
                    });
                }
                self.captures.push(SourceCaptureTemplate {
                    event,
                    literal_owner,
                    literal_span: declaration.literal_span.clone(),
                    signature,
                    inputs,
                    parameters: formal_parameters,
                });
            }
        }
        Ok(())
    }

    fn capture_shape(
        &mut self,
        event: &SourceEventId,
        producer: &ProducerCfgOwnerId,
        ty: &ScalarType,
        structs: &[ScalarStruct],
        enums: &[ScalarEnum],
        context: &SourceCapturePolicyContext,
    ) -> Result<ValueShapeId, SourceEpochError> {
        intern_value_shape(&mut self.shapes, producer, ty, structs, enums, &context.generic_scope, &context.generic_declarations).map_err(|error| {
            SourceEpochError::CaptureShapeConstruction {
                event: event.clone(),
                producer: producer.clone(),
                ty: ty.clone(),
                error,
            }
        })
    }
}
