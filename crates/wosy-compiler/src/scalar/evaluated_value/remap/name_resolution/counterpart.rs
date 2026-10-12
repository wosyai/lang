use super::*;

fn declaration(remap: &ValueSchemaRemap, source: &SourceNameDeclarationRef, destination: &SourceNameDeclarationRef) -> Result<bool, ValueSchemaRemapError> {
    if remap.producer(&source.owner)? != destination.owner { return Ok(false); }
    Ok(match (&source.site, &destination.site) {
        (SourceNameDeclarationSite::Parameter { ordinal }, SourceNameDeclarationSite::Parameter { ordinal: actual }) => ordinal == actual,
        (SourceNameDeclarationSite::Receiver { occurrence, ordinal }, SourceNameDeclarationSite::Receiver { occurrence: actual, ordinal: actual_ordinal }) => ordinal == actual_ordinal
            && occurrence.source_span.range == actual.source_span.range && occurrence.output_projection == actual.output_projection,
        (SourceNameDeclarationSite::Assignment { occurrence, ordinal }, SourceNameDeclarationSite::Assignment { occurrence: actual, ordinal: actual_ordinal }) => ordinal == actual_ordinal
            && occurrence.source_span.range == actual.source_span.range && occurrence.output_projection == actual.output_projection,
        (SourceNameDeclarationSite::Capture { event, ordinal }, SourceNameDeclarationSite::Capture { event: actual, ordinal: actual_ordinal }) => ordinal == actual_ordinal && remap.source_event(event)? == *actual,
        _ => false,
    })
}

pub(in crate::scalar::evaluated_value::remap) fn validate(remap: &ValueSchemaRemap, source: &ValueTableSource, destination: &ValueTableSource) -> Result<(), ValueSchemaRemapError> {
    original_inventory::source(source)?;
    original_inventory::source(destination)?;
    null_constructor::counterpart(remap, source, destination)?;
    checked_parameter::counterpart(remap, source.resource_construction.original(), destination.resource_construction.original())?;
    let fail = || ValueSchemaRemapError::MissingProducer(source.producer.clone());
    if remap.producer(&source.producer)? != destination.producer || source.tables.source_name_reads.len() != destination.tables.source_name_reads.len() { return Err(fail()); }
    for original in &source.tables.source_name_reads {
        let transported = remap.name_read(original)?;
        let rows = destination.tables.source_name_reads.iter().filter(|row| row.event == transported.event && row.value == transported.value).collect::<Vec<_>>();
        let [actual] = rows.as_slice() else { return Err(fail()); };
        if actual.binding != transported.binding || actual.expression != original.expression || actual.occurrence.range != original.occurrence.range
            || actual.role != original.role || !declaration(remap, &original.declaration, &actual.declaration)?
            || !declaration(remap, &original.original_declaration, &actual.original_declaration)? { return Err(fail()); }
        let source_values = source.tables.values.iter().filter(|row| row.id == original.value).collect::<Vec<_>>();
        let destination_values = destination.tables.values.iter().filter(|row| row.id == actual.value).collect::<Vec<_>>();
        let ([source_value], [destination_value]) = (source_values.as_slice(), destination_values.as_slice()) else { return Err(fail()); };
        if remap.boundary_shape(&source_value.shape)? != destination_value.shape { return Err(fail()); }
    }
    Ok(())
}
