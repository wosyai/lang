use super::*;
use control::ModuleInitializationDetail;
use reference::{OccurrenceOrdinal, OccurrenceRole};

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
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        let standalone = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        for (init, pair, run) in [
            (
                project.initialization_region(0).unwrap(),
                project.calls.function(0, 1),
                project.calls.function(0, 7),
            ),
            (
                &standalone.calls.initialization,
                standalone.calls.function(1),
                standalone.calls.function(7),
            ),
        ] {
            let ModuleInitializationDetail::Occurrences(initializer) = &init.detail else {
                panic!("prepared initializer topology");
            };
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
            let pair_root = &pair.structure.blocks[pair.structure.root.0];
            let producer = pair_root.final_outputs().next().unwrap().expression;
            assert_eq!(
                pair.structure.conditional_output_projections[&producer].types,
                [ScalarType::I8, ScalarType::Bool],
            );
            for signature in [
                &initializer.checked_call_signatures[&initializer.calls[1]],
                &run.checked_call_signatures[&run.call_occurrences[0].ordinal],
            ] {
                assert!(signature
                    .outputs
                    .outputs
                    .iter()
                    .map(|output| &output.ty)
                    .eq([&ScalarType::I8, &ScalarType::Bool]));
            }
        }
    }
}

#[test]
fn production_obligation_attributes_a_removed_checked_expression_row() {
    let parsed = parsed("%%start\ni32(i32) echo = fn(input) { input };\n%%end");
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let prepared = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        let mut damaged = prepared.calls.clone();
        let ordinal = OccurrenceOrdinal(0);
        assert_eq!(
            damaged
                .function_mut(0)
                .structure
                .checked_types
                .expressions
                .remove(&ordinal),
            Some(ScalarType::I32),
        );
        let error = occurrence_obligations::validate_module(&damaged).unwrap_err();
        let span = SourceSpan::new(parsed.source.clone(), ByteSpan::new(36, 41));
        assert_eq!(
            error,
            ReferenceAnalysisError::MissingContext {
                source_span: span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingCheckedExpressionResult { ordinal: 0 },
            },
        );
        let diagnostic = error.diagnostic();
        assert_eq!(diagnostic.code, "B0003");
        assert_eq!(
            diagnostic.message,
            "missing checked expression result (occurrence 0)"
        );
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].span, span);
        assert_eq!(diagnostic.labels[0].message, diagnostic.message);
    }
}
