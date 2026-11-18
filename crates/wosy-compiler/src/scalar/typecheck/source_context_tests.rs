use super::*;
use crate::parse_source;

mod project_context_tests;

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
            let overloads = overloads.typing(0);
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
            assert_eq!(outer.counts().module_constructions, 0);
            assert_eq!(outer.counts().program_copies, 0);
            assert_eq!(outer.counts().module_copies, 0);
            assert_eq!(outer.counts().cfg_copies, 0);
        }
    }
}

#[test]
fn standalone_place_entry_borrows_actual_declarations_and_resolves_fields_without_integral_copies()
{
    for target_layout in [ScalarTargetLayout::WASM32, ScalarTargetLayout::NATIVE64] {
        for structs in [4, 8, 16] {
            for body in [4, 8, 16] {
                let mut text = String::from("%%start\nenum Mode { ready; }\n");
                for index in 0..structs {
                    text.push_str(&format!("struct Record{index} {{ i32 value; }}\n"));
                }
                for index in 0..structs {
                    text.push_str(&format!(
                        "i32(Record{index}) read_{index} = fn(record) {{ unsafe {{ "
                    ));
                    for _ in 0..body {
                        text.push_str("&?record.value; ");
                    }
                    text.push_str("}; record.value };\n");
                }
                text.push_str("bool() mode = fn { Mode.ready == Mode.ready };\nu32(i32) bits = fn(value) { core.bitcast<u32>(value) };\n%%end");
                let parsed = parse_source(source(), text.clone(), &[]);
                assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
                let public = layout::observations::Scope::new();
                let result = derive_scalar_program_with_layout(&parsed.result, target_layout);
                assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
                assert_eq!(public.counts().program_copies, 0);
                assert_eq!(public.counts().module_copies, 0);
                assert_eq!(public.counts().struct_copies, 0);
                assert_eq!(public.counts().cfg_copies, 0);
                drop(public);
                let mut program = result.program;
                assert_eq!(program.structs.len(), structs);
                assert_eq!(program.items.len(), structs + 2);
                assert_eq!(program.enums.len(), 1);
                let pointers = {
                    let typing = ScalarProgramTyping {
                        source: &program.source,
                        structs: &program.structs,
                        enums: &program.enums,
                        target_layout: program.target_layout,
                    };
                    assert!(std::ptr::eq(typing.source, &program.source));
                    assert!(std::ptr::eq(typing.structs, program.structs.as_slice()));
                    assert!(std::ptr::eq(typing.enums, program.enums.as_slice()));
                    assert_eq!(typing.target_layout, target_layout);
                    (
                        typing.source as *const SourceIdentity,
                        typing.structs.as_ptr(),
                        typing.enums.as_ptr(),
                    )
                };
                for (index, item) in program.items[..structs].iter_mut().enumerate() {
                    let ScalarItem::Function(function) = item else {
                        panic!("read function")
                    };
                    assert_eq!(function.name, format!("read_{index}"));
                    assert_eq!(function.parameters, ["record"]);
                    assert_eq!(function.body.items.len(), 2);
                    assert_eq!(function.body.terminated_items, [true, false]);
                    let ScalarBlockItem::Expression(ScalarExpression::Block(block)) =
                        &mut function.body.items[0]
                    else {
                        panic!("unsafe body")
                    };
                    assert!(block.unsafe_context);
                    assert_eq!(block.items.len(), body);
                    assert!(block.terminated_items.iter().all(|terminated| *terminated));
                    for item in &mut block.items {
                        let ScalarBlockItem::Expression(ScalarExpression::RawAddress {
                            place: ScalarPlace::Field { field, span, base },
                            ..
                        }) = item
                        else {
                            panic!("field address")
                        };
                        assert!(
                            matches!(base.as_ref(), ScalarPlace::Name { name, .. } if name == "record")
                        );
                        assert_eq!(
                            *field,
                            ScalarFieldReference::Resolved(
                                program.structs[index].fields[0].id.clone()
                            )
                        );
                        let field_span = ByteSpan::new(span.end - "value".len() as u32, span.end);
                        assert_eq!(
                            &text[field_span.start as usize..field_span.end as usize],
                            "value"
                        );
                        *field = ScalarFieldReference::Unresolved {
                            name: "value".into(),
                            span: field_span,
                        };
                    }
                }
                let ScalarItem::Function(mode) = &mut program.items[structs] else {
                    panic!("mode")
                };
                let ScalarBlockItem::Expression(ScalarExpression::Binary { left, right, .. }) =
                    &mut mode.body.items[0]
                else {
                    panic!("enum comparison")
                };
                for expression in [left, right] {
                    let ScalarExpression::Member { enum_tag, .. } = expression.as_mut() else {
                        panic!("enum member")
                    };
                    assert_eq!(*enum_tag, Some(0));
                    *enum_tag = None;
                }
                let overloads = ScalarOverloadEnvironment::single(&program.source, &program.items);
                let view = overloads.typing(0);
                let owners = ScalarAssignmentOwners::new(&program.items);
                let scope = layout::observations::Scope::new();
                resolution::resolve_program_places(&mut program, &view, &owners);
                assert_eq!(scope.counts(), layout::observations::Counts::default());
                drop(scope);
                assert!(std::ptr::eq(&program.source, pointers.0));
                assert_eq!(program.structs.as_ptr(), pointers.1);
                assert_eq!(program.enums.as_ptr(), pointers.2);
                assert_eq!(program.source, source());
                for (index, structure) in program.structs.iter().enumerate() {
                    assert_eq!(structure.name, format!("Record{index}"));
                    assert_eq!(
                        structure.id,
                        ScalarStructId {
                            source: source(),
                            index
                        }
                    );
                    assert_eq!(structure.fields.len(), 1);
                    assert_eq!(structure.fields[0].ty, ScalarType::I32);
                }
                for (index, item) in program.items[..structs].iter().enumerate() {
                    let ScalarItem::Function(function) = item else {
                        panic!("read function")
                    };
                    assert!(
                        matches!(&function.signature, ScalarType::Callable { parameters, .. } if parameters == &[ScalarType::Struct(program.structs[index].id.clone())])
                    );
                    let ScalarBlockItem::Expression(ScalarExpression::Block(block)) =
                        &function.body.items[0]
                    else {
                        panic!("unsafe body")
                    };
                    assert_eq!(block.items.len(), body);
                    for item in &block.items {
                        let ScalarBlockItem::Expression(ScalarExpression::RawAddress {
                            place: ScalarPlace::Field { field, base, .. },
                            ..
                        }) = item
                        else {
                            panic!("field address")
                        };
                        assert!(
                            matches!(base.as_ref(), ScalarPlace::Name { name, .. } if name == "record")
                        );
                        assert_eq!(
                            *field,
                            ScalarFieldReference::Resolved(ScalarStructFieldId {
                                structure: ScalarStructId {
                                    source: source(),
                                    index
                                },
                                index: 0
                            })
                        );
                    }
                }
                assert_eq!(program.enums[0].id.source, source());
                let ScalarItem::Function(mode) = &program.items[structs] else {
                    panic!("mode")
                };
                let ScalarBlockItem::Expression(ScalarExpression::Binary { left, right, .. }) =
                    &mode.body.items[0]
                else {
                    panic!("enum comparison")
                };
                for expression in [left, right] {
                    assert!(matches!(
                        expression.as_ref(),
                        ScalarExpression::Member {
                            enum_tag: Some(0),
                            ..
                        }
                    ));
                }
                let mut inference = ScalarInferenceBuilder::new(&owners);
                let diagnostics = validate(&program, &view, &mut inference);
                assert!(diagnostics.is_empty(), "{diagnostics:?}");
                assert!(inference.freeze().is_empty());
            }
        }
    }
}

#[test]
fn borrowed_standalone_typing_preserves_local_index_and_full_field_identity_predicates() {
    let parsed = parse_source(
        source(),
        "%%start\nstruct Record { i32 value; }\n%%end".into(),
        &[],
    );
    assert!(parsed.diagnostics.is_empty());
    let result = derive_scalar_program(&parsed.result);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let program = result.program;
    let typing = ScalarProgramTyping {
        source: &program.source,
        structs: &program.structs,
        enums: &program.enums,
        target_layout: program.target_layout,
    };
    let foreign = ScalarStructId {
        source: SourceIdentity::new(
            "foreign".into(),
            "package".into(),
            "src/main.w".into(),
            "r1".into(),
        ),
        index: 0,
    };
    let span = ByteSpan::new(5, 10);
    let mut diagnostics = Vec::new();
    assert_eq!(
        field_type(
            Some(&ScalarType::Struct(foreign.clone())),
            "value",
            span,
            span,
            &typing,
            &mut diagnostics
        ),
        ScalarType::I32
    );
    assert!(diagnostics.is_empty());
    let place = ScalarPlace::Field {
        base: Box::new(ScalarPlace::Name {
            name: "record".into(),
            span,
        }),
        field: ScalarFieldReference::Resolved(ScalarStructFieldId {
            structure: foreign.clone(),
            index: 0,
        }),
        span,
    };
    let lexical = BTreeMap::from([("record".into(), ScalarType::Struct(foreign))]);
    let overloads = ScalarOverloadEnvironment::single(&program.source, &program.items);
    let view = overloads.typing(0);
    let owners = ScalarAssignmentOwners::new(&program.items);
    let mut inference = ScalarInferenceBuilder::new(&owners);
    assert_eq!(
        place_type(
            &place,
            &lexical,
            &typing,
            &mut diagnostics,
            &view,
            &mut inference
        ),
        ScalarType::Error
    );
    assert_eq!(
        diagnostics,
        [expected(&program.source, "value has no field", span)]
    );
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
    let typing = ScalarProgramTyping {
        source: &program.source,
        structs: &program.structs,
        enums: &program.enums,
        target_layout: program.target_layout,
    };
    assert_eq!(
        diagnostic(&typing, "B0003", "example diagnostic", span),
        expected
    );
    let mut module = ScalarModule::from_program(program, Vec::new());
    let (modules, _items) = split_module_typing(std::slice::from_mut(&mut module));
    let actual = module_diagnostic(&modules[0], "B0003", "example diagnostic", span);
    assert_eq!(actual, expected);
    assert_eq!(
        serde_json::to_vec(&actual).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}
