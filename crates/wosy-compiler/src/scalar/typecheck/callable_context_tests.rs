use super::*;
use crate::parse_source;

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "main.w".into(),
        "r1".into(),
    )
}

fn program() -> ScalarProgram {
    let parsed = parse_source(source(), "%%start\ni32(i32) identity = fn(value) { value };\nenv = extern wasm \"helper\" { i32() fetch; };\n%%end".into(), &[]);
    assert!(parsed.diagnostics.is_empty());
    let result = derive_scalar_program(&parsed.result);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.program
}

fn callable(ty: ScalarType) -> ScalarType {
    ScalarType::Callable {
        outputs: ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty,
                span: ByteSpan::new(1, 2),
            }],
            span: ByteSpan::new(1, 2),
        },
        parameters: Vec::new(),
    }
}

fn call(
    receiver: Option<&str>,
    name: &str,
    type_arguments: Vec<ScalarTypeArgument>,
) -> ScalarExpression {
    ScalarExpression::Call {
        receiver: receiver.map(str::to_owned),
        name: name.into(),
        receiver_span: receiver.map(|_| ByteSpan::new(10, 13)),
        name_span: ByteSpan::new(14, 19),
        span: ByteSpan::new(10, 24),
        type_arguments,
        arguments: Vec::new(),
        overload_selection: None,
    }
}

fn type_argument() -> ScalarTypeArgument {
    ScalarTypeArgument {
        ty: ScalarType::I32,
        span: ByteSpan::new(20, 23),
    }
}

fn expected(source: &SourceIdentity, code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: DiagnosticSeverity::Error,
        message: message.into(),
        labels: vec![DiagnosticLabel {
            kind: DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), ByteSpan::new(10, 24)),
            message: message.into(),
        }],
        notes: Vec::new(),
    }
}

fn single_type(
    program: &ScalarProgram,
    view: &ScalarTypingView<'_>,
    scope: &BTreeMap<String, ScalarType>,
    call: &ScalarExpression,
) -> (ScalarType, Vec<Diagnostic>) {
    let owners = ScalarAssignmentOwners::new(&program.items);
    let mut inference = ScalarInferenceBuilder::new(&owners);
    let mut diagnostics = Vec::new();
    let ty = expression_type(
        call,
        scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        program,
        &mut diagnostics,
        false,
        view,
        &mut inference,
    );
    assert!(inference.freeze().is_empty());
    (ty, diagnostics)
}

fn module_type(
    modules: &[ScalarModule],
    ordinal: usize,
    view: &ScalarTypingView<'_>,
    call: &ScalarExpression,
) -> (ScalarType, Vec<Diagnostic>) {
    let module = &modules[ordinal];
    let owners = ScalarAssignmentOwners::new(&module.items);
    let mut inference = ScalarInferenceBuilder::new(&owners);
    let mut diagnostics = Vec::new();
    let ty = expression_type_in_module(
        call,
        &module.members,
        &BTreeSet::new(),
        &BTreeMap::new(),
        module,
        modules,
        &mut diagnostics,
        false,
        view,
        &mut inference,
    );
    assert!(inference.freeze().is_empty());
    (ty, diagnostics)
}

#[test]
fn late_extern_member_is_available_to_outputs_and_standalone_but_first_project_binding_controls_expression(
) {
    for empty in [false, true] {
        let mut program = program();
        let ScalarItem::Extern(first) = &mut program.items[1] else {
            panic!("extern")
        };
        let late = first.clone();
        if empty {
            first.functions.clear();
        } else {
            first.functions[0].name = "other".into();
        }
        program.items.push(ScalarItem::Extern(late));
        let expression = call(Some("env"), "fetch", Vec::new());
        let environment = ScalarOverloadEnvironment::single(&program.source, &program.items);
        let view = environment.typing(0);
        assert_eq!(
            single_type(&program, &view, &BTreeMap::new(), &expression),
            (ScalarType::I32, Vec::new())
        );
        assert_eq!(
            call_output_sequence(&expression, &BTreeMap::new(), &program, &view)
                .unwrap()
                .outputs[0]
                .ty,
            ScalarType::I32
        );
        let modules = [ScalarModule::from_program(program, Vec::new())];
        let environment = ScalarOverloadEnvironment::project(&modules);
        let view = environment.typing(0);
        assert_eq!(
            call_output_sequence_in_module(
                &expression,
                &modules[0].members,
                &modules[0],
                &modules,
                &view
            )
            .unwrap()
            .outputs[0]
                .ty,
            ScalarType::I32
        );
        assert_eq!(
            module_type(&modules, 0, &view, &expression),
            (
                ScalarType::Error,
                vec![expected(
                    &modules[0].source,
                    "B0001",
                    "unknown callable name"
                )]
            )
        );
    }
}

#[test]
fn duplicate_extern_markers_keep_standalone_or_project_first_and_qualified_lexical_precedence() {
    for same_declaration in [false, true] {
        let mut program = program();
        let ScalarItem::Extern(first) = &mut program.items[1] else {
            panic!("extern")
        };
        let mut later = first.functions[0].clone();
        later.unsafe_marker = true;
        later.signature = callable(ScalarType::Bool);
        if same_declaration {
            first.functions.push(later);
        } else {
            let mut duplicate = first.clone();
            duplicate.functions = vec![later];
            program.items.push(ScalarItem::Extern(duplicate));
        }
        let expression = call(Some("env"), "fetch", Vec::new());
        let environment = ScalarOverloadEnvironment::single(&program.source, &program.items);
        let view = environment.typing(0);
        let unsafe_diagnostic = expected(
            &program.source,
            "B0012",
            "unsafe extern call requires an unsafe block",
        );
        assert_eq!(
            single_type(&program, &view, &BTreeMap::new(), &expression),
            (ScalarType::I32, vec![unsafe_diagnostic.clone()])
        );
        let lexical = BTreeMap::from([("env.fetch".into(), callable(ScalarType::Bool))]);
        assert_eq!(
            single_type(&program, &view, &lexical, &expression),
            (ScalarType::Bool, vec![unsafe_diagnostic])
        );
        assert_eq!(
            call_output_sequence(&expression, &lexical, &program, &view)
                .unwrap()
                .outputs[0]
                .ty,
            ScalarType::I32
        );
        let generic_call = call(Some("env"), "fetch", vec![type_argument()]);
        assert!(call_output_sequence(&generic_call, &lexical, &program, &view).is_none());
        assert_eq!(
            single_type(&program, &view, &lexical, &generic_call),
            (
                ScalarType::Error,
                vec![expected(
                    &program.source,
                    "B0004",
                    "generic argument arity does not match callable declaration"
                )]
            )
        );
        let modules = [ScalarModule::from_program(program, Vec::new())];
        let environment = ScalarOverloadEnvironment::project(&modules);
        let view = environment.typing(0);
        assert_eq!(
            module_type(&modules, 0, &view, &expression),
            (ScalarType::I32, Vec::new())
        );
    }
}

#[test]
fn first_empty_generic_declaration_controls_both_typing_routes() {
    let mut program = program();
    let ScalarItem::Function(first) = &mut program.items[0] else {
        panic!("function")
    };
    first.signature = callable(ScalarType::I32);
    let mut later = first.clone();
    later.generic_parameters = vec![ScalarGenericParameter {
        name: "T".into(),
        span: ByteSpan::new(1, 2),
    }];
    later.signature = callable(ScalarType::Bool);
    program.items.push(ScalarItem::Function(later));
    let environment = ScalarOverloadEnvironment::single(&program.source, &program.items);
    let view = environment.typing(0);
    let scope = BTreeMap::from([("identity".into(), callable(ScalarType::Bool))]);
    let expression = call(None, "identity", vec![type_argument(), type_argument()]);
    assert_eq!(
        call_output_sequence(&expression, &scope, &program, &view)
            .unwrap()
            .outputs[0]
            .ty,
        ScalarType::Bool
    );
    assert_eq!(
        single_type(&program, &view, &scope, &expression),
        (ScalarType::Bool, Vec::new())
    );
    let no_arguments = call(None, "identity", Vec::new());
    assert_eq!(
        single_type(&program, &view, &scope, &no_arguments),
        (ScalarType::Bool, Vec::new())
    );
    let modules = [ScalarModule::from_program(program, Vec::new())];
    let environment = ScalarOverloadEnvironment::project(&modules);
    let view = environment.typing(0);
    assert_eq!(
        call_output_sequence_in_module(
            &expression,
            &modules[0].members,
            &modules[0],
            &modules,
            &view
        )
        .unwrap()
        .outputs[0]
            .ty,
        ScalarType::I32
    );
    assert_eq!(
        module_type(&modules, 0, &view, &expression),
        (ScalarType::I32, Vec::new())
    );
}

#[test]
fn local_ordinal_and_first_import_source_choose_independent_generic_headers() {
    let mut first = program();
    let ScalarItem::Function(function) = &mut first.items[0] else {
        panic!("function")
    };
    function.signature = callable(ScalarType::Bool);
    let mut second = first.clone();
    let ScalarItem::Function(function) = &mut second.items[0] else {
        panic!("function")
    };
    function.generic_parameters = vec![ScalarGenericParameter {
        name: "T".into(),
        span: ByteSpan::new(1, 2),
    }];
    function.signature = callable(ScalarType::Named {
        name: "T".into(),
        span: ByteSpan::new(1, 2),
    });
    let first = ScalarModule::from_program(first, Vec::new());
    let second = ScalarModule::from_program(
        second,
        vec![ScalarNamespaceBinding {
            binding: "peer".into(),
            span: ByteSpan::new(1, 2),
            target: first.source.clone(),
        }],
    );
    let modules = [first, second];
    let environment = ScalarOverloadEnvironment::project(&modules);
    let view = environment.typing(1);
    let local = call(None, "identity", vec![type_argument()]);
    assert_eq!(
        module_type(&modules, 1, &view, &local),
        (ScalarType::I32, Vec::new())
    );
    assert_eq!(
        call_output_sequence_in_module(&local, &modules[1].members, &modules[1], &modules, &view)
            .unwrap()
            .outputs[0]
            .ty,
        ScalarType::I32
    );
    let imported = call(Some("peer"), "identity", vec![type_argument()]);
    assert_eq!(
        call_output_sequence_in_module(
            &imported,
            &modules[1].members,
            &modules[1],
            &modules,
            &view
        )
        .unwrap()
        .outputs[0]
            .ty,
        ScalarType::Bool
    );
    assert_eq!(
        module_type(&modules, 1, &view, &imported),
        (ScalarType::Bool, Vec::new())
    );
    let imported = call(Some("peer"), "identity", Vec::new());
    assert_eq!(
        module_type(&modules, 1, &view, &imported),
        (ScalarType::Bool, Vec::new())
    );
    let local_missing_argument = call(None, "identity", Vec::new());
    assert!(call_output_sequence_in_module(
        &local_missing_argument,
        &modules[1].members,
        &modules[1],
        &modules,
        &view
    )
    .is_none());
    assert_eq!(
        module_type(&modules, 1, &view, &local_missing_argument),
        (
            ScalarType::Error,
            vec![expected(
                &modules[1].source,
                "B0004",
                "generic argument arity does not match callable declaration"
            )]
        )
    );
}

#[test]
fn imported_extern_name_presence_keeps_visibility_gate_and_includes_private_names() {
    for visible in [false, true] {
        let mut target = program();
        let ScalarItem::Function(function) = &mut target.items[0] else {
            panic!("function")
        };
        function.name = "_private".into();
        function.generic_parameters = vec![ScalarGenericParameter {
            name: "T".into(),
            span: ByteSpan::new(1, 2),
        }];
        function.signature = callable(ScalarType::Named {
            name: "T".into(),
            span: ByteSpan::new(1, 2),
        });
        let ScalarItem::Extern(declaration) = &mut target.items[1] else {
            panic!("extern")
        };
        declaration.binding = "unrelated_binding".into();
        declaration.functions[0].name = "_private".into();
        let mut target = ScalarModule::from_program(target, Vec::new());
        if visible {
            target
                .members
                .insert("_private".into(), callable(ScalarType::I32));
        }
        let mut current_program = program();
        current_program.source = SourceIdentity::new(
            "other".into(),
            "package".into(),
            "caller.w".into(),
            "r1".into(),
        );
        let current = ScalarModule::from_program(
            current_program,
            vec![ScalarNamespaceBinding {
                binding: "peer".into(),
                span: ByteSpan::new(1, 2),
                target: target.source.clone(),
            }],
        );
        let modules = [target, current];
        let environment = ScalarOverloadEnvironment::project(&modules);
        let view = environment.typing(1);
        let expression = call(Some("peer"), "_private", vec![type_argument()]);
        assert!(call_output_sequence_in_module(
            &expression,
            &modules[1].members,
            &modules[1],
            &modules,
            &view
        )
        .is_none());
        let diagnostic = if visible {
            expected(
                &modules[1].source,
                "B0004",
                "generic argument arity does not match callable declaration",
            )
        } else {
            Diagnostic {
                code: "M0002".into(),
                severity: DiagnosticSeverity::Error,
                message: "unknown module member".into(),
                labels: vec![
                    DiagnosticLabel {
                        kind: DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(modules[1].source.clone(), ByteSpan::new(14, 19)),
                        message: "unknown module member".into(),
                    },
                    DiagnosticLabel {
                        kind: DiagnosticLabelKind::Secondary,
                        span: SourceSpan::new(modules[0].source.clone(), ByteSpan::new(0, 0)),
                        message: "module resolved here".into(),
                    },
                ],
                notes: Vec::new(),
            }
        };
        assert_eq!(
            module_type(&modules, 1, &view, &expression),
            (ScalarType::Error, vec![diagnostic])
        );
    }
}
