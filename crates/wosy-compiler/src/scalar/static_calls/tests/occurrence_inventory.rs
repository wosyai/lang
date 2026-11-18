use super::*;
use reference::{CallOccurrence, OccurrenceOrdinal, OccurrenceRole};

#[test]
fn preparation_publishes_stable_callable_owned_expression_place_and_call_ordinals() {
    let text = "%%start\nu8 padding = 0;\nbool(bool) echo = fn(flag) { flag };\n(u8, bool)(bool) pair = fn(flag) { u8 value = 1; value, flag };\nu64(bool) index = fn(flag) { flag; 0 };\nprobe = overload { unit(bool) => fn(flag) { u8 a, bool valid = pair(echo(flag)); a; valid; u8[2] cells = [1, 2]; if (flag) { echo(flag); } else { echo(flag); }; cells[index(flag)] = a; cells; }; unit(u8) => fn(byte) { byte; }; };\n%%end";
    let mut expected_inventory = None;
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let scope = Scope::new();
        let copies = layout::observations::Scope::new();
        let prepared = PreparedProgram::new(declarations, target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            scope.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(scope);
        let calls = prepared.calls.as_ref().unwrap();
        let owner = calls.function(4);
        assert!(owner.occurrences.is_empty());
        assert_eq!(owner.arms.len(), 2);
        let row = &owner.arms[0];
        assert_eq!(row.occurrences.len(), 20);
        assert_eq!(
            row.call_occurrences,
            vec![
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(0),
                    site: 0
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(1),
                    site: 1
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(10),
                    site: 2
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(12),
                    site: 3
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(17),
                    site: 4
                },
            ]
        );
        assert_eq!(row.sites.len(), 5);
        for (site, item) in row.sites.iter().zip([2, 1, 1, 1, 3]) {
            let ScalarItem::Function(callee) = &prepared.program.items[item] else {
                panic!("callee item ordinal")
            };
            assert_eq!(
                site.target,
                ResolvedCallTarget::ModuleCallable {
                    source: prepared.program.source.clone(),
                    declaration_span: callee.name_span,
                    concrete: ConcreteCallSelection::Declaration,
                }
            );
        }
        assert_eq!(owner.arms[1].occurrences.len(), 1);
        assert!(owner.arms[1].call_occurrences.is_empty());
        let ScalarItem::Function(function) = &prepared.program.items[4] else {
            panic!("actual function item ordinal")
        };
        let arm = &function.overload_arms[0];
        let ScalarBlockItem::LocalBinding(binding) = &arm.body.items[0] else {
            panic!("projected binding")
        };
        let ScalarExpression::Call { arguments, .. } = &binding.value else {
            panic!("primary call")
        };
        let ScalarExpression::Call {
            arguments: nested, ..
        } = &arguments[0]
        else {
            panic!("nested argument")
        };
        assert_eq!(row.occurrences[0].span, expression_span(&binding.value));
        assert_eq!(row.occurrences[1].span, expression_span(&arguments[0]));
        assert_eq!(row.occurrences[2].span, expression_span(&nested[0]));
        assert_eq!(
            row.occurrences
                .iter()
                .filter(|metadata| metadata.span == expression_span(&arguments[0]))
                .count(),
            1
        );
        let assignment = arm
            .body
            .items
            .iter()
            .find_map(|item| match item {
                ScalarBlockItem::Assignment(assignment) => Some(assignment),
                _ => None,
            })
            .unwrap();
        let place = &assignment.targets[0].place;
        let ScalarPlace::Index { base, index, .. } = place else {
            panic!("projected selector")
        };
        assert_eq!(row.occurrences[15].span, scalar_place_target_span(place));
        assert_eq!(row.occurrences[16].span, scalar_place_target_span(base));
        assert_eq!(row.occurrences[17].span, expression_span(index));
        for (ordinal, metadata) in row.occurrences.iter().enumerate() {
            assert_eq!(
                metadata.role,
                if ordinal == 15 || ordinal == 16 {
                    OccurrenceRole::Place
                } else {
                    OccurrenceRole::Expression
                }
            );
        }
        for association in &row.call_occurrences {
            let metadata = row.occurrences[association.ordinal.0];
            assert_eq!(metadata.role, OccurrenceRole::Expression);
            assert_eq!(
                row.sites[association.site].source_span,
                SourceSpan::new(prepared.program.source.clone(), metadata.span)
            );
        }
        let qualified = (
            prepared.program.source.clone(),
            4,
            0,
            row.occurrences.clone(),
            row.call_occurrences.clone(),
        );
        if let Some(expected) = &expected_inventory {
            assert_eq!(&qualified, expected);
        } else {
            expected_inventory = Some(qualified);
        }
        let before = prepared.program.clone();
        let scope = Scope::new();
        let results = prepared.analyze().unwrap();
        assert_eq!(scope.counts().call_sites, 0);
        drop(scope);
        let mut installed = before.clone();
        let errors = results.install(&mut installed);
        assert_eq!(
            errors,
            vec![ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(before.source.clone(), ByteSpan::new(338, 342)),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            }]
        );
        assert_eq!(prepared.program, before);
        let finished = prepared.finish();
        assert_eq!(finished.program, installed);
        assert_eq!(
            finished.diagnostics,
            errors
                .iter()
                .map(ReferenceAnalysisError::diagnostic)
                .collect::<Vec<_>>()
        );
        let public = derive_scalar_program_from_cst_with_layout(parsed.canonical_cst(), target);
        assert_eq!(finished.program, public.program);
        assert_eq!(finished.diagnostics, public.diagnostics);
    }
}

#[test]
fn preparation_keeps_equal_span_independent_occurrences_and_earlier_legacy_call_row() {
    let text = "%%start\nu8(u8) number = fn(input) { input };\nunit(u8) probe = fn(input) { u8 left, u8 right = number(input) + input, number(input) + input; left; right; };\n%%end";
    for target in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        let parsed = parsed(text);
        let validated = derive_scalar_program_from_cst_with_layout(parsed.canonical_cst(), target);
        assert!(
            validated.diagnostics.is_empty(),
            "{:?}",
            validated.diagnostics
        );
        let mut declarations = derive_scalar_declarations_from_cst(parsed.canonical_cst());
        let ScalarItem::Function(function) = &mut declarations.items[1] else {
            panic!("probe")
        };
        let ScalarBlockItem::LocalBinding(binding) = &mut function.body.items[0] else {
            panic!("independent roots")
        };
        let ScalarExpression::Binary {
            left,
            span: root_span,
            ..
        } = &binding.value
        else {
            panic!("primary root")
        };
        let root_span = *root_span;
        let call_span = expression_span(left);
        let ScalarExpression::Binary { left, span, .. } = &mut binding.output_values[1].value
        else {
            panic!("second root")
        };
        *span = root_span;
        let ScalarExpression::Call { span, .. } = left.as_mut() else {
            panic!("second nested call")
        };
        *span = call_span;
        let scope = Scope::new();
        let copies = layout::observations::Scope::new();
        let prepared = PreparedProgram::new(declarations, target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(
            scope.counts(),
            Counts {
                call_sites: 1,
                ..Counts::default()
            }
        );
        assert_eq!(copies.counts().program_copies, 0);
        assert_eq!(copies.counts().module_copies, 0);
        assert_eq!(copies.counts().cfg_copies, 0);
        drop(copies);
        drop(scope);
        let row = prepared.calls.as_ref().unwrap().function(1);
        assert_eq!(row.occurrences.len(), 10);
        assert_eq!(row.occurrences[0], row.occurrences[4]);
        assert_eq!(row.occurrences[0].span, root_span);
        assert_eq!(row.occurrences[1], row.occurrences[5]);
        assert_eq!(row.occurrences[1].span, call_span);
        assert_eq!(
            row.call_occurrences,
            vec![
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(1),
                    site: 0
                },
                CallOccurrence {
                    ordinal: OccurrenceOrdinal(5),
                    site: 0
                },
            ]
        );
        assert_eq!(row.sites.len(), 1);
        assert_eq!(row.sites[0].source_span.range, call_span);
        let before = prepared.program.clone();
        let scope = Scope::new();
        let results = prepared.analyze().unwrap();
        assert_eq!(scope.counts().call_sites, 0);
        drop(scope);
        let mut installed = before.clone();
        assert!(results.install(&mut installed).is_empty());
        assert_eq!(prepared.program, before);
    }
}
