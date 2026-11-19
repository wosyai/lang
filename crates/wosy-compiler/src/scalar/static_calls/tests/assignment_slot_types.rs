use super::*;
use crate::scalar::control::AssignmentReplacement;

#[test]
fn reference_queries_keep_frozen_assignment_types_after_legacy_table_removal() {
    let text = "%%start\n(u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };\nunit(bool) run = fn(flag) { u8 value = 0; bool valid = false; value; valid; value, valid = pair(flag); value; valid; value, valid = value, flag; value; valid; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let stages = Scope::new();
        let mut prepared = PreparedProgram::new(declarations, target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        drop(stages);
        let row = prepared.calls.function(1);
        assert_eq!(row.assignments.len(), 2);
        for group in row.assignments.values() {
            assert_eq!(
                group
                    .destinations()
                    .map(|slot| slot.replacement.clone())
                    .collect::<Vec<_>>(),
                vec![
                    AssignmentReplacement::Resolved(ScalarType::U8),
                    AssignmentReplacement::Resolved(ScalarType::Bool)
                ]
            );
        }
        let static_rows = row.clone();
        let before = prepared.program.clone();
        let stages = Scope::new();
        let first = prepared.analyze();
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 0,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 1
            }
        );
        drop(stages);
        let mut expected = before.clone();
        let errors = first.install(&mut expected);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(prepared.program, before);
        assert_eq!(prepared.program.resolved_assignment_outputs.len(), 4);
        prepared.program.resolved_assignment_outputs = ScalarAssignmentTyping::default();
        let stripped = prepared.program.clone();
        assert_eq!(prepared.calls.function(1), &static_rows);
        let stages = Scope::new();
        let copies = layout::observations::Scope::new();
        let second = prepared.analyze();
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 0,
                reference_origins: 1,
                output_contracts: 1,
                copy_specialization: 1
            }
        );
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(stages);
        let mut actual = stripped.clone();
        assert_eq!(second.install(&mut actual), errors);
        expected.resolved_assignment_outputs = ScalarAssignmentTyping::default();
        assert_eq!(actual, expected);
        assert_eq!(prepared.program, stripped);
        let finished = prepared.finish();
        assert_eq!(finished.program, expected);
        assert!(finished.diagnostics.is_empty());
    }
}
