use super::*;
use control::{InitializationMember, InitializationStorage, ModuleInitializationDetail};
use reference::{InitializerMemberRequirement, OccurrenceOrdinal, OccurrenceRole};

fn initializer_member_tokens(
    topology: &control::ModuleInitializationTopology,
) -> Vec<(OccurrenceOrdinal, ByteSpan)> {
    topology
        .occurrences
        .iter()
        .enumerate()
        .filter_map(|(index, metadata)| match metadata.initializer_member {
            InitializerMemberRequirement::Token(span) => Some((OccurrenceOrdinal(index), span)),
            InitializerMemberRequirement::NotRequired => None,
        })
        .collect()
}

#[test]
fn nested_conditional_retains_signed_float_literal_and_contextual_result_types() {
    let text = "%%start\nf32(f32) delta = fn(direction) { if (direction > 0.0) { 1.0 / direction } else { if (direction < 0.0) { -1.0 / direction } else { -0.0 } } };\n%%end";
    let parsed = parsed(text);
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let project = prepare_extracted(&parsed, target);
        let standalone = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        for (module, items) in [
            (project.calls.module(0), &project.project().modules[0].items),
            (&standalone.calls, &standalone.program.items),
        ] {
            assert_eq!(occurrence_obligations::validate_module(module), Ok(()));
            let row = module.function(0);
            let ScalarItem::Function(function) = &items[0] else {
                panic!("delta function");
            };
            let ScalarExpression::If { else_branch, .. } = function.body.final_expression(0).unwrap().0 else {
                panic!("outer conditional");
            };
            let ScalarExpression::If { then_branch, else_branch, span, .. } = else_branch.final_expression(0).unwrap().0 else {
                panic!("nested conditional");
            };
            let ScalarExpression::Binary { left, .. } = then_branch.final_expression(0).unwrap().0 else {
                panic!("division");
            };
            for (literal, expected, negative) in [
                (left.as_ref(), "-1.0", -1.0_f64),
                (else_branch.final_expression(0).unwrap().0, "-0.0", -0.0_f64),
            ] {
                let ScalarExpression::Float { value, spelling, span } = literal else {
                    panic!("signed float literal");
                };
                assert_eq!(value.to_bits(), negative.to_bits());
                assert_eq!(spelling, expected);
                assert_eq!(&text[span.start as usize..span.end as usize], expected);
                let ordinal = row.occurrences.iter().position(|occurrence| {
                    occurrence.role == OccurrenceRole::Expression && occurrence.span == *span
                }).expect("signed literal occurrence");
                assert_eq!(
                    row.structure.checked_types.expressions[&OccurrenceOrdinal(ordinal)],
                    ScalarType::F32,
                );
            }
            let ordinal = row.occurrences.iter().position(|occurrence| {
                occurrence.role == OccurrenceRole::Expression && occurrence.span == *span
            }).expect("nested conditional occurrence");
            assert_eq!(
                row.structure.checked_types.expressions[&OccurrenceOrdinal(ordinal)],
                ScalarType::F32,
            );
        }
    }
}

#[test]
fn initializer_members_retain_selection_and_type_on_both_routes_and_layouts() {
    let text = "%%start\nstruct Record { u8 field; }\nenum Mode { ready; }\nRecord item = { .field = 1; };\nu8 extracted = item.field;\nMode status = Mode.ready;\n*?Record pointer = null;\nu8 indirect = pointer.field;\n%%end";
    let parsed = parsed(text);
    let span = |needle: &str, token: &str| {
        let start = (text.find(needle).unwrap() + needle.find(token).unwrap()) as u32;
        ByteSpan::new(start, start + token.len() as u32)
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let stages = Scope::new();
        let standalone = PreparedProgram::new(
            derive_scalar_declarations_from_cst(parsed.canonical_cst()),
            target,
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        let project = prepare_extracted(&parsed, target);
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 2,
                ..Default::default()
            }
        );
        drop(stages);
        let structure = &standalone.program.structs[0];
        let enumeration = &standalone.program.enums[0];
        for module in [&standalone.calls, project.calls.module(0)] {
            let region = &module.initialization;
            let ModuleInitializationDetail::Occurrences(topology) = &region.detail else {
                panic!("initializer topology")
            };
            let rows = initializer_member_tokens(topology);
            assert_eq!(
                rows.iter().map(|(_, span)| *span).collect::<Vec<_>>(),
                [
                    span("item.field", "field"),
                    span("Mode.ready", "ready"),
                    span("pointer.field", "field")
                ]
            );
            let field =
                |receiver, receiver_name: &str, through_raw_pointer| InitializationMember::Field {
                    receiver: InitializationStorage::Module(receiver),
                    receiver_name: receiver_name.to_owned(),
                    field: structure.fields[0].id.clone(),
                    through_raw_pointer,
                };
            for ((ordinal, _), (identity, ty)) in rows.iter().zip([
                (field(span("item =", "item"), "item", false), ScalarType::U8),
                (
                    InitializationMember::Enum {
                        enumeration: enumeration.id.clone(),
                        tag: 0,
                    },
                    ScalarType::Enum(enumeration.id.clone()),
                ),
                (
                    field(span("pointer =", "pointer"), "pointer", true),
                    ScalarType::U8,
                ),
            ]) {
                assert_eq!(topology.identities.members[ordinal], identity);
                assert_eq!(topology.structure.checked_types.expressions[ordinal], ty);
            }
            let stages = Scope::new();
            let mut corrupt = module.clone();
            let ModuleInitializationDetail::Occurrences(topology) =
                &mut corrupt.initialization.detail
            else {
                unreachable!()
            };
            for (ordinal, _) in rows.iter().rev() {
                assert!(topology.identities.members.remove(ordinal).is_some());
            }
            assert_eq!(
                occurrence_obligations::validate_module(&corrupt),
                Err(ReferenceAnalysisError::MissingContext {
                    source_span: SourceSpan::new(region.source.clone(), rows[0].1),
                    reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
                })
            );
            assert_eq!(stages.counts(), Counts::default());
        }
    }
}

#[test]
fn initializer_namespace_members_retain_selected_declaration_and_symbol_on_both_layouts() {
    let root_text = "%%start\nmath = namespace package \"math.w\";\ni32 result = math.value;\ni32(i32) callback = math.identity;\n%%end";
    let target_text = "%%start\ni32 value = 1;\ni32(i32) identity = fn(input) { input };\n%%end";
    let root = parsed(root_text);
    let target = SourceIdentity::new(
        "project".into(),
        "package".into(),
        "math.w".into(),
        "r1".into(),
    );
    let dependency = parse_source(target.clone(), target_text.to_owned(), &[]);
    assert!(
        dependency.diagnostics.is_empty(),
        "{:?}",
        dependency.diagnostics
    );
    for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let declarations = derive_scalar_declarations_from_cst(root.canonical_cst());
        let namespace = declarations.namespaces().next().unwrap();
        let binding = ScalarNamespaceBinding {
            binding: namespace.binding.clone(),
            span: namespace.span,
            target: target.clone(),
        };
        let qualifier = namespace.binding_span;
        let stages = Scope::new();
        let project = crate::prepare_scalar_project(
            vec![
                (declarations, vec![binding]),
                (
                    derive_scalar_declarations_from_cst(dependency.result.canonical_cst()),
                    Vec::new(),
                ),
            ],
            vec![target.clone(), root.source.clone()],
            crate::ScalarPreparationInputs { target_layout },
        )
        .unwrap_or_else(|failure| panic!("{:?}", failure.diagnostics));
        assert_eq!(
            stages.counts(),
            Counts {
                call_sites: 2,
                ..Default::default()
            }
        );
        drop(stages);
        let ModuleInitializationDetail::Occurrences(topology) =
            &project.initialization_region(0).unwrap().detail
        else {
            panic!("initializer topology")
        };
        let ordered = initializer_member_tokens(topology);
        assert_eq!(ordered.len(), 2);
        for (&(ordinal, _), name) in ordered.iter().zip(["value", "identity"]) {
            let start = target_text.find(name).unwrap() as u32;
            assert_eq!(
                topology.identities.members[&ordinal],
                InitializationMember::Module {
                    qualifier,
                    declaration: SourceSpan::new(
                        target.clone(),
                        ByteSpan::new(start, start + name.len() as u32)
                    ),
                    name: name.to_owned(),
                }
            );
            assert!(matches!(
                (
                    &topology.structure.checked_types.expressions[&ordinal],
                    name
                ),
                (ScalarType::I32, "value") | (ScalarType::Callable { .. }, "identity")
            ));
        }
        let mut corrupt = project.calls.module(0).clone();
        let ModuleInitializationDetail::Occurrences(topology) = &mut corrupt.initialization.detail
        else {
            unreachable!()
        };
        for (ordinal, _) in ordered.iter().rev() {
            assert!(topology.identities.members.remove(ordinal).is_some());
        }
        let start = (root_text.find("math.value").unwrap() + "math.".len()) as u32;
        let stages = Scope::new();
        assert_eq!(
            occurrence_obligations::validate_module(&corrupt),
            Err(ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(root.source.clone(), ByteSpan::new(start, start + 5)),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            })
        );
        assert_eq!(stages.counts(), Counts::default());
    }
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
