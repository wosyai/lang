use super::*;

fn missing(source: &ValueTableSource) -> ValueSchemaRemapError {
    ValueSchemaRemapError::MissingProducer(source.producer.clone())
}

fn root(context: &SourceNameResolutionContext) -> SourceNameResolutionContextRoot {
    SourceNameResolutionContextRoot { owner: context.owner.clone(), root: context.root.clone(), globals: context.globals.clone(), parents: context.parents.clone() }
}

pub(in crate::scalar::evaluated_value::remap) fn source(source: &ValueTableSource) -> Result<(), ValueSchemaRemapError> {
    let original = source.resource_construction.original();
    source_boundary::module_resolver_input::validate_endpoint(original)?;
    if original.producer_owner != source.producer || original.initial_points != source.initial_points
        || source.resource_construction.structs() != source.policy_context.structs
        || original.evaluated_values.source_name_reads != source.tables.source_name_reads {
        return Err(missing(source));
    }
    reference::name_resolution::validate(original, &source.policy_context.structs).map_err(|_| missing(source))?;
    let mut candidate = original.clone();
    candidate.evaluated_values = source.tables.clone();
    reference::name_resolution::validate(&candidate, &source.policy_context.structs).map_err(|_| missing(source))?;
    Ok(())
}

pub(in crate::scalar::evaluated_value::remap) fn forest(sources: &[ValueTableSource]) -> Result<(), ValueSchemaRemapError> {
    for row in sources {
        source(row)?;
        let NameResolutionContext::Complete(context) = &row.resource_construction.original().name_resolution_context else { return Err(missing(row)); };
        for ancestor in &context.ancestors {
            let endpoints = sources.iter().filter(|endpoint| endpoint.producer == ancestor.owner).collect::<Vec<_>>();
            match endpoints.as_slice() {
                [] => {},
                [endpoint] => {
                    let NameResolutionContext::Complete(original) = &endpoint.resource_construction.original().name_resolution_context else { return Err(missing(endpoint)); };
                    if root(original) != *ancestor { return Err(missing(row)); }
                },
                _ => return Err(missing(row)),
            }
        }
        if let SourceNameResolutionRoot::Literal { parent, occurrence, signature, parameters, parameter_spans, captures, body } = &context.root {
            if context.parents != vec![parent.clone()] { return Err(missing(row)); }
            let endpoints = sources.iter().filter(|endpoint| endpoint.producer == parent.owner).collect::<Vec<_>>();
            match endpoints.as_slice() {
                [] => {},
                [endpoint] => {
                    let points = endpoint.initial_points.iter().filter(|point| point.id == parent.point).collect::<Vec<_>>();
                    let [point] = points.as_slice() else { return Err(ValueSchemaRemapError::MissingPoint { producer: parent.owner.clone(), point: parent.point }); };
                    let CfgPointKind::Capture { literal, environment, signature: actual_signature, parameters: actual_parameters, parameter_spans: actual_spans, captures: actual_captures, body: actual_body } = &point.kind else { return Err(missing(row)); };
                    if point.source_span != *occurrence || *literal != occurrence.range || environment.literal != *occurrence
                        || signature != actual_signature || parameters != actual_parameters || parameter_spans != actual_spans
                        || captures != actual_captures || body != actual_body
                        || endpoint.children.iter().filter(|child| child.event == *parent && child.child == row.producer).count() != 1 { return Err(missing(row)); }
                },
                _ => return Err(missing(row)),
            }
        }
    }
    Ok(())
}

pub(in crate::scalar::evaluated_value::remap) fn owned(sources: &[ValueTableSource], resources: &CheckedBoundaryResources) -> Result<(), ValueSchemaRemapError> {
    forest(sources)?;
    for row in sources {
        let snapshots = resources.source_originals().iter().filter(|snapshot| snapshot.original().producer_owner == row.producer).collect::<Vec<_>>();
        let [snapshot] = snapshots.as_slice() else { return Err(missing(row)); };
        if *snapshot != &row.resource_construction { return Err(missing(row)); }
    }
    Ok(())
}
