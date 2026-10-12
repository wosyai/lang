use super::*;

pub(super) fn pair(
    point: &ReferenceFlowPoint,
    state: &ReferenceEdgeState,
    selected: Vec<selected_source::SelectedSourceState>,
    tables: &EvaluatedValueTables,
    last_use: &[usize],
    isolated: bool,
    #[cfg(test)] observations: &mut ReferenceTransferTestObservations,
) -> Result<Vec<ReferenceWorkItem>, ReferenceAnalysisError> {
    let read = matches!(point.kind, CfgPointKind::Read { .. } | CfgPointKind::ReadPlace { .. });
    if (!read && selected.len() != 1) || (read && !isolated) {
        return Err(ReferenceAnalysisError::MissingContext {
            source_span: point.source_span.clone(),
            reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
        });
    }
    let mut outputs = Vec::new();
    for selected in selected {
        let mut state = state.clone();
        assignment_saved_flow::attach_selected(&mut state, point, &selected, tables)?;
        #[cfg(test)]
        observations.points.push(ReferenceTransferPointSnapshot {
            point: point.id,
            phase: ReferenceTransferPhase::BeforeInputCleanup,
            state: state.clone(),
        });
        for (index, value) in state.point_values.iter_mut().enumerate().take(point.id.0 + 1) {
            if last_use[index] <= point.id.0 {
                *value = None;
            }
        }
        #[cfg(test)]
        observations.points.push(ReferenceTransferPointSnapshot {
            point: point.id,
            phase: ReferenceTransferPhase::AfterInputCleanup,
            state: state.clone(),
        });
        let output = ReferenceWorkItem { state, selected };
        if !outputs.contains(&output) {
            outputs.push(output);
        }
    }
    Ok(outputs)
}
