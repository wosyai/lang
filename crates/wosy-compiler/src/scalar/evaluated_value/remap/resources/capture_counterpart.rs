use super::*;

pub(super) fn validate(remap: &ValueSchemaRemap, source: &BoundaryCaptureResourceTuple, transported: &BoundaryCaptureResourceTuple, actual: &BoundaryCaptureResourceTuple, correspondence: &BoundaryArenaCorrespondence, source_originals: &[BoundaryResourceConstructionEvidence], actual_originals: &[BoundaryResourceConstructionEvidence], types: &input_counterpart::BoundaryInputTypeEvidence, associations: &[BoundaryResourceAssociation]) -> Result<(), ValueSchemaRemapError> {
    let invalid = || ValueSchemaRemapError::MissingPoint { producer: source.production.event.owner.clone(), point: source.production.event.point };
    if allocation::mapped_tuple(remap, source, correspondence, associations, true)? != *transported { return Err(invalid()); }
    let input_counterpart::BoundaryInputTypeEvidence::Independent(checked) = types else { return Err(invalid()); };
    let original = &source.production;
    let mapped = &transported.production;
    let target = &actual.production;
    if original.output.resolved.producer != original.event.owner || original.output.point != original.event.point
        || target.output.resolved.producer != target.event.owner || target.output.point != target.event.point
        || mapped.output.resolved.producer != mapped.event.owner || mapped.output.point != mapped.event.point
        || mapped.event != target.event || mapped.child != target.child || mapped.signature != original.signature || mapped.inputs.len() != target.inputs.len()
        || mapped.output.resolved.producer != target.output.resolved.producer || mapped.output.point != target.output.point
        || mapped.output.resolved.output != original.output.resolved.output || mapped.output.resolved.output != target.output.resolved.output
        || mapped.output.resolved.output_projection != original.output.resolved.output_projection || mapped.output.resolved.output_projection != target.output.resolved.output_projection
        || mapped.output.resolved.occurrence != original.output.resolved.occurrence
        || original.output.resolved.occurrence.source != *copied::lexical(&original.event.owner)
        || target.output.resolved.occurrence.source != *copied::lexical(&target.event.owner)
        || original.output.resolved.occurrence.range != target.output.resolved.occurrence.range {
        return Err(invalid());
    }
    let before = checked.capture_original(source_originals, &source.production, input_counterpart::OriginalInputEndpoint::Source)?;
    let after = match checked.capture_original(actual_originals, &actual.production, input_counterpart::OriginalInputEndpoint::Actual) {
        Err(ValueSchemaRemapError::MissingPoint { .. }) => return Err(invalid()),
        result => result?,
    };
    checked.capture_input_pair(remap, &source.production, &actual.production)?;
    for ((original, mapped), actual) in original.inputs.iter().zip(&mapped.inputs).zip(&target.inputs) {
        if correspondence.formal(&original.formal)? != actual.formal || mapped.formal != actual.formal
            || correspondence.slot(&original.coordinate)? != actual.coordinate || mapped.coordinate != actual.coordinate
            || remap.boundary_shape(&original.shape)? != actual.shape || mapped.shape != actual.shape
            || mapped.policy != original.policy || actual.policy != original.policy
            || remap.binding(&source.production.event.owner, &original.capture.binding)? != actual.capture.binding
            || mapped.capture.binding != actual.capture.binding || mapped.capture.name != original.capture.name
            || mapped.capture.ty != original.capture.ty || mapped.capture.place_type != original.capture.place_type
            || mapped.capture.use_span != original.capture.use_span || mapped.capture.place != original.capture.place { return Err(invalid()); }
    }
    let source_paths = before.capture_input_path_facts.iter().filter(|path| path.owner == original.event.owner && path.capture == original.event.point).cloned().collect::<Vec<_>>();
    let actual_paths = after.capture_input_path_facts.iter().filter(|path| path.owner == target.event.owner && path.capture == target.event.point).cloned().collect::<Vec<_>>();
    if source.paths != source_paths || actual.paths != actual_paths || transported.paths != actual.paths {
        let holder = source.paths.iter().flat_map(|path| &path.inputs).find_map(|input| match &input.origin {
            CaptureInputOriginFact::Value => None,
            CaptureInputOriginFact::Callable { holders } => holders.first(),
        });
        return Err(match holder { Some(holder) => ValueSchemaRemapError::MissingHolder(holder.clone()), None => invalid() });
    }
    Ok(())
}

pub(super) fn validate_owned(remap: &ValueSchemaRemap, source: &BoundaryCaptureResourceTuple, transported: &BoundaryCaptureResourceTuple, actual: &BoundaryCaptureResourceTuple, correspondence: &BoundaryArenaCorrespondence, resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    let BoundaryResourceProvenance::Independent { originals } = &resources.provenance else { return Err(ValueSchemaRemapError::MissingPoint { producer: source.production.event.owner.clone(), point: source.production.event.point }); };
    validate(remap, source, transported, actual, correspondence, &resources.source_originals, originals, &resources.input_types, &resources.associations)
}
