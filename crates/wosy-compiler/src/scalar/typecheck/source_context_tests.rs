use super::*;
use crate::parse_source;

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/main.w".into(),
        "r1".into(),
    )
}

fn expected(source: &SourceIdentity, message: &str, span: ByteSpan) -> Diagnostic {
    Diagnostic {
        code: "B0003".into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        labels: vec![DiagnosticLabel {
            kind: DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span),
            message: message.into(),
        }],
        notes: Vec::new(),
    }
}

#[test]
fn standalone_extern_validation_has_zero_constructions_and_copies_across_independent_axes() {
    for externs in [4, 8, 16] {
        for body in [4, 8, 16] {
            let mut text = String::from("%%start\n");
            for index in 0..externs {
                text.push_str(&format!(
                    "env_{index} = extern wasm \"helper\" {{ unit() flush; }};\n"
                ));
            }
            text.push_str("i32(i32) identity = fn(value) { ");
            for _ in 1..body {
                text.push_str("value; ");
            }
            text.push_str("value };\n%%end");
            let parsed = parse_source(source(), text, &[]);
            assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
            let outer = layout::observations::Scope::new();
            let result = derive_scalar_program(&parsed.result);
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            let program = result.program;
            assert_eq!(program.source, source());
            assert_eq!(program.items.len(), externs + 1);
            for (index, item) in program.items[..externs].iter().enumerate() {
                let ScalarItem::Extern(declaration) = item else {
                    panic!("extern {index}")
                };
                assert_eq!(declaration.binding, format!("env_{index}"));
                assert_eq!(declaration.kind, "wasm");
                assert_eq!(
                    declaration.actual_module,
                    ScalarExternModule::Valid("helper".into())
                );
                assert_eq!(declaration.functions.len(), 1);
                let function = &declaration.functions[0];
                assert_eq!(function.name, "flush");
                assert!(
                    matches!(&function.signature, ScalarType::Callable { outputs, parameters } if outputs.outputs.len() == 1 && outputs.outputs[0].ty == ScalarType::Unit && parameters.is_empty())
                );
            }
            let ScalarItem::Function(function) = &program.items[externs] else {
                panic!("identity")
            };
            assert_eq!(function.name, "identity");
            assert_eq!(function.parameters, ["value"]);
            assert_eq!(function.body.items.len(), body);
            assert_eq!(
                function.body.terminated_items,
                (0..body).map(|index| index + 1 < body).collect::<Vec<_>>()
            );
            for item in &function.body.items {
                assert!(
                    matches!(item, ScalarBlockItem::Expression(ScalarExpression::Name { name, .. }) if name == "value")
                );
            }
            let overloads = ScalarOverloadEnvironment::single(&program.source, &program.items);
            let owners = ScalarAssignmentOwners::new(&program.items);
            let mut inference = ScalarInferenceBuilder::new(&owners);
            let scope = layout::observations::Scope::new();
            let diagnostics = validate(&program, &overloads, &mut inference);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            assert!(inference.freeze().is_empty());
            assert_eq!(scope.counts(), layout::observations::Counts::default());
            assert_eq!(program.source, source());
            assert_eq!(program.items.len(), externs + 1);
            drop(scope);
            assert!(outer.counts().module_constructions > 0);
            assert!(outer.counts().program_copies > 0);
            assert!(outer.counts().cfg_copies > 0);
        }
    }
}

#[test]
fn source_only_extern_preserves_complete_ordered_recursive_type_and_style_diagnostics() {
    let parsed = parse_source(
        source(),
        "%%start\nenv = extern wasm \"helper\" { unit() flush; };\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty());
    let result = derive_scalar_program(&parsed.result);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Extern(mut declaration) = result.program.items.into_iter().next().unwrap()
    else {
        panic!("extern")
    };
    declaration.kind = "unsupported".into();
    let signature_span = declaration.functions[0].signature_span;
    let name_span = declaration.functions[0].name_span;
    let nested_span = ByteSpan::new(1, 2);
    let named_span = ByteSpan::new(3, 4);
    let qualified_span = ByteSpan::new(5, 6);
    declaration.functions[0].name = "camelCase".into();
    declaration.functions[0].signature = ScalarType::Callable {
        outputs: ScalarOutputSequence {
            span: signature_span,
            outputs: vec![ScalarOutput {
                span: nested_span,
                ty: ScalarType::Callable {
                    outputs: ScalarOutputSequence {
                        span: nested_span,
                        outputs: vec![ScalarOutput {
                            span: nested_span,
                            ty: ScalarType::CheckedReference {
                                mutability: ScalarReferenceMutability::Shared,
                                inner: Box::new(ScalarType::Array {
                                    element: Box::new(ScalarType::Named {
                                        name: "Missing".into(),
                                        span: named_span,
                                    }),
                                    length: 2,
                                    length_span: nested_span,
                                    span: nested_span,
                                }),
                            },
                        }],
                    },
                    parameters: vec![ScalarType::RawPointer(Box::new(ScalarType::I32))],
                },
            }],
        },
        parameters: vec![ScalarType::RuntimeArray {
            span: signature_span,
            element: Box::new(ScalarType::Qualified {
                receiver: "other".into(),
                receiver_span: qualified_span,
                member: "Missing".into(),
                member_span: qualified_span,
                span: qualified_span,
            }),
        }],
    };
    for source in [
        source(),
        SourceIdentity::new(
            "other-project".into(),
            "other-package".into(),
            "other.w".into(),
            "r2".into(),
        ),
    ] {
        let mut diagnostics = Vec::new();
        validate_extern(&source, &declaration, &mut diagnostics);
        let expected = vec![
            expected(&source, "unsupported extern declaration", declaration.span),
            expected(
                &source,
                "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE",
                name_span,
            ),
            expected(&source, "unknown scalar type", named_span),
            expected(&source, "unsupported raw pointer type", nested_span),
            expected(&source, "unknown scalar type", qualified_span),
            expected(
                &source,
                "direct FFI arrays are not supported",
                signature_span,
            ),
            expected(
                &source,
                "direct FFI checked references are not supported",
                signature_span,
            ),
        ];
        assert_eq!(diagnostics, expected);
        assert_eq!(
            serde_json::to_vec(&diagnostics).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
    }
}

#[test]
fn source_only_extern_preserves_module_pointer_rules_and_invalid_declaration_cases() {
    let parsed = parse_source(
        source(),
        "%%start\nenv = extern wasm \"helper\" { *?u8(*?u8) flush; };\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty());
    let result = derive_scalar_program(&parsed.result);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let ScalarItem::Extern(mut declaration) = result.program.items.into_iter().next().unwrap()
    else {
        panic!("extern")
    };
    let source = source();
    let mut diagnostics = Vec::new();
    validate_extern(&source, &declaration, &mut diagnostics);
    assert!(diagnostics.is_empty());
    declaration.actual_module = ScalarExternModule::Invalid {
        span: declaration.module_span,
        error_span: declaration.module_span,
    };
    validate_extern(&source, &declaration, &mut diagnostics);
    assert_eq!(
        diagnostics,
        [expected(
            &source,
            "unsupported extern declaration",
            declaration.span
        )]
    );
    declaration.actual_module = ScalarExternModule::Valid("helper".into());
    declaration.functions.clear();
    diagnostics.clear();
    validate_extern(&source, &declaration, &mut diagnostics);
    assert_eq!(
        diagnostics,
        [expected(
            &source,
            "unsupported extern declaration",
            declaration.span
        )]
    );
}

#[test]
fn diagnostic_wrappers_preserve_exact_source_qualified_structure_and_bytes() {
    let parsed = parse_source(
        source(),
        "%%start\ni32(i32) identity = fn(value) { value };\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty());
    let result = derive_scalar_program(&parsed.result);
    assert!(result.diagnostics.is_empty());
    let program = result.program;
    let span = ByteSpan::new(2, 7);
    let expected = expected(&program.source, "example diagnostic", span);
    assert_eq!(
        diagnostic(&program, "B0003", "example diagnostic", span),
        expected
    );
    let module = ScalarModule::from_program(program, Vec::new());
    let actual = module_diagnostic(&module, "B0003", "example diagnostic", span);
    assert_eq!(actual, expected);
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}
