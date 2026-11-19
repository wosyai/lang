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
fn prepared_plain_names_preserve_secondary_module_receiver_identity_gap() {
    let text = "%%start\nu8 first, u8 second = 1, 2;\nu8() run = fn { second };\n%%end";
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (typed, occurrences) = typecheck_scalar_program_with_occurrences(
            derive_scalar_declarations_from_cst(parsed.result.canonical_cst()),
            target,
        );
        assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);
        let program = typed.program;
        let raw = resolve_program_call_sites(&program).unwrap();
        let prepared = resolve_program_call_sites_with_occurrences(&program, occurrences).unwrap();
        let ScalarItem::Function(function) = &program.items[1] else {
            panic!("run declaration");
        };
        let module_names = HashSet::from(["first".to_owned(), "run".to_owned()]);
        let diagnostic = |calls: &ModuleCallSites| {
            let (_, errors) = derive_function_reference_cfg(
                &program.source,
                function,
                &module_names,
                calls.function(1),
                &program.structs,
                &program.enums,
                true,
            );
            assert_eq!(errors.len(), 1, "{errors:?}");
            errors[0].diagnostic()
        };
        let start = text.rfind("second").unwrap() as u32;
        let expected = ReferenceAnalysisError::MissingBinding {
            source_span: SourceSpan::new(program.source.clone(), ByteSpan::new(start, start + 6)),
            name: "second".to_owned(),
        };
        assert_eq!(diagnostic(&raw), expected.diagnostic());
        assert_eq!(diagnostic(&prepared), expected.diagnostic());
    }
}
