use std::collections::HashMap;

use super::control::{CheckedCallSignature, ModuleInitializationDetail};
use super::reference::{
    CallOccurrence, FunctionCallSites, ModuleCallSites, OccurrenceMetadata, OccurrenceOrdinal,
    OccurrenceRole, OccurrenceStructure,
};
use super::{ReferenceAnalysisError, ReferenceAnalysisErrorReason as Reason};
use wosy_syntax::{SourceIdentity, SourceSpan};

pub(super) fn validate_module(calls: &ModuleCallSites) -> Result<(), ReferenceAnalysisError> {
    let source = &calls.initialization.source;
    for function in calls.functions_in_item_order() {
        validate_callable(source, function)?;
    }

    let ModuleInitializationDetail::Occurrences(initializer) = &calls.initialization.detail else {
        unreachable!("completed module call lookup has initializer topology");
    };
    validate_region(
        source,
        &initializer.occurrences,
        &initializer.structure,
        &[],
        &initializer.calls,
        &initializer.checked_call_signatures,
        None,
    )
}

fn validate_callable(
    source: &SourceIdentity,
    row: &FunctionCallSites,
) -> Result<(), ReferenceAnalysisError> {
    validate_region(
        source,
        &row.occurrences,
        &row.structure,
        &row.call_occurrences,
        &row.allocation_length_calls,
        &row.checked_call_signatures,
        Some(row),
    )?;
    for arm in &row.arms {
        validate_callable(source, arm)?;
    }
    Ok(())
}

fn validate_region(
    source: &SourceIdentity,
    occurrences: &[OccurrenceMetadata],
    structure: &OccurrenceStructure,
    site_calls: &[CallOccurrence],
    executable_calls: &[OccurrenceOrdinal],
    signatures: &HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    callable: Option<&FunctionCallSites>,
) -> Result<(), ReferenceAnalysisError> {
    let mut site_calls = site_calls.iter().map(|call| call.ordinal).peekable();
    let mut executable_calls = executable_calls.iter().copied().peekable();
    for (index, metadata) in occurrences.iter().enumerate() {
        let ordinal = OccurrenceOrdinal(index);
        let is_call = site_calls.next_if_eq(&ordinal).is_some()
            | executable_calls.next_if_eq(&ordinal).is_some();
        let has_signature = is_call && signatures.contains_key(&ordinal);
        let reason = match metadata.role {
            OccurrenceRole::Expression
                if metadata.requires_plain_name
                    && callable.is_some_and(|row| !row.has_prepared_plain_name(ordinal)) =>
            {
                Reason::MissingResolvedBinding
            }
            OccurrenceRole::Expression
                if !structure.checked_types.expressions.contains_key(&ordinal)
                    && !has_signature
                    && !structure
                        .conditional_output_projections
                        .contains_key(&ordinal)
                    && !structure.folded_expressions.contains_key(&ordinal)
                    && !structure.synthetic_expressions.contains_key(&ordinal) =>
            {
                Reason::MissingCheckedExpressionResult { ordinal: index }
            }
            OccurrenceRole::Place
                if metadata.requires_direct_destination
                    && callable
                        .is_some_and(|row| !row.has_prepared_direct_destination(ordinal)) =>
            {
                Reason::MissingResolvedBinding
            }
            OccurrenceRole::Place
                if !structure.checked_types.places.contains_key(&ordinal)
                    && !structure.resolved_namespace_places.contains_key(&ordinal) =>
            {
                Reason::MissingCheckedPlaceType { ordinal: index }
            }
            _ if is_call && !has_signature => {
                Reason::MissingCheckedCallSignature { ordinal: index }
            }
            _ => continue,
        };
        return Err(ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(source.clone(), metadata.span),
            reason,
        });
    }
    Ok(())
}
