use super::*;

const TEXT: &str = "%%start
struct Payload { u8 value; }
u8 item = 1;
Payload(Payload) take_local = fn(input) { input };
u8(Payload) take_global = fn(decoy) { decoy; item };
%%end";

fn prepare_names(target: ScalarTargetLayout) -> (ScalarProgram, ModuleCallSites) {
    let parsed = parse_source(source(), TEXT.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
        target,
    );
    assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
    let calls = resolve_program_call_sites_with_occurrences(&typed.program, occurrences).unwrap();
    (typed.program, calls)
}

fn final_name(calls: &FunctionCallSites) -> OccurrenceOrdinal {
    calls.structure.blocks[calls.structure.root.0]
        .final_outputs()
        .next()
        .unwrap()
        .expression
}

#[test]
fn prepared_plain_names_select_reads_and_moves_by_identity_on_both_targets() {
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (mut program, calls) = prepare_names(target);
        let ScalarItem::Binding(module_item) = &program.items[0] else {
            panic!("module binding");
        };
        let module_token = SourceSpan::new(program.source.clone(), module_item.name_span);
        for (item, spelling) in [(1, "item"), (2, "decoy")] {
            let calls = calls.function(item);
            let PlainNameFacts::Prepared(identities) = &calls.plain_names else {
                panic!("prepared name identities");
            };
            let ScalarItem::Function(function) = &mut program.items[item] else {
                panic!("function declaration");
            };
            let parameter = ReferenceBindingId {
                source: program.source.clone(),
                function_span: function.span,
                declaration_span: function.parameter_spans[0],
                block_span: function.body.span,
                kind: ReferenceBindingKind::Declared,
            };
            let expected = if item == 1 {
                PlainNameIdentity::Lexical(parameter.clone())
            } else {
                PlainNameIdentity::ModuleValue(module_token.clone())
            };
            let ordinal = final_name(calls);
            assert_eq!(identities[&ordinal], expected);
            let Some(ScalarBlockItem::Expression(ScalarExpression::Name { name, span })) =
                function.body.items.last_mut()
            else {
                panic!("canonical final name");
            };
            let use_span = SourceSpan::new(program.source.clone(), *span);
            *name = spelling.to_owned();

            let (cfg, errors) = derive_function_reference_cfg(
                &program.source,
                function,
                &HashSet::from(["item".to_owned()]),
                calls,
                &program.structs,
                &program.enums,
                true,
            );
            assert!(errors.is_empty(), "{errors:?}");
            let point_kinds: Vec<_> = cfg
                .points
                .iter()
                .filter(|point| {
                    point.source_span == use_span
                        && !matches!(
                            point.kind,
                            CfgPointKind::ReturnOutput { .. } | CfgPointKind::Return
                        )
                })
                .map(|point| &point.kind)
                .collect();
            if item == 1 {
                assert!(
                    matches!(point_kinds.as_slice(),
                        [CfgPointKind::Read { binding }, CfgPointKind::Move { place, owner, .. }]
                            if binding == &parameter && owner == &parameter && place.binding == parameter
                            && matches!(&place.place, ScalarPlace::Name { name, .. } if name == spelling)
                    ),
                    "{point_kinds:?}"
                );
            } else {
                assert!(
                    matches!(point_kinds.as_slice(),
                        [CfgPointKind::Evaluate(ScalarExpression::Name { name, .. })] if name == spelling
                    ),
                    "{point_kinds:?}"
                );
            }
        }
    }
}

#[test]
fn prepared_plain_names_reject_missing_rows_at_the_exact_name_span_on_both_targets() {
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (program, mut calls) = prepare_names(target);
        let ScalarItem::Function(function) = &program.items[1] else {
            panic!("lexical name function");
        };
        let calls = calls.function_mut(1);
        let ordinal = final_name(calls);
        let [ScalarBlockItem::Expression(expression)] = function.body.items.as_slice() else {
            panic!("canonical final name");
        };
        let span = expression_span(expression);
        let PlainNameFacts::Prepared(identities) = &mut calls.plain_names else {
            panic!("prepared name identities");
        };
        let Some(PlainNameIdentity::Lexical(binding)) = identities.remove(&ordinal) else {
            panic!("lexical name identity");
        };
        let expected = ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(program.source.clone(), span),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        };
        let module_names = HashSet::from(["item".to_owned()]);
        let (cfg, errors) = derive_function_reference_cfg(
            &program.source,
            function,
            &module_names,
            calls,
            &program.structs,
            &program.enums,
            true,
        );
        assert_eq!(errors, [expected.clone()]);
        assert!(cfg
            .points
            .iter()
            .all(|point| point.source_span != *expected.source_span()));

        let mut builder = ReferenceFlowBuilder::new(
            &program.source,
            function,
            &module_names,
            calls,
            &program.structs,
            &program.enums,
        );
        builder.names.insert("input".to_owned(), binding.clone());
        builder
            .binding_types
            .insert(binding, ScalarType::Struct(program.structs[0].id.clone()));
        assert_eq!(
            builder.move_value(expression, ReferenceScopeId(0), ordinal),
            Err(expected)
        );
        assert!(builder.cfg.points.is_empty());
    }
}

#[test]
fn static_plain_name_obligation_rejects_a_removed_row_at_the_name_token() {
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (program, mut calls) = prepare_names(target);
        for item in [2, 1] {
            let row = calls.function_mut(item);
            let ordinal = final_name(row);
            let PlainNameFacts::Prepared(identities) = &mut row.plain_names else {
                panic!("prepared name identities");
            };
            assert!(identities.remove(&ordinal).is_some());
            assert!(row
                .structure
                .checked_types
                .expressions
                .contains_key(&ordinal));
        }
        let start = TEXT.find("{ input }").unwrap() as u32 + 2;
        let span = SourceSpan::new(program.source.clone(), ByteSpan::new(start, start + 5));
        let stages = stage_observations::Scope::new();
        let error = occurrence_obligations::validate_module(&calls).unwrap_err();
        assert_eq!(stages.counts(), stage_observations::Counts::default());
        assert_eq!(
            error,
            ReferenceAnalysisError::MissingContext {
                source_span: span.clone(),
                reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
            }
        );
        let message = "missing resolved binding";
        assert_eq!(
            error.diagnostic(),
            Diagnostic {
                code: "B0003".into(),
                severity: DiagnosticSeverity::Error,
                message: message.into(),
                labels: vec![DiagnosticLabel {
                    kind: DiagnosticLabelKind::Primary,
                    span,
                    message: message.into(),
                }],
                notes: Vec::new(),
            }
        );
    }
}

#[test]
fn prepared_plain_names_select_secondary_module_receiver_on_both_routes_and_targets() {
    let text = "%%start\nu8 first, u8 second = 1, 2;\nu8() run = fn { second };\n%%end";
    let source = source();
    let parsed = parse_source(source.clone(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = || derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    let token_span = |start| ByteSpan::new(start, start + 6);
    let declaration = token_span(text.find("second").unwrap() as u32);
    let occurrence = token_span(text.rfind("second").unwrap() as u32);
    let assert_identity = |row: &FunctionCallSites| {
        let ordinal = final_name(row);
        assert_eq!(row.occurrences[ordinal.0].span, occurrence);
        let PlainNameFacts::Prepared(identities) = &row.plain_names else {
            panic!("prepared name identities");
        };
        assert_eq!(
            identities[&ordinal],
            PlainNameIdentity::ModuleValue(SourceSpan::new(source.clone(), declaration))
        );
    };
    let lookup_only = stage_observations::Counts {
        call_sites: 1,
        ..stage_observations::Counts::default()
    };
    let build_only = stage_observations::Counts {
        reference_origins: 1,
        output_contracts: 1,
        copy_specialization: 1,
        ..stage_observations::Counts::default()
    };
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let stages = stage_observations::Scope::new();
        let standalone = static_calls::PreparedProgram::new(declarations(), target)
            .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(stages.counts(), lookup_only);
        assert_identity(standalone.function_calls(1));
        drop(stages);

        let stages = stage_observations::Scope::new();
        let finished = standalone.finish();
        assert_eq!(stages.counts(), build_only);
        assert!(
            finished.diagnostics.is_empty(),
            "{:?}",
            finished.diagnostics
        );
        drop(stages);

        let stages = stage_observations::Scope::new();
        let prepared = prepare_scalar_project(
            vec![(declarations(), Vec::new())],
            vec![source.clone()],
            ScalarPreparationInputs {
                target_layout: target,
            },
        )
        .unwrap_or_else(|error| panic!("{:?}", error.diagnostics));
        assert_eq!(stages.counts(), lookup_only);
        assert_identity(prepared.function_calls(0, 1));
        drop(stages);

        let stages = stage_observations::Scope::new();
        let build = analyze_scalar_build(&prepared);
        assert_eq!(stages.counts(), build_only);
        assert!(build.diagnostics().is_empty(), "{:?}", build.diagnostics());
        drop(stages);

        let program = finished.program;
        let ScalarItem::Function(function) = &program.items[1] else {
            panic!("run declaration");
        };
        let raw = resolve_program_call_sites(&program).unwrap();
        let module_names = HashSet::from(["first".to_owned(), "run".to_owned()]);
        let (_, errors) = derive_function_reference_cfg(
            &program.source,
            function,
            &module_names,
            raw.function(1),
            &program.structs,
            &program.enums,
            true,
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        let expected = ReferenceAnalysisError::MissingBinding {
            source_span: SourceSpan::new(source.clone(), occurrence),
            name: "second".to_owned(),
        };
        assert_eq!(errors[0].diagnostic(), expected.diagnostic());
    }
}
