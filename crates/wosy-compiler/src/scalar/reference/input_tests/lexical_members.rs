use super::*;

const TEXT: &str = "%%start
struct Payload { u8 value; }
struct Holder { Payload nested; u8 value; }
Payload(Holder, Holder) take = fn(input, decoy) { decoy.value; input.nested };
%%end";

fn prepare_members(target: ScalarTargetLayout) -> (ScalarProgram, ModuleCallSites) {
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

fn final_member(calls: &FunctionCallSites) -> OccurrenceOrdinal {
    calls.structure.blocks[calls.structure.root.0]
        .final_outputs()
        .next()
        .unwrap()
        .expression
}

fn install_parameters(builder: &mut ReferenceFlowBuilder<'_>, calls: &FunctionCallSites) {
    for slot in &calls.boundary.parameters {
        let binding = parameter(builder, slot.span);
        builder.names.insert(slot.name.clone(), binding.clone());
        builder.binding_types.insert(binding, slot.ty.clone());
    }
}

fn parameter(builder: &ReferenceFlowBuilder<'_>, span: ByteSpan) -> ReferenceBindingId {
    ReferenceBindingId {
        source: builder.source.clone(),
        function_span: builder.function_span,
        declaration_span: span,
        block_span: builder.call_sites.structure.blocks[builder.call_sites.structure.root.0].span,
        kind: ReferenceBindingKind::Declared,
    }
}

#[test]
fn prepared_lexical_members_use_frozen_receiver_field_and_type_on_both_targets() {
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (mut program, calls) = prepare_members(target);
        let raw = resolve_program_call_sites(&program).unwrap();
        let calls = calls.function(0);
        let ordinal = final_member(calls);
        let module_names = HashSet::from(["take".to_owned()]);
        let ScalarItem::Function(function) = &mut program.items[0] else {
            panic!("take declaration");
        };
        let derive = |calls| {
            derive_function_reference_cfg(
                &program.source,
                function,
                &module_names,
                calls,
                &program.structs,
                &program.enums,
                true,
            )
        };
        let (prepared_cfg, errors) = derive(calls);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(derive(raw.function(0)), (prepared_cfg, errors));

        let mut expression = match function.body.items.last().unwrap() {
            ScalarBlockItem::Expression(expression) => expression.clone(),
            _ => panic!("canonical final expression"),
        };
        let ScalarExpression::Member {
            receiver,
            name,
            receiver_span,
            span,
            ..
        } = &mut expression
        else {
            panic!("canonical final member");
        };
        *receiver = "decoy".to_owned();
        *name = "value".to_owned();
        let receiver_span = *receiver_span;
        let span = *span;
        let mut builder = ReferenceFlowBuilder::new(
            &program.source,
            function,
            &module_names,
            calls,
            &program.structs,
            &program.enums,
        );
        install_parameters(&mut builder, calls);
        let binding = parameter(&builder, function.parameter_spans[0]);
        let field = program.structs[1].fields[0].id.clone();
        let ty = ScalarType::Struct(program.structs[0].id.clone());
        let MemberFacts::Prepared(identities) = &calls.members else {
            panic!("prepared member identities");
        };
        assert_eq!(
            identities[&ordinal],
            PreparedMemberIdentity::Lexical {
                binding: binding.clone(),
                field: field.clone(),
            }
        );
        assert_eq!(calls.structure.checked_types.expressions[&ordinal], ty);

        builder.checked.insert(binding.clone());
        let scope = ReferenceScopeId(0);
        builder.expression(&expression, scope, ordinal).unwrap();
        builder.move_value(&expression, scope, ordinal).unwrap();
        let [preliminary, read, moved] = builder.cfg.points.as_slice() else {
            panic!(
                "ordered preliminary read, member read and move: {:?}",
                builder.cfg.points
            );
        };
        assert_eq!(
            preliminary.kind,
            CfgPointKind::Read {
                binding: binding.clone()
            }
        );
        assert_eq!(
            preliminary.source_span,
            SourceSpan::new(program.source.clone(), receiver_span)
        );
        assert_eq!(
            read.source_span,
            SourceSpan::new(program.source.clone(), span)
        );
        assert_eq!(moved.source_span, read.source_span);
        let place = synthetic_member_place("decoy", receiver_span, &field, span);
        let expected = builder.reference_place(&place, binding.clone(), None);
        assert_eq!(
            read.kind,
            CfgPointKind::ReadPlace {
                place,
                pointer_point: None,
                candidates: vec![expected.clone()],
                reads_origin: false,
            }
        );
        assert_eq!(
            moved.kind,
            CfgPointKind::Move {
                place: expected,
                ty,
                owner: binding
            }
        );
    }
}

#[test]
fn prepared_lexical_members_reject_missing_rows_at_the_exact_member_span_on_both_targets() {
    for target in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
        let (program, mut calls) = prepare_members(target);
        let calls = calls.function_mut(0);
        let ordinal = final_member(calls);
        let ScalarItem::Function(function) = &program.items[0] else {
            panic!("take declaration");
        };
        let Some(ScalarBlockItem::Expression(expression)) = function.body.items.last() else {
            panic!("canonical final member");
        };
        let MemberFacts::Prepared(identities) = &mut calls.members else {
            panic!("prepared member identities");
        };
        assert!(matches!(
            identities.remove(&ordinal),
            Some(PreparedMemberIdentity::Lexical { .. })
        ));

        let expected = ReferenceAnalysisError::MissingContext {
            source_span: SourceSpan::new(program.source.clone(), expression_span(expression)),
            reason: ReferenceAnalysisErrorReason::MissingResolvedBinding,
        };
        let module_names = HashSet::from(["take".to_owned()]);
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
        install_parameters(&mut builder, calls);
        assert_eq!(
            builder.move_value(expression, ReferenceScopeId(0), ordinal),
            Err(expected)
        );
        assert!(builder.cfg.points.is_empty());
    }
}
