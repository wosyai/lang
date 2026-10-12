use super::*;

impl ReferenceFlowBuilder {
    pub(super) fn evaluate_aggregate_member(
        &mut self,
        expression: &ScalarExpression,
        scope: ReferenceScopeId,
    ) -> Result<assignment::EvaluatedOutputEntry, ReferenceAnalysisError> {
        let source_span = SourceSpan::new(self.source.clone(), expression_span(expression));
        let mut declarations = self
            .value_types
            .expressions
            .iter()
            .filter(|fact| fact.source_span == source_span);
        let declaration = declarations.next().ok_or_else(|| {
            self.missing_context(
                source_span.range,
                ReferenceAnalysisErrorReason::MissingResolvedBinding,
            )
        })?;
        if declarations.next().is_some() || declaration.outputs.outputs.len() != 1 {
            return Err(self.missing_context(
                source_span.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        }
        let outputs = declaration.outputs.clone();
        let mut entries = self
            .evaluate_outputs(expression, &outputs, scope)?
            .entries
            .into_iter();
        let entry = entries.next().ok_or_else(|| {
            self.missing_context(
                source_span.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            )
        })?;
        if entries.next().is_some() {
            return Err(self.missing_context(
                source_span.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        }
        Ok(entry)
    }

    fn assignment_selectors(
        &mut self,
        place: &ScalarPlace,
        scope: ReferenceScopeId,
    ) -> Result<assignment::EvaluatedAssignmentSelectors, ReferenceAnalysisError> {
        match place {
            ScalarPlace::Name { .. } => Ok(assignment::EvaluatedAssignmentSelectors {
                root: assignment::EvaluatedAssignmentRoot::Direct,
                indexes: Vec::new(),
            }),
            ScalarPlace::Field { base, .. } => self.assignment_selectors(base, scope),
            ScalarPlace::Dereference { pointer, span } => {
                let source_span = SourceSpan::new(self.source.clone(), expression_span(pointer));
                let outputs = self
                    .value_types
                    .expressions
                    .iter()
                    .find(|fact| fact.source_span == source_span)
                    .ok_or_else(|| {
                        self.missing_context(
                            *span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?
                    .outputs
                    .clone();
                let evaluated = self.evaluate_outputs(pointer, &outputs, scope)?;
                let mut entries = evaluated.entries.into_iter();
                let entry = entries.next().ok_or_else(|| {
                    self.missing_context(
                        *span,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    )
                })?;
                Ok(assignment::EvaluatedAssignmentSelectors {
                    root: assignment::EvaluatedAssignmentRoot::Through(entry),
                    indexes: Vec::new(),
                })
            }
            ScalarPlace::Index {
                base,
                index,
                index_span,
                ..
            } => {
                let mut selectors = self.assignment_selectors(base, scope)?;
                let source_span = SourceSpan::new(self.source.clone(), expression_span(index));
                let outputs = self
                    .value_types
                    .expressions
                    .iter()
                    .find(|fact| fact.source_span == source_span)
                    .ok_or_else(|| {
                        self.missing_context(
                            *index_span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?
                    .outputs
                    .clone();
                let evaluated = self.evaluate_outputs(index, &outputs, scope)?;
                self.index_points
                    .insert(*index_span, evaluated.entries[0].value.point);
                selectors
                    .indexes
                    .extend(evaluated.entries.into_iter().map(|entry| entry.value));
                Ok(selectors)
            }
        }
    }

    fn evaluate_outputs(
        &mut self,
        expression: &ScalarExpression,
        outputs: &ScalarOutputSequence,
        scope: ReferenceScopeId,
    ) -> Result<assignment::EvaluatedOutputSequence, ReferenceAnalysisError> {
        self.expression(expression, scope)?;
        if matches!(expression, ScalarExpression::Call { .. }) {
            let sequence = self.call_output_sequence(expression)?;
            if sequence.entries.len() != outputs.outputs.len() {
                return Err(self.missing_context(
                    expression_span(expression),
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
            for (entry, output) in sequence.entries.iter().zip(&outputs.outputs) {
                let shape = self
                    .cfg
                    .evaluated_values
                    .shapes
                    .iter()
                    .find(|shape| shape.id == entry.shape)
                    .expect("canonical call source shape");
                if !scalar_type_equal(&shape.ty, &output.ty) {
                    return Err(self.missing_context(
                        expression_span(expression),
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
            }
            return Ok(sequence);
        }
        if let ScalarExpression::Assignment(assignment) = expression {
            let evaluation = self
                .cfg
                .evaluated_values
                .assignment_evaluations
                .iter()
                .find(|evaluation| evaluation.operation.occurrence == *assignment.occurrence)
                .expect("producing assignment evaluation");
            let mut entries = Vec::new();
            for (output, resolved) in outputs.outputs.iter().enumerate() {
                let result = evaluation
                    .results
                    .iter()
                    .find(|result| result.output == output)
                    .expect("assignment returned output");
                let fact = self
                    .cfg
                    .evaluated_values
                    .values
                    .iter()
                    .find(|fact| fact.id == result.value)
                    .expect("assignment returned value fact");
                let shape = self
                    .cfg
                    .evaluated_values
                    .shapes
                    .iter()
                    .find(|shape| shape.id == fact.shape)
                    .expect("assignment returned shape");
                if !scalar_type_equal(&shape.ty, &resolved.ty) {
                    return Err(self.missing_context(
                        expression_span(expression),
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
                entries.push(assignment::EvaluatedOutputEntry {
                    value: result.value.clone(),
                    shape: fact.shape.clone(),
                });
            }
            return Ok(assignment::EvaluatedOutputSequence { entries });
        }
        let producer_point = self.cursor.expect("evaluated output producer");
        let sequence = self.bind_final_output_values(expression, producer_point)?;
        if sequence.entries.len() != outputs.outputs.len() {
            return Err(self.missing_context(
                expression_span(expression),
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        }
        for (entry, resolved) in sequence.entries.iter().zip(&outputs.outputs) {
            self.validate_final_output_entry(expression, entry, &resolved.ty)?;
            let shapes = self
                .cfg
                .evaluated_values
                .shapes
                .iter()
                .filter(|shape| shape.id == entry.shape)
                .collect::<Vec<_>>();
            let [shape] = shapes.as_slice() else {
                return Err(self.missing_context(
                    expression_span(expression),
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            };
            if !scalar_type_equal(&shape.ty, &resolved.ty) {
                return Err(self.missing_context(
                    expression_span(expression),
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
        }
        Ok(sequence)
    }

    pub(super) fn assignment(
        &mut self,
        assignment: &ScalarAssignment,
        block_span: ByteSpan,
        scope: ReferenceScopeId,
        declared: &mut Vec<(String, ReferenceBindingId, Option<ReferenceBindingId>)>,
        observed: bool,
    ) -> Result<(), ReferenceAnalysisError> {
        (|| -> Result<(), ReferenceAnalysisError> {
            let output_types = self.assignment_output_types(assignment)?;
            let operation = AssignmentOperationId {
                producer: self.cfg.producer_owner.clone(),
                occurrence: assignment.occurrence.as_ref().clone(),
            };
            let mut evaluation = ScalarAssignmentEvaluation {
                operation: operation.clone(),
                rhs: Vec::new(),
                destinations: Vec::new(),
                previous: Vec::new(),
                writes: Vec::new(),
                results: Vec::new(),
            };
            let mut plan = ResolvedAssignmentPlan {
                operation: operation.clone(),
                targets: Vec::new(),
                replacements: Vec::new(),
                previous_outputs: Vec::new(),
                consumption: if observed {
                    AssignmentConsumption::Received {
                        outputs: Vec::new(),
                        discards: Vec::new(),
                    }
                } else {
                    AssignmentConsumption::Discarded
                },
            };
            let mut values = Vec::new();
            for (expression_ordinal, value) in assignment.values.iter().enumerate() {
                let source_span = SourceSpan::new(self.source.clone(), expression_span(value));
                let outputs = self
                    .value_types
                    .expressions
                    .iter()
                    .find(|fact| fact.source_span == source_span)
                    .ok_or_else(|| {
                        self.missing_context(
                            source_span.range,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?
                    .outputs
                    .clone();
                let evaluated = self.evaluate_outputs(value, &outputs, scope)?;
                self.move_value(value, scope)?;
                for (producer_output, entry) in evaluated.entries.into_iter().enumerate() {
                    evaluation.rhs.push(AssignmentRhsEvaluation {
                        expression_ordinal,
                        producer_output,
                        value: entry.value.clone(),
                    });
                    plan.replacements.push(AssignmentReplacementProjection {
                        expression_ordinal,
                        producer_output,
                        destination_ordinal: values.len(),
                        source_type: outputs.outputs[producer_output].ty.clone(),
                    });
                    let point = entry.value.point;
                    let result = match &self.cfg.points[point.0].possible_origins {
                        Some(origins) if !origins.is_empty() => {
                            ReferenceValue::Checked(origins.clone())
                        }
                        Some(_) => ReferenceValue::Other,
                        None => ReferenceValue::Uncomputed,
                    };
                    values.push((result, point));
                }
            }
            for (position, target) in assignment.targets.iter().enumerate() {
                let mut inferred = false;
                if let ScalarPlace::Name { name, .. } = &target.place {
                    if !self.names.contains_key(name) && !self.module_names.contains(name) {
                        if observed {
                            return Err(self.missing_context(
                                target.target_span,
                                ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            ));
                        }
                        let id = ReferenceBindingId {
                            source: self.source.clone(),
                            function_span: self.function_span,
                            declaration_span: target.target_span,
                            block_span,
                            kind: ReferenceBindingKind::Declared,
                        };
                        self.names.insert(name.clone(), id.clone());
                        self.binding_types
                            .insert(id.clone(), output_types[position].clone());
                        if matches!(output_types[position], ScalarType::CheckedReference { .. }) {
                            self.checked_storage(&id, target.place.clone());
                        }
                        declared.push((name.clone(), id, None));
                        inferred = true;
                    }
                }
                let selectors = self.assignment_selectors(&target.place, scope)?;
                if let Some(selected) =
                    self.select_callable_array_element(&target.place, target.target_span, scope)?
                {
                    self.assignment_point(
                        assignment,
                        position,
                        ScalarAssignmentPointRole::SelectElement,
                        selected,
                    );
                }
                let shape = intern_value_shape(
                    &mut self.cfg.evaluated_values.shapes,
                    &self.cfg.producer_owner,
                    &output_types[position],
                    &self.structs,
                    &self.enums,
                    &self.generic_scope,
                    &self.generic_declarations,
                )
                .map_err(|_| {
                    self.missing_context(
                        target.target_span,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })?;
                let storage = match selectors.root {
                    assignment::EvaluatedAssignmentRoot::Through(entry) => {
                        EvaluatedDestinationStorage::Through {
                            pointer: entry.value,
                            cases: Vec::new(),
                        }
                    }
                    assignment::EvaluatedAssignmentRoot::Direct => {
                        let place = self.assignment_place(&target.place, target.target_span)?;
                        if inferred {
                            EvaluatedDestinationStorage::Inferred(place.binding)
                        } else {
                            EvaluatedDestinationStorage::Direct(place)
                        }
                    }
                };
                let point = self.point(
                    CfgPointKind::AssignmentDestination {
                        storage: storage.clone(),
                        place: target.place.clone(),
                    },
                    target.target_span,
                    scope,
                    BTreeSet::new(),
                );
                self.assignment_point(
                    assignment,
                    position,
                    ScalarAssignmentPointRole::Destination,
                    point,
                );
                let source_span = SourceSpan::new(self.source.clone(), target.target_span);
                let resolved_storage = match &storage {
                    EvaluatedDestinationStorage::Direct(place) => {
                        ResolvedAssignmentStorage::Existing(place.clone())
                    }
                    EvaluatedDestinationStorage::Inferred(binding) => {
                        ResolvedAssignmentStorage::InferredDeclaration(binding.clone())
                    }
                    EvaluatedDestinationStorage::Through { pointer, .. } => {
                        ResolvedAssignmentStorage::Through {
                            pointer: pointer.resolved.clone(),
                        }
                    }
                };
                plan.targets.push(ResolvedAssignmentTarget {
                    ordinal: position,
                    source_span: source_span.clone(),
                    place: target.place.clone(),
                    ty: output_types[position].clone(),
                    storage: resolved_storage,
                });
                evaluation
                    .destinations
                    .push(EvaluatedAssignmentDestination {
                        id: AssignmentDestinationId {
                            operation: operation.clone(),
                            target: position,
                        },
                        point,
                        source_span,
                        shape,
                        storage,
                        selector_inputs: selectors.indexes,
                    });
            }
            validate_assignment_destinations(
                &self.cfg,
                &plan,
                &evaluation,
                &self.value_types.expressions,
                &self.structs,
            )
            .map_err(|error| ReferenceAnalysisError::MissingContext {
                source_span: error.source_span,
                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            })?;
            for (position, destination) in evaluation.destinations.iter().enumerate() {
                if let EvaluatedDestinationStorage::Inferred(binding) = &destination.storage {
                    plan.previous_outputs.push(AssignmentPreviousPlan {
                        target: position,
                        previous: PreviousOutputPlan::InferredDeclaration {
                            binding: binding.clone(),
                        },
                    });
                    evaluation.previous.push(AssignmentSnapshot {
                        destination: destination.id.clone(),
                        point: destination.point,
                        state: SnapshotState::NoPriorDeclaration,
                    });
                    continue;
                }
                let snapshot_point = CfgPointId(self.cfg.points.len());
                let holder = ReferenceBindingId {
                    source: self.source.clone(),
                    function_span: self.function_span,
                    declaration_span: destination.source_span.range,
                    block_span,
                    kind: ReferenceBindingKind::AssignmentPrevious {
                        snapshot: snapshot_point,
                        output: position,
                    },
                };
                let point = self.point_with_origins(
                    CfgPointKind::AssignmentSnapshot {
                        destination_point: destination.point,
                        holder,
                        observed,
                    },
                    destination.source_span.range,
                    scope,
                    None,
                );
                self.assignment_point(
                    assignment,
                    position,
                    ScalarAssignmentPointRole::PreviousSnapshot,
                    point,
                );
                let resolved = ResolvedValueOutputId {
                    producer: self.cfg.producer_owner.clone(),
                    occurrence: SourceSpan::new(self.source.clone(), assignment.span),
                    output_projection: assignment.occurrence.output_projection.clone(),
                    output: position,
                };
                let value =
                    bind_evaluated_value_id(&self.cfg, resolved.clone(), point).map_err(|_| {
                        self.missing_context(
                            assignment.span,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                self.cfg.evaluated_values.values.push(EvaluatedValueFact {
                    id: value.clone(),
                    shape: destination.shape.clone(),
                    cases: Vec::new(),
                });
                plan.previous_outputs.push(AssignmentPreviousPlan {
                    target: position,
                    previous: PreviousOutputPlan::Existing {
                        output: resolved,
                        shape: destination.shape.clone(),
                    },
                });
                evaluation.previous.push(AssignmentSnapshot {
                    destination: destination.id.clone(),
                    point,
                    state: SnapshotState::Saved(value),
                });
            }
            for (position, (target, (value, rhs_point))) in
                assignment.targets.iter().zip(values).enumerate()
            {
                if let ScalarPlace::Name { span, .. } = &target.place {
                    let target_place = match &evaluation.destinations[position].storage {
                        EvaluatedDestinationStorage::Direct(place) => place.clone(),
                        EvaluatedDestinationStorage::Inferred(binding) => ReferencePlaceId {
                            source: binding.source.clone(),
                            function_span: binding.function_span,
                            declaration_span: binding.declaration_span,
                            block_span: binding.block_span,
                            binding: binding.clone(),
                            projections: Vec::new(),
                            place: target.place.clone(),
                        },
                        EvaluatedDestinationStorage::Through { .. } => {
                            unreachable!("name destination")
                        }
                    };
                    let id = target_place.binding.clone();
                    let previous_origins = self.origins.get(&id).cloned().unwrap_or_default();
                    let previous_loans = previous_origins
                        .iter()
                        .filter_map(|origin| match &self.cfg.facts[origin.0] {
                            ReferenceOrigin::Fresh { loan, .. }
                            | ReferenceOrigin::BorrowedFrom { loan, .. } => Some(loan.clone()),
                            ReferenceOrigin::Null { .. } | ReferenceOrigin::Invalid { .. } => None,
                        })
                        .collect();
                    let assign = CfgPointKind::Assign {
                        target: target_place,
                        rhs_point,
                        previous_origins,
                        previous_loans,
                    };
                    if let ReferenceValue::Checked(origins) = value {
                        self.checked.insert(id.clone());
                        self.origins.insert(id.clone(), origins.clone());
                        self.point(assign, *span, scope, origins);
                    } else if matches!(value, ReferenceValue::Uncomputed)
                        && self.checked.contains(&id)
                    {
                        self.checked.insert(id.clone());
                        self.origins.remove(&id);
                        self.uncomputed_point(assign, *span, scope);
                    } else {
                        if self.checked.contains(&id) {
                            return Err(self.missing_origin(
                                *span,
                                id,
                                ReferenceAnalysisErrorReason::MissingCheckedReferenceOrigin,
                            ));
                        }
                        self.origins.remove(&id);
                        self.point(assign, *span, scope, BTreeSet::new());
                    }
                    self.assignment_point(
                        assignment,
                        position,
                        ScalarAssignmentPointRole::Assign,
                        self.cursor.expect("assigned name"),
                    );
                    if matches!(&output_types[position], ScalarType::Array { element, .. } if matches!(element.as_ref(), ScalarType::Callable { .. }))
                    {
                        if let Some(ScalarExpression::Name {
                            name: source_name, ..
                        }) = assignment.values.get(position)
                        {
                            let source = self.binding(source_name, target.target_span)?;
                            let copy = self.point(
                                CfgPointKind::CopyCallableFields {
                                    source: CallableFieldSource::Assignment(source),
                                    target: id.clone(),
                                },
                                target.target_span,
                                scope,
                                BTreeSet::new(),
                            );
                            self.assignment_point(
                                assignment,
                                position,
                                ScalarAssignmentPointRole::CopyCallableFields,
                                copy,
                            );
                        }
                    }
                    if matches!(output_types[position], ScalarType::Struct(_))
                        && position < assignment.values.len()
                    {
                        let source = match &assignment.values[position] {
                            ScalarExpression::Name { name, .. } => {
                                self.names.get(name).cloned().or_else(|| {
                                    self.global_bindings
                                        .get(name)
                                        .map(|(binding, _)| binding.clone())
                                })
                            }
                            ScalarExpression::Call { .. } => self
                                .cfg
                                .points
                                .iter()
                                .rev()
                                .find(|point| {
                                    point.source_span.range
                                        == expression_span(&assignment.values[position])
                                        && matches!(
                                            point.kind,
                                            CfgPointKind::Call { output: 0, .. }
                                        )
                                })
                                .map(|point| ReferenceBindingId {
                                    source: self.source.clone(),
                                    function_span: self.function_span,
                                    declaration_span: point.source_span.range,
                                    block_span: self.function_span,
                                    kind: ReferenceBindingKind::CallOutput {
                                        point: point.id,
                                        output: 0,
                                    },
                                }),
                            _ => None,
                        };
                        if let Some(source) = source {
                            let copy_fields = self.point(
                                CfgPointKind::CopyCallableFields {
                                    source: CallableFieldSource::Assignment(source),
                                    target: id.clone(),
                                },
                                *span,
                                scope,
                                BTreeSet::new(),
                            );
                            self.assignment_point(
                                assignment,
                                position,
                                ScalarAssignmentPointRole::CopyCallableFields,
                                copy_fields,
                            );
                        }
                    }
                    if matches!(
                        output_types[position],
                        ScalarType::RuntimeArray { .. } | ScalarType::Array { .. }
                    ) && position < assignment.values.len()
                    {
                        if let ScalarExpression::Name {
                            name: source_name,
                            span: source_span,
                        } = &assignment.values[position]
                        {
                            if self.names.get(source_name).is_some_and(|source| self.cfg.backing_parameters.contains(source) || self.cfg.points.iter().any(|point| matches!(&point.kind, CfgPointKind::Backing { target, .. } if target == source))) {
                                    let source = self.names.get(source_name).ok_or_else(|| {
                                        self.missing_context(
                                            *source_span,
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        )
                                    })?;
                                    let source_ty = self.binding_types.get(source).ok_or_else(|| self.missing_context(*source_span, ReferenceAnalysisErrorReason::MissingResolvedBinding))?;
                                    let transfer = match copy_policy(
                                        source_ty,
                                        &self.structs,
                                        &self.enums,
                                        &self.generic_parameters,
                                    )
                                    .map_err(|_| {
                                        self.missing_context(
                                            *source_span,
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        )
                                    })? {
                                        CopyPolicy::Copy => RuntimeBackingTransfer::CopyFixed {
                                            source: source.clone(),
                                        },
                                        CopyPolicy::Move => RuntimeBackingTransfer::Move {
                                            source: source.clone(),
                                        },
                                        CopyPolicy::Deferred => return Err(self.missing_context(
                                            *source_span,
                                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                        )),
                                    };
                                    self.point(
                                        CfgPointKind::Backing {
                                            target: id.clone(),
                                            transfer,
                                            materialize: matches!(
                                                self.binding_types.get(&id),
                                                Some(ScalarType::RuntimeArray { .. })
                                            ),
                                        },
                                        *span,
                                        scope,
                                        BTreeSet::new(),
                                    );
                                    }
                        }
                    }
                    if matches!(output_types[position], ScalarType::RuntimeArray { .. }) {
                        if let CfgPointKind::Call { output, .. } =
                            &self.cfg.points[rhs_point.0].kind
                        {
                            let output = *output;
                            let call_span = self.cfg.points[rhs_point.0].source_span.range;
                            self.point(
                                CfgPointKind::Backing {
                                    target: id.clone(),
                                    transfer: RuntimeBackingTransfer::Move {
                                        source: ReferenceBindingId {
                                            source: self.source.clone(),
                                            function_span: self.function_span,
                                            declaration_span: call_span,
                                            block_span: self.cfg.points[0].source_span.range,
                                            kind: ReferenceBindingKind::CallOutput {
                                                point: rhs_point,
                                                output,
                                            },
                                        },
                                    },
                                    materialize: true,
                                },
                                target.target_span,
                                scope,
                                BTreeSet::new(),
                            );
                        }
                    }
                } else {
                    if let ScalarPlace::Name { name, span } = reference_root(&target.place) {
                        if self.module_names.contains(name)
                            && !self.names.contains_key(name)
                            && !self.global_bindings.contains_key(name)
                        {
                            self.point(
                                CfgPointKind::Evaluate(ScalarExpression::Name {
                                    name: name.clone(),
                                    span: *span,
                                }),
                                target.target_span,
                                scope,
                                BTreeSet::new(),
                            );
                            continue;
                        }
                    }
                    if let ScalarPlace::Dereference { .. } = reference_root(&target.place) {
                        let EvaluatedDestinationStorage::Through { pointer, .. } =
                            &evaluation.destinations[position].storage
                        else {
                            unreachable!("through destination")
                        };
                        let pointer_point = pointer.point;
                        let assign = CfgPointKind::AssignThrough {
                            place: target.place.clone(),
                            pointer_point,
                            rhs_point,
                            candidates: Vec::new(),
                            previous_origins: BTreeSet::new(),
                            previous_loans: Vec::new(),
                        };
                        match value {
                            ReferenceValue::Checked(origins) => {
                                self.point(assign, target.target_span, scope, origins);
                            }
                            ReferenceValue::Uncomputed
                                if matches!(
                                    output_types[position],
                                    ScalarType::CheckedReference { .. }
                                ) =>
                            {
                                self.uncomputed_point(assign, target.target_span, scope);
                            }
                            _ => {
                                self.point(assign, target.target_span, scope, BTreeSet::new());
                            }
                        }
                        self.assignment_point(
                            assignment,
                            position,
                            ScalarAssignmentPointRole::AssignThrough,
                            self.cursor.expect("assigned through pointer"),
                        );
                    } else {
                        let EvaluatedDestinationStorage::Direct(target_place) =
                            &evaluation.destinations[position].storage
                        else {
                            unreachable!("direct destination")
                        };
                        let target_place = target_place.clone();
                        let assign = CfgPointKind::Assign {
                            target: target_place,
                            rhs_point,
                            previous_origins: BTreeSet::new(),
                            previous_loans: Vec::new(),
                        };
                        match value {
                            ReferenceValue::Checked(origins) => {
                                self.point(assign, target.target_span, scope, origins);
                            }
                            ReferenceValue::Uncomputed
                                if matches!(
                                    output_types[position],
                                    ScalarType::CheckedReference { .. }
                                ) =>
                            {
                                self.uncomputed_point(assign, target.target_span, scope);
                            }
                            _ => {
                                self.point(assign, target.target_span, scope, BTreeSet::new());
                            }
                        }
                        self.assignment_point(
                            assignment,
                            position,
                            ScalarAssignmentPointRole::Assign,
                            self.cursor.expect("assigned place"),
                        );
                    }
                }
            }
            for (position, destination) in evaluation.destinations.iter().enumerate() {
                let point = self
                    .cfg
                    .assignment_points
                    .iter()
                    .find(|entry| {
                        entry.occurrence == *assignment.occurrence
                            && entry.target == position
                            && matches!(
                                entry.role,
                                ScalarAssignmentPointRole::Assign
                                    | ScalarAssignmentPointRole::AssignThrough
                            )
                    })
                    .expect("assignment commit correspondence")
                    .point;
                evaluation.writes.push(AssignmentCommit {
                    destination: destination.id.clone(),
                    point,
                    replacement: evaluation.rhs[position].value.clone(),
                    previous: evaluation.previous[position].point,
                    dispositions: Vec::new(),
                });
            }
            for (position, snapshot) in evaluation.previous.iter().enumerate() {
                if let SnapshotState::Saved(saved) = &snapshot.state {
                    if observed {
                        let point = self.point_with_origins(
                            CfgPointKind::AssignmentResult {
                                snapshot_point: snapshot.point,
                                output: position,
                            },
                            assignment.span,
                            scope,
                            None,
                        );
                        self.assignment_point(
                            assignment,
                            position,
                            ScalarAssignmentPointRole::Result,
                            point,
                        );
                        let value =
                            bind_evaluated_value_id(&self.cfg, saved.resolved.clone(), point)
                                .map_err(|_| {
                                    self.missing_context(
                                        assignment.span,
                                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                                    )
                                })?;
                        self.cfg.evaluated_values.values.push(EvaluatedValueFact {
                            id: value.clone(),
                            shape: evaluation.destinations[position].shape.clone(),
                            cases: Vec::new(),
                        });
                        evaluation.results.push(AssignmentResult {
                            operation: operation.clone(),
                            output: position,
                            snapshot: snapshot.point,
                            value,
                            consumers: Vec::new(),
                        });
                    } else {
                        let point = self.point(
                            CfgPointKind::AssignmentDispose {
                                snapshot_point: snapshot.point,
                            },
                            assignment.span,
                            scope,
                            BTreeSet::new(),
                        );
                        self.assignment_point(
                            assignment,
                            position,
                            ScalarAssignmentPointRole::Dispose,
                            point,
                        );
                    }
                }
            }
            self.cfg
                .evaluated_values
                .assignment_previous
                .extend(plan.previous_outputs.clone());
            self.cfg
                .evaluated_values
                .destinations
                .extend(evaluation.destinations.clone());
            self.cfg
                .evaluated_values
                .snapshots
                .extend(evaluation.previous.clone());
            self.cfg
                .evaluated_values
                .commits
                .extend(evaluation.writes.clone());
            self.cfg
                .evaluated_values
                .results
                .extend(evaluation.results.clone());
            self.cfg.evaluated_values.assignment_plans.push(plan);
            self.cfg
                .evaluated_values
                .assignment_evaluations
                .push(evaluation);
            Ok(())
        })()?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "correction_tests.rs"]
mod correction_tests;
