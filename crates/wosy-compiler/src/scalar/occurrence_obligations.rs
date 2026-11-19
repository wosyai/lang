use std::collections::HashMap;

use super::control::{CheckedCallSignature, InitializationIdentities, ModuleInitializationDetail};
use super::reference::{
    CallOccurrence, FunctionCallSites, InitializerMemberRequirement, ModuleCallSites,
    OccurrenceMetadata, OccurrenceOrdinal, OccurrenceRole, OccurrenceStructure,
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
        IdentityRows::Initializer(&initializer.identities),
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
        IdentityRows::Callable(row),
    )?;
    for arm in &row.arms {
        validate_callable(source, arm)?;
    }
    Ok(())
}

enum IdentityRows<'a> {
    Callable(&'a FunctionCallSites),
    Initializer(&'a InitializationIdentities),
}

impl IdentityRows<'_> {
    fn has_name(&self, ordinal: OccurrenceOrdinal) -> bool {
        match self {
            Self::Callable(row) => row.has_prepared_plain_name(ordinal),
            Self::Initializer(rows) => rows.names.contains_key(&ordinal),
        }
    }

    fn has_destination(&self, ordinal: OccurrenceOrdinal) -> bool {
        match self {
            Self::Callable(row) => row.has_prepared_direct_destination(ordinal),
            Self::Initializer(rows) => rows.destinations.contains_key(&ordinal),
        }
    }

    fn has_root(&self, ordinal: OccurrenceOrdinal) -> bool {
        match self {
            Self::Callable(row) => row.has_prepared_named_place_root(ordinal),
            Self::Initializer(rows) => rows.roots.contains_key(&ordinal),
        }
    }
}

fn validate_region(
    source: &SourceIdentity,
    occurrences: &[OccurrenceMetadata],
    structure: &OccurrenceStructure,
    site_calls: &[CallOccurrence],
    executable_calls: &[OccurrenceOrdinal],
    signatures: &HashMap<OccurrenceOrdinal, CheckedCallSignature>,
    identities: IdentityRows<'_>,
) -> Result<(), ReferenceAnalysisError> {
    let mut site_calls = site_calls.iter().map(|call| call.ordinal).peekable();
    let mut executable_calls = executable_calls.iter().copied().peekable();
    for (index, metadata) in occurrences.iter().enumerate() {
        let ordinal = OccurrenceOrdinal(index);
        let is_call = site_calls.next_if_eq(&ordinal).is_some()
            | executable_calls.next_if_eq(&ordinal).is_some();
        let has_signature = is_call && signatures.contains_key(&ordinal);
        if let InitializerMemberRequirement::Token(span) = metadata.initializer_member {
            if matches!(&identities, IdentityRows::Initializer(rows) if !rows.members.contains_key(&ordinal))
            {
                return Err(ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(source.clone(), span),
                    reason: Reason::MissingResolvedBinding,
                });
            }
        }
        let reason = match metadata.role {
            OccurrenceRole::Expression
                if metadata.requires_plain_name && !identities.has_name(ordinal) =>
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
                if metadata.requires_direct_destination && !identities.has_destination(ordinal) =>
            {
                Reason::MissingResolvedBinding
            }
            OccurrenceRole::Place
                if metadata.requires_named_place_root
                    && !structure.resolved_namespace_places.contains_key(&ordinal)
                    && !identities.has_root(ordinal) =>
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
