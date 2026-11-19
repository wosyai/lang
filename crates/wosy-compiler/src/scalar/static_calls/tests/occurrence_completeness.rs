use super::*;
use control::{CheckedCallSignature, ModuleInitializationDetail, ModuleInitializationRegion};
use reference::{
    FunctionCallSites, OccurrenceMetadata, OccurrenceOrdinal, OccurrenceRole, OccurrenceStructure,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Eq, PartialEq)]
enum RequiredFact {
    ExpressionResult,
    PlaceType,
    CallSignature,
}

#[derive(Debug, Eq, PartialEq)]
struct MissingFact {
    source_span: SourceSpan,
    ordinal: OccurrenceOrdinal,
    fact: RequiredFact,
}

fn missing_facts(
    source: &SourceIdentity,
    occurrences: &[OccurrenceMetadata],
    structure: &OccurrenceStructure,
    calls: &HashSet<OccurrenceOrdinal>,
    signatures: &HashMap<OccurrenceOrdinal, CheckedCallSignature>,
) -> Vec<MissingFact> {
    let mut missing = Vec::new();
    for (index, metadata) in occurrences.iter().enumerate() {
        let ordinal = OccurrenceOrdinal(index);
        let mut record = |fact| {
            missing.push(MissingFact {
                source_span: SourceSpan::new(source.clone(), metadata.span),
                ordinal,
                fact,
            });
        };
        let is_call = calls.contains(&ordinal);
        let has_signature = is_call && signatures.contains_key(&ordinal);
        match metadata.role {
            OccurrenceRole::Expression => {
                if !structure.checked_types.expressions.contains_key(&ordinal)
                    && !has_signature
                    && !structure
                        .conditional_output_projections
                        .contains_key(&ordinal)
                    && !structure.folded_expressions.contains_key(&ordinal)
                    && !structure.synthetic_expressions.contains_key(&ordinal)
                {
                    record(RequiredFact::ExpressionResult);
                }
            }
            OccurrenceRole::Place => {
                if !structure.checked_types.places.contains_key(&ordinal) {
                    record(RequiredFact::PlaceType);
                }
            }
        }
        if is_call && !has_signature {
            record(RequiredFact::CallSignature);
        }
    }
    missing
}

fn callable_missing(source: &SourceIdentity, row: &FunctionCallSites) -> Vec<MissingFact> {
    let calls = row
        .call_occurrences
        .iter()
        .map(|call| call.ordinal)
        .chain(row.allocation_length_calls.iter().copied())
        .collect();
    missing_facts(
        source,
        &row.occurrences,
        &row.structure,
        &calls,
        &row.checked_call_signatures,
    )
}

fn initializer_missing(region: &ModuleInitializationRegion) -> Vec<MissingFact> {
    let ModuleInitializationDetail::Occurrences(row) = &region.detail else {
        panic!("prepared initializer topology");
    };
    missing_facts(
        &region.source,
        &row.occurrences,
        &row.structure,
        &row.calls.iter().copied().collect(),
        &row.checked_call_signatures,
    )
}

#[test]
fn inventories_prepared_initializer_and_callable_occurrences() {
    let parsed = parsed(
        "%%start
u64(u64) length = fn(n) { n };
(i8, bool)(i8) pair = fn(input) { if (true) { input, true } else { input, false } };
i8 module_signed = -128;
u8[length(2)] module_bytes;
i8[2] module_cells = [1, 2];
module_cells[0] = module_signed;
i8 initial, bool ready = pair(module_signed);
(i8, bool)(u64) run = fn(n) {
    u8[length(n)] local_bytes;
    local_bytes;
    i8[2] cells = [3, 4];
    cells[0] = -5;
    cells;
    pair(-6)
};
%%end",
    );
    let mut incomplete = Vec::new();
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        let standalone = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        for (route, init, rows) in [
            (
                "project",
                project.initialization_region(0).unwrap(),
                [0, 1, 7].map(|item| project.calls.function(0, item)),
            ),
            (
                "standalone",
                &standalone.calls.initialization,
                [0, 1, 7].map(|item| standalone.calls.function(item)),
            ),
        ] {
            let ModuleInitializationDetail::Occurrences(initializer) = &init.detail else {
                panic!("prepared initializer topology");
            };
            let run = rows[2];
            assert_eq!(initializer.structure.folded_expressions.len(), 1);
            assert!(initializer.structure.synthetic_expressions.is_empty());
            assert_eq!(initializer.calls.len(), 2);
            assert_eq!(run.structure.folded_expressions.len(), 2);
            assert_eq!(run.structure.synthetic_expressions.len(), 1);
            assert_eq!(run.allocation_length_calls.len(), 1);
            assert_eq!(run.call_occurrences.len(), 1);
            for occurrences in [&initializer.occurrences, &run.occurrences] {
                assert_eq!(
                    occurrences
                        .iter()
                        .filter(|row| row.role == OccurrenceRole::Place)
                        .count(),
                    2,
                );
            }
            let pair = rows[1];
            let pair_root = &pair.structure.blocks[pair.structure.root.0];
            let producer = pair_root.final_outputs().next().unwrap().expression;
            assert_eq!(
                pair.structure.conditional_output_projections[&producer].types,
                [ScalarType::I8, ScalarType::Bool],
            );
            for signature in [
                initializer
                    .checked_call_signatures
                    .get(&initializer.calls[1]),
                run.checked_call_signatures
                    .get(&run.call_occurrences[0].ordinal),
            ]
            .into_iter()
            .flatten()
            {
                assert!(signature
                    .outputs
                    .outputs
                    .iter()
                    .map(|output| &output.ty)
                    .eq([&ScalarType::I8, &ScalarType::Bool]));
            }
            let missing = initializer_missing(init);
            if !missing.is_empty() {
                incomplete.push((target, route, "initializer", missing));
            }
            for (function, row) in ["length", "pair", "run"].into_iter().zip(rows) {
                let missing = callable_missing(&init.source, row);
                if !missing.is_empty() {
                    incomplete.push((target, route, function, missing));
                }
            }
        }
    }
    assert!(incomplete.is_empty(), "{incomplete:#?}");
}

#[test]
fn inventory_attributes_a_removed_checked_expression_row() {
    let parsed = parsed("%%start\ni32(i32) echo = fn(input) { input };\n%%end");
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = prepare_extracted(&parsed, target);
        let row = prepared.calls.function(0, 0);
        let source = &prepared.project().modules[0].source;
        assert!(callable_missing(source, row).is_empty());

        let mut damaged = row.clone();
        let ordinal = OccurrenceOrdinal(0);
        assert_eq!(
            damaged.structure.checked_types.expressions.remove(&ordinal),
            Some(ScalarType::I32),
        );
        assert_eq!(
            callable_missing(source, &damaged),
            vec![MissingFact {
                source_span: SourceSpan::new(parsed.source.clone(), ByteSpan::new(36, 41)),
                ordinal,
                fact: RequiredFact::ExpressionResult,
            }],
        );
        assert!(callable_missing(source, row).is_empty());
    }
}
