use super::*;

pub(super) fn checked_source(
    tables: &mut EvaluatedValueTables,
    row: &SourceConfigurationFact,
    owner: &ProducerCfgOwnerId,
    point: &ReferenceFlowPoint,
    points: &[ReferenceFlowPoint],
    checked: &Option<BTreeSet<ReferenceOriginId>>,
    origins: &[ReferenceOrigin],
    value: &EvaluatedValueId,
    shape: &ValueShapeId,
) -> Result<Option<SourceValueViewRef>, ReferenceAnalysisError> {
    let Some(checked) = checked else {
        return Ok(None);
    };
    if checked.is_empty() {
        return Ok(None);
    }
    let mut referents = Vec::new();
    let mut dependencies = Vec::new();
    let mut state = CheckedValueState::Live;
    for origin_id in checked {
        let origin = origins
            .get(origin_id.0)
            .ok_or_else(|| invalid(&point.source_span))?;
        let loan = match origin {
            ReferenceOrigin::Fresh { loan, .. } | ReferenceOrigin::BorrowedFrom { loan, .. } => {
                loan
            }
            ReferenceOrigin::Null { span } => {
                if checked.len() != 1 {
                    return Ok(None);
                }
                state = CheckedValueState::Null {
                    source: SourceSpan::new(point.source_span.source.clone(), *span),
                };
                continue;
            }
            ReferenceOrigin::Invalid { conflict_span, .. } => {
                if checked.len() != 1 {
                    return Ok(None);
                }
                let effects = points
                    .iter()
                    .filter(|effect| {
                        effect.source_span.source == point.source_span.source
                            && effect.source_span.range == *conflict_span
                    })
                    .collect::<Vec<_>>();
                let [effect] = effects.as_slice() else {
                    return Ok(None);
                };
                state = CheckedValueState::Expired {
                    origin: *origin_id,
                    effect_point: effect.id,
                };
                continue;
            }
        };
        let slot = match selected_slot(
            tables,
            row,
            &SourceCoordinateRole::Binding(loan.origin_place.binding.clone()),
        ) {
            Ok(slot) => slot.clone(),
            Err(()) => return Ok(None),
        };
        let referent_view = match projected_view(tables, row, &loan.origin_place) {
            Ok(view) => view,
            Err(())
                if matches!(
                    tables.source_views[slot.view.index].payload,
                    SourcePayloadView::Backing { .. }
                ) =>
            {
                slot.view.clone()
            }
            Err(()) => return Ok(None),
        };
        let instance = match &tables.source_views[slot.view.index].payload {
            SourcePayloadView::Backing { instance, .. } => instance.clone(),
            _ => match current_cell(tables, row, &slot.coordinate) {
                Ok(cell) => cell,
                Err(()) => return Ok(None),
            },
        };
        let mut projection = Vec::new();
        for component in &loan.origin_place.projections {
            match component {
                ReferencePlaceProjection::Field(ScalarFieldReference::Resolved(field)) => {
                    projection.push(ValueProjection::Field(field.clone()))
                }
                ReferencePlaceProjection::Index(expression) => {
                    let occurrence = SourceSpan::new(
                        loan.origin_place.source.clone(),
                        expression_span(expression),
                    );
                    let selectors = tables
                        .values
                        .iter()
                        .filter(|value| {
                            value.id.resolved.occurrence == occurrence
                                && value.id.resolved.output == 0
                        })
                        .collect::<Vec<_>>();
                    let [selector] = selectors.as_slice() else {
                        return Ok(None);
                    };
                    projection.push(ValueProjection::RuntimeIndex {
                        selector: selector.id.clone(),
                    });
                }
                _ => return Ok(None),
            }
        }
        for dependency in &tables.source_views[referent_view.index].dependencies {
            if !dependencies.contains(dependency) {
                dependencies.push(dependency.clone());
            }
        }
        referents.push(SourceCheckedReferentView {
            instance,
            coordinate: slot.coordinate,
            projection,
            grant: SourceSpan::new(loan.creation_source.clone(), loan.creation_span),
            mode: loan.mode,
            loan: SourceCheckedGrantState::Pending {
                event: SourceEventId {
                    owner: owner.clone(),
                    point: point.id,
                },
            },
        });
    }
    if value.resolved.producer != *owner {
        return Err(invalid(&point.source_span));
    }
    Ok(Some(intern_view(
        tables,
        owner,
        shape.clone(),
        SourcePayloadView::Checked { state, referents },
        dependencies,
    )))
}
