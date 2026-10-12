use super::*;
use reference::ReferenceFlowBuilder;

impl ReferenceFlowBuilder {
    fn final_assignment_point_reaches(&self, start: CfgPointId, end: CfgPointId) -> bool {
        let mut pending = vec![start];
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == end {
                return true;
            }
            if visited.insert(id) {
                if let Some(point) = self.cfg.points.get(id.0) {
                    pending.extend(point.successors.iter().copied());
                }
            }
        }
        false
    }

    fn final_assignment_result_indices(
        &self,
        assignment: &ScalarAssignment,
        entry: &assignment::EvaluatedOutputEntry,
    ) -> Result<(usize, usize, usize, usize), ReferenceAnalysisError> {
        let invalid = || {
            self.missing_context(
                assignment.span,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            )
        };
        let operation = AssignmentOperationId {
            producer: self.cfg.producer_owner.clone(),
            occurrence: (*assignment.occurrence).clone(),
        };
        let evaluations = self
            .cfg
            .evaluated_values
            .assignment_evaluations
            .iter()
            .enumerate()
            .filter(|(_, row)| row.operation == operation)
            .collect::<Vec<_>>();
        let [(evaluation_index, evaluation)] = evaluations.as_slice() else {
            return Err(invalid());
        };
        let results = evaluation
            .results
            .iter()
            .enumerate()
            .filter(|(_, row)| row.value == entry.value)
            .collect::<Vec<_>>();
        let [(result_index, result)] = results.as_slice() else {
            return Err(invalid());
        };
        let registered = self
            .cfg
            .evaluated_values
            .results
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row.value == entry.value
                    || (row.operation == operation && row.output == entry.value.resolved.output)
            })
            .collect::<Vec<_>>();
        let [(registered_index, registered)] = registered.as_slice() else {
            return Err(invalid());
        };
        let plans = self
            .cfg
            .evaluated_values
            .assignment_plans
            .iter()
            .enumerate()
            .filter(|(_, row)| row.operation == operation)
            .collect::<Vec<_>>();
        let [(plan_index, plan)] = plans.as_slice() else {
            return Err(invalid());
        };
        if !matches!(plan.consumption, AssignmentConsumption::Received { .. })
            || result != registered
            || result.operation != operation
            || result.output != entry.value.resolved.output
            || evaluation
                .results
                .iter()
                .filter(|row| row.output == result.output)
                .count()
                != 1
            || entry.value.resolved.producer != operation.producer
            || entry.value.resolved.occurrence != operation.occurrence.source_span
            || entry.value.resolved.output_projection != operation.occurrence.output_projection
            || operation.occurrence.source_span
                != SourceSpan::new(self.source.clone(), assignment.span)
            || self
                .cfg
                .evaluated_values
                .assignment_evaluations
                .iter()
                .flat_map(|row| &row.results)
                .filter(|row| row.value == entry.value)
                .count()
                != 1
        {
            return Err(invalid());
        }
        let snapshots = evaluation
            .previous
            .iter()
            .filter(|row| row.point == result.snapshot)
            .collect::<Vec<_>>();
        let [snapshot] = snapshots.as_slice() else {
            return Err(invalid());
        };
        let registered_snapshots = self
            .cfg
            .evaluated_values
            .snapshots
            .iter()
            .filter(|row| row.point == result.snapshot)
            .collect::<Vec<_>>();
        let [registered_snapshot] = registered_snapshots.as_slice() else {
            return Err(invalid());
        };
        let SnapshotState::Saved(saved) = &snapshot.state else {
            return Err(invalid());
        };
        let previous = plan
            .previous_outputs
            .iter()
            .filter(|row| row.target == result.output)
            .collect::<Vec<_>>();
        let [previous] = previous.as_slice() else {
            return Err(invalid());
        };
        let PreviousOutputPlan::Existing { output, shape } = &previous.previous else {
            return Err(invalid());
        };
        let destinations = evaluation
            .destinations
            .iter()
            .filter(|row| row.id == snapshot.destination)
            .collect::<Vec<_>>();
        let [destination] = destinations.as_slice() else {
            return Err(invalid());
        };
        let saved_facts = self
            .cfg
            .evaluated_values
            .values
            .iter()
            .filter(|row| row.id == *saved)
            .collect::<Vec<_>>();
        let [saved_fact] = saved_facts.as_slice() else {
            return Err(invalid());
        };
        let result_facts = self
            .cfg
            .evaluated_values
            .values
            .iter()
            .filter(|row| row.id == entry.value)
            .collect::<Vec<_>>();
        let [result_fact] = result_facts.as_slice() else {
            return Err(invalid());
        };
        let shapes = self
            .cfg
            .evaluated_values
            .shapes
            .iter()
            .filter(|row| row.id == entry.shape)
            .collect::<Vec<_>>();
        let [actual_shape] = shapes.as_slice() else {
            return Err(invalid());
        };
        let snapshot_points = self
            .cfg
            .points
            .iter()
            .filter(|row| row.id == result.snapshot)
            .collect::<Vec<_>>();
        let [snapshot_point] = snapshot_points.as_slice() else {
            return Err(invalid());
        };
        let result_points = self
            .cfg
            .points
            .iter()
            .filter(|row| row.id == entry.value.point)
            .collect::<Vec<_>>();
        let [result_point] = result_points.as_slice() else {
            return Err(invalid());
        };
        if snapshot != registered_snapshot
            || snapshot.destination.operation != operation
            || snapshot.destination.target != result.output
            || saved.point != result.snapshot
            || saved.resolved != entry.value.resolved
            || saved == &entry.value
            || output != &saved.resolved
            || shape != &entry.shape
            || saved_fact.shape != entry.shape
            || result_fact.shape != entry.shape
            || destination.shape != entry.shape
            || actual_shape.id.producer != operation.producer
            || snapshot_point.source_span != destination.source_span
            || result_point.source_span != operation.occurrence.source_span
            || snapshot_point.scope != result_point.scope
            || result.snapshot >= entry.value.point
            || !matches!(&snapshot_point.kind, CfgPointKind::AssignmentSnapshot { destination_point, observed: true, .. }
                if *destination_point == destination.point)
            || !matches!(&result_point.kind, CfgPointKind::AssignmentResult { snapshot_point, output }
                if *snapshot_point == result.snapshot && *output == result.output)
            || !self.final_assignment_point_reaches(destination.point, snapshot.point)
            || !self.final_assignment_point_reaches(snapshot.point, result.value.point)
        {
            return Err(invalid());
        }
        Ok((
            *evaluation_index,
            *result_index,
            *registered_index,
            *plan_index,
        ))
    }

    pub(super) fn register_final_assignment_consumer(
        &mut self,
        assignment: &ScalarAssignment,
        entry: &assignment::EvaluatedOutputEntry,
        consumer: &ValueConsumerId,
    ) -> Result<(), ReferenceAnalysisError> {
        let (evaluation, result, registered, plan) =
            self.final_assignment_result_indices(assignment, entry)?;
        let invalid = || {
            self.missing_context(
                assignment.span,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            )
        };
        let row = &self.cfg.evaluated_values.assignment_evaluations[evaluation].results[result];
        let points = self
            .cfg
            .points
            .iter()
            .filter(|point| point.id == consumer.point)
            .collect::<Vec<_>>();
        let [point] = points.as_slice() else {
            return Err(invalid());
        };
        let AssignmentConsumption::Received { outputs, discards } =
            &self.cfg.evaluated_values.assignment_plans[plan].consumption
        else {
            return Err(invalid());
        };
        let connected = match (&consumer.kind, &point.kind) {
            (
                ValueConsumerKind::FinalOutput { output, .. },
                CfgPointKind::ReturnOutput {
                    output: actual,
                    value_point,
                    ..
                },
            ) => output == actual && *value_point == entry.value.point,
            (ValueConsumerKind::Discard, CfgPointKind::AssignmentDispose { snapshot_point }) => {
                *snapshot_point == row.snapshot
                    && self
                        .cfg
                        .assignment_points
                        .iter()
                        .filter(|binding| {
                            binding.occurrence == *assignment.occurrence
                                && binding.target == row.output
                                && binding.role == ScalarAssignmentPointRole::Dispose
                                && binding.point == consumer.point
                        })
                        .count()
                        == 1
            }
            _ => false,
        };
        if !connected
            || consumer.producer != self.cfg.producer_owner
            || point.source_span != entry.value.resolved.occurrence
            || point.scope != ReferenceScopeId(0)
            || consumer.point <= entry.value.point
            || !row.consumers.is_empty()
            || !self.final_assignment_point_reaches(entry.value.point, consumer.point)
            || outputs.contains(consumer)
            || discards.contains(&entry.value.resolved)
            || outputs.iter().any(|output| {
                self.cfg.points.iter().any(|point| {
                    point.id == output.point
                        && matches!(&point.kind, CfgPointKind::ReturnOutput { value_point, .. }
                    if *value_point == entry.value.point)
                })
            })
        {
            return Err(invalid());
        }
        self.cfg.evaluated_values.assignment_evaluations[evaluation].results[result]
            .consumers
            .push(consumer.clone());
        self.cfg.evaluated_values.results[registered]
            .consumers
            .push(consumer.clone());
        let AssignmentConsumption::Received { outputs, discards } =
            &mut self.cfg.evaluated_values.assignment_plans[plan].consumption
        else {
            unreachable!("validated received plan")
        };
        match consumer.kind {
            ValueConsumerKind::FinalOutput { .. } => outputs.push(consumer.clone()),
            ValueConsumerKind::Discard => discards.push(entry.value.resolved.clone()),
            _ => unreachable!("validated final consumer"),
        }
        Ok(())
    }

    pub(super) fn bind_final_output_values(
        &mut self,
        expression: &ScalarExpression,
        producer_point: CfgPointId,
    ) -> Result<assignment::EvaluatedOutputSequence, ReferenceAnalysisError> {
        if matches!(expression, ScalarExpression::If { .. } | ScalarExpression::Block(_)) {
            let sequence = self.control_output_sequence(expression)?;
            if sequence.entries.first().map(|entry| entry.value.point) != Some(producer_point) {
                return Err(self.missing_context(expression_span(expression), ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence));
            }
            return Ok(sequence);
        }
        let occurrence = SourceSpan::new(self.source.clone(), expression_span(expression));
        let mut facts = self
            .value_types
            .expressions
            .iter()
            .filter(|fact| fact.source_span == occurrence);
        let outputs = facts
            .next()
            .ok_or_else(|| {
                self.missing_context(
                    occurrence.range,
                    ReferenceAnalysisErrorReason::MissingResolvedBinding,
                )
            })?
            .outputs
            .clone();
        if facts.next().is_some() {
            return Err(self.missing_context(
                occurrence.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        }
        if matches!(expression, ScalarExpression::Call { .. })
            && self
                .cfg
                .evaluated_values
                .productions
                .iter()
                .any(|production| {
                    production.producer == self.cfg.producer_owner
                        && production.occurrence == occurrence
                })
        {
            let sequence = self.call_output_sequence(expression)?;
            if sequence.entries.len() != outputs.outputs.len()
                || sequence.entries[0].value.point != producer_point
            {
                return Err(self.missing_context(
                    occurrence.range,
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
                    .ok_or_else(|| {
                        self.missing_context(
                            occurrence.range,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                if !scalar_type_equal(&shape.ty, &output.ty) {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
            }
            self.validate_final_output_sequence(expression, &sequence, &outputs)?;
            return Ok(sequence);
        }
        let projection = match expression {
            ScalarExpression::Call { occurrence, .. } => &occurrence.output_projection,
            ScalarExpression::Assignment(assignment) => &assignment.occurrence.output_projection,
            ScalarExpression::Block(block) => &block.occurrence.output_projection,
            ScalarExpression::Dereference { occurrence, .. }
            | ScalarExpression::IndexedRead { occurrence, .. } => &occurrence.output_projection,
            _ => &Vec::new(),
        };
        let mut entries = Vec::new();
        for (output, resolved) in outputs.outputs.iter().enumerate() {
            let id = if let ScalarExpression::Assignment(assignment) = expression {
                let evaluations = self
                    .cfg
                    .evaluated_values
                    .assignment_evaluations
                    .iter()
                    .filter(|evaluation| {
                        evaluation.operation.producer == self.cfg.producer_owner
                            && evaluation.operation.occurrence == *assignment.occurrence
                    })
                    .collect::<Vec<_>>();
                let [evaluation] = evaluations.as_slice() else {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                };
                if evaluation.results.len() != outputs.outputs.len() {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
                let results = evaluation
                    .results
                    .iter()
                    .filter(|result| result.output == output)
                    .collect::<Vec<_>>();
                let [result] = results.as_slice() else {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                };
                if evaluation.results[output] != **result
                    || result.operation != evaluation.operation
                {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
                let expected = ResolvedValueOutputId {
                    producer: self.cfg.producer_owner.clone(),
                    occurrence: occurrence.clone(),
                    output_projection: projection.clone(),
                    output,
                };
                if result.value.resolved != expected
                    || !self.cfg.points.iter().any(|point| {
                        point.id == result.value.point
                            && matches!(&point.kind,
                        CfgPointKind::AssignmentResult { snapshot_point, output: actual }
                            if *snapshot_point == result.snapshot && *actual == output)
                    })
                {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
                result.value.clone()
            } else {
                let point = if let ScalarExpression::Call { .. } = expression {
                    let mut points = self.cfg.points.iter().filter(|point| {
                        matches!(&point.kind, CfgPointKind::Call {
                            expression: actual, output: actual_output, ..
                        } if actual == expression && *actual_output == output)
                    });
                    let point = points
                        .next()
                        .ok_or_else(|| {
                            self.missing_context(
                                occurrence.range,
                                ReferenceAnalysisErrorReason::MissingResolvedBinding,
                            )
                        })?
                        .id;
                    if points.next().is_some() || (output == 0 && point != producer_point) {
                        return Err(self.missing_context(
                            occurrence.range,
                            ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                        ));
                    }
                    point
                } else if output == 0 {
                    producer_point
                } else {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    ));
                };
                bind_evaluated_value_id(
                    &self.cfg,
                    ResolvedValueOutputId {
                        producer: self.cfg.producer_owner.clone(),
                        occurrence: occurrence.clone(),
                        output_projection: projection.clone(),
                        output,
                    },
                    point,
                )
                .map_err(|_| {
                    self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })?
            };
            evaluated_value::template::validate_shapes(&self.cfg.evaluated_values.shapes, &self.cfg.producer_owner, &self.generic_scope, &self.generic_declarations)
                .map_err(|_| self.missing_context(occurrence.range, ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence))?;
            let known = self.cfg.evaluated_values.values.iter().find(|fact| {
                if matches!(expression, ScalarExpression::Assignment(_)) {
                    fact.id == id
                } else {
                    fact.id.resolved == id.resolved
                }
            });
            let shape = if let Some(known) = known {
                let shape = self
                    .cfg
                    .evaluated_values
                    .shapes
                    .iter()
                    .find(|shape| shape.id == known.shape)
                    .ok_or_else(|| {
                        self.missing_context(
                            occurrence.range,
                            ReferenceAnalysisErrorReason::MissingResolvedBinding,
                        )
                    })?;
                if known.id != id || !scalar_type_equal(&shape.ty, &resolved.ty) {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
                known.shape.clone()
            } else {
                if matches!(expression, ScalarExpression::Assignment(_)) {
                    return Err(self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                    ));
                }
                let shape = intern_value_shape(
                    &mut self.cfg.evaluated_values.shapes,
                    &self.cfg.producer_owner,
                    &resolved.ty,
                    &self.structs,
                    &self.enums,
                    &self.generic_scope,
                    &self.generic_declarations,
                )
                .map_err(|_| {
                    self.missing_context(
                        occurrence.range,
                        ReferenceAnalysisErrorReason::MissingResolvedBinding,
                    )
                })?;
                self.cfg.evaluated_values.values.push(EvaluatedValueFact {
                    id: id.clone(),
                    shape: shape.clone(),
                    cases: Vec::new(),
                });
                shape
            };
            entries.push(assignment::EvaluatedOutputEntry { value: id, shape });
        }
        let sequence = assignment::EvaluatedOutputSequence { entries };
        self.validate_final_output_sequence(expression, &sequence, &outputs)?;
        Ok(sequence)
    }

    fn validate_final_output_sequence(
        &self,
        expression: &ScalarExpression,
        sequence: &assignment::EvaluatedOutputSequence,
        outputs: &ScalarOutputSequence,
    ) -> Result<(), ReferenceAnalysisError> {
        if sequence.entries.len() != outputs.outputs.len() {
            return Err(self.missing_context(
                expression_span(expression),
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            ));
        }
        for (output, (entry, resolved)) in sequence.entries.iter().zip(&outputs.outputs).enumerate()
        {
            if entry.value.resolved.output != output
                || sequence
                    .entries
                    .iter()
                    .filter(|other| other.value == entry.value)
                    .count()
                    != 1
            {
                return Err(self.missing_context(
                    expression_span(expression),
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
            self.validate_final_output_entry(expression, entry, &resolved.ty)?;
            let shape = self
                .cfg
                .evaluated_values
                .shapes
                .iter()
                .find(|shape| shape.id == entry.shape)
                .expect("validated final source shape");
            if !scalar_type_equal(&shape.ty, &resolved.ty) {
                return Err(self.missing_context(
                    expression_span(expression),
                    ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_final_output_entry(
        &self,
        expression: &ScalarExpression,
        entry: &assignment::EvaluatedOutputEntry,
        receiving_type: &ScalarType,
    ) -> Result<(), ReferenceAnalysisError> {
        let occurrence = SourceSpan::new(self.source.clone(), expression_span(expression));
        let invalid = || {
            self.missing_context(
                occurrence.range,
                ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            )
        };
        let projection = match expression {
            ScalarExpression::If { then_branch, .. } => then_branch.occurrence.output_projection.clone(),
            ScalarExpression::Call { occurrence, .. } => occurrence.output_projection.clone(),
            ScalarExpression::Assignment(assignment) => {
                assignment.occurrence.output_projection.clone()
            }
            ScalarExpression::Block(block) => block.occurrence.output_projection.clone(),
            ScalarExpression::Dereference { occurrence, .. }
            | ScalarExpression::IndexedRead { occurrence, .. } => {
                occurrence.output_projection.clone()
            }
            _ => Vec::new(),
        };
        let facts = self
            .cfg
            .evaluated_values
            .values
            .iter()
            .filter(|fact| {
                if matches!(expression, ScalarExpression::Assignment(_)) {
                    fact.id == entry.value
                } else {
                    fact.id.resolved == entry.value.resolved
                }
            })
            .collect::<Vec<_>>();
        let [fact] = facts.as_slice() else {
            return Err(invalid());
        };
        let shapes = self
            .cfg
            .evaluated_values
            .shapes
            .iter()
            .filter(|shape| shape.id == entry.shape)
            .collect::<Vec<_>>();
        let [shape] = shapes.as_slice() else {
            return Err(invalid());
        };
        let point = self
            .cfg
            .points
            .get(entry.value.point.0)
            .ok_or_else(invalid)?;
        if entry.value.resolved.producer != self.cfg.producer_owner
            || entry.value.resolved.occurrence != occurrence
            || entry.value.resolved.output_projection != projection
            || fact.id != entry.value
            || fact.shape != entry.shape
            || shape.id.producer != self.cfg.producer_owner
            || point.id != entry.value.point
            || point.source_span.source != occurrence.source
            || !scalar_type_equal(&shape.ty, receiving_type)
        {
            return Err(invalid());
        }
        match expression {
            ScalarExpression::Assignment(assignment) => {
                self.final_assignment_result_indices(assignment, entry)?;
            }
            ScalarExpression::Call { .. } => {
                if !matches!(&point.kind, CfgPointKind::Call { expression: actual, output, .. }
                    if actual == expression && *output == entry.value.resolved.output)
                    || point.source_span != occurrence
                {
                    return Err(invalid());
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
