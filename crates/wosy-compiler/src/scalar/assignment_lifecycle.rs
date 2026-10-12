use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SupportedInstanceVariable(pub usize);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrameVariableRef {
    pub owner: ProducerCfgOwnerId,
    pub variable: SupportedInstanceVariable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AssignmentApplicationRef {
    pub frame: FrameVariableRef,
    pub operation: AssignmentOperationId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrozenSelectionOccurrence {
    pub application: AssignmentApplicationRef,
    pub destination: AssignmentDestinationId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum FrozenConsumerRole {
    Snapshot(CfgPointId),
    Commit(CfgPointId),
    Result(EvaluatedValueId),
    Dispose(CfgPointId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrozenGuardUse {
    pub selection: FrozenSelectionOccurrence,
    pub consumer: FrozenConsumerRole,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrozenApplicationState {
    pub application: AssignmentApplicationRef,
    pub remaining: Vec<FrozenGuardUse>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetainedIndexEndpointId {
    pub producer: ProducerCfgOwnerId,
    pub slot: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RetainedIndexSourceRole {
    LivePlace(ReferencePlaceId),
    Evaluated {
        producer: ProducerCfgOwnerId,
        point: CfgPointId,
        occurrence: SourceSpan,
        function_span: ByteSpan,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetainedIndexSourceFact {
    pub introduction: EvaluatedValueId,
    pub role: RetainedIndexSourceRole,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FrozenDestinationProtocol {
    pub(super) destination: AssignmentDestinationId,
    pub(super) point: CfgPointId,
    pub(super) consumers: Vec<FrozenConsumerRole>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FrozenAssignmentProtocol {
    pub(super) owner: ProducerCfgOwnerId,
    pub(super) destinations: Vec<FrozenDestinationProtocol>,
}

impl FrozenAssignmentProtocol {
    pub(super) fn new(cfg: &ScalarReferenceCfg) -> Result<Self, ReferenceAnalysisError> {
        let invalid = |point: CfgPointId| ReferenceAnalysisError::MissingContext {
            source_span: cfg.initial_points[point.0].source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        };
        let mut destinations = Vec::new();
        for destination in &cfg.evaluated_values.destinations {
            if destination.id.operation.producer != cfg.producer_owner
                || destinations.iter().any(|row: &FrozenDestinationProtocol| {
                    row.destination == destination.id || row.point == destination.point
                })
            {
                return Err(invalid(destination.point));
            }
            let mut evaluations = cfg
                .evaluated_values
                .assignment_evaluations
                .iter()
                .filter(|evaluation| evaluation.operation == destination.id.operation);
            let evaluation = evaluations
                .next()
                .ok_or_else(|| invalid(destination.point))?;
            if evaluations.next().is_some()
                || evaluation.destinations.get(destination.id.target) != Some(destination)
            {
                return Err(invalid(destination.point));
            }
            let snapshot = evaluation
                .previous
                .get(destination.id.target)
                .ok_or_else(|| invalid(destination.point))?;
            let commit = evaluation
                .writes
                .get(destination.id.target)
                .ok_or_else(|| invalid(destination.point))?;
            if snapshot.destination != destination.id
                || commit.destination != destination.id
                || commit.previous != snapshot.point
                || !cfg.evaluated_values.snapshots.contains(snapshot)
                || !cfg.evaluated_values.commits.contains(commit)
            {
                return Err(invalid(destination.point));
            }
            let mut consumers = Vec::new();
            match &snapshot.state {
                SnapshotState::NoPriorDeclaration => {
                    if !matches!(
                        destination.storage,
                        EvaluatedDestinationStorage::Inferred(_)
                    ) {
                        return Err(invalid(destination.point));
                    }
                }
                SnapshotState::Saved(_)
                | SnapshotState::ElidedDiscard { .. }
                | SnapshotState::Unavailable { .. } => {
                    if !matches!(
                        &cfg.initial_points[snapshot.point.0].kind,
                        CfgPointKind::AssignmentSnapshot { destination_point, .. }
                            if *destination_point == destination.point
                    ) {
                        return Err(invalid(snapshot.point));
                    }
                    consumers.push(FrozenConsumerRole::Snapshot(snapshot.point));
                }
            }
            if !matches!(
                cfg.initial_points[commit.point.0].kind,
                CfgPointKind::Assign { .. } | CfgPointKind::AssignThrough { .. }
            ) {
                return Err(invalid(commit.point));
            }
            consumers.push(FrozenConsumerRole::Commit(commit.point));
            for result in evaluation
                .results
                .iter()
                .filter(|result| result.snapshot == snapshot.point)
            {
                if result.operation != evaluation.operation
                    || result.output != destination.id.target
                    || !cfg.evaluated_values.results.contains(result)
                    || !matches!(
                        &cfg.initial_points[result.value.point.0].kind,
                        CfgPointKind::AssignmentResult { snapshot_point, output }
                            if *snapshot_point == snapshot.point && *output == result.output
                    )
                {
                    return Err(invalid(result.value.point));
                }
                consumers.push(FrozenConsumerRole::Result(result.value.clone()));
            }
            for point in &cfg.initial_points {
                if matches!(
                    &point.kind,
                    CfgPointKind::AssignmentDispose { snapshot_point }
                        if *snapshot_point == snapshot.point
                ) {
                    consumers.push(FrozenConsumerRole::Dispose(point.id));
                }
            }
            destinations.push(FrozenDestinationProtocol {
                destination: destination.id.clone(),
                point: destination.point,
                consumers,
            });
        }
        Ok(Self {
            owner: cfg.producer_owner.clone(),
            destinations,
        })
    }

    pub(super) fn selection(
        &self,
        frame: &FrameVariableRef,
        point: CfgPointId,
    ) -> Result<FrozenSelectionOccurrence, ReferenceAnalysisErrorReason> {
        if frame.owner != self.owner {
            return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
        }
        let destination = self
            .destinations
            .iter()
            .find(|row| row.point == point)
            .ok_or(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)?;
        Ok(FrozenSelectionOccurrence {
            application: AssignmentApplicationRef {
                frame: frame.clone(),
                operation: destination.destination.operation.clone(),
            },
            destination: destination.destination.clone(),
        })
    }

    pub(super) fn uses(
        &self,
        selection: &FrozenSelectionOccurrence,
    ) -> Result<Vec<FrozenGuardUse>, ReferenceAnalysisErrorReason> {
        if selection.application.frame.owner != self.owner
            || selection.application.operation != selection.destination.operation
        {
            return Err(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence);
        }
        let destination = self
            .destinations
            .iter()
            .find(|row| row.destination == selection.destination)
            .ok_or(ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence)?;
        Ok(destination
            .consumers
            .iter()
            .map(|consumer| FrozenGuardUse {
                selection: selection.clone(),
                consumer: consumer.clone(),
            })
            .collect())
    }
}
