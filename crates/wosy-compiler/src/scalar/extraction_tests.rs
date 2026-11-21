use super::*;
use crate::parse_source;

fn source() -> SourceIdentity {
    SourceIdentity::new(
        "project".into(),
        "package".into(),
        "src/extract.w".into(),
        "r1".into(),
    )
}

fn assert_empty_cfg(function: &ScalarFunction) {
    assert!(function.reference_cfg.facts.is_empty());
    assert!(function.reference_cfg.points.is_empty());
    assert!(function.reference_cfg.initial_points.is_empty());
    assert!(function.reference_cfg.allocations.is_empty());
    assert!(function.reference_cfg.specialized_policies.is_empty());
    for arm in &function.overload_arms {
        assert_empty_cfg(arm);
    }
}

#[test]
fn extraction_preserves_complete_ordered_unresolved_declarations_and_borrowed_namespaces() {
    let text = "%%start\nfirst = namespace app \"first.w\";\nstruct Outer { Inner inner; *?Inner pointer; }\nenum Mode { pending; ready; }\nenv = extern wasm \"helper\" { i32(i32) echo; unit() flush; };\nInner global = { .value = 1; };\ngeneric T; T(T) identity = fn(value) { value };\npick = overload { i32(i32) => fn(value) { value }; bool(bool) => fn(value) { value }; };\nfirst = namespace other \"second.w\";\nstruct Inner { u32 value; }\nInner(Inner) ordinary = fn(value) { next = value; next };\nfirst.run();\n%%end";
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let scope = layout::observations::Scope::new();
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert_eq!(scope.counts(), layout::observations::Counts::default());
    drop(scope);
    assert_eq!(declarations.source(), &source());
    assert!(std::ptr::eq(declarations.source(), &declarations.source));
    assert!(declarations.diagnostics().is_empty());
    assert!(std::ptr::eq(
        declarations.diagnostics(),
        declarations.diagnostics.as_slice()
    ));
    let membership = declarations
        .items
        .iter()
        .map(|item| match item {
            ScalarItem::Namespace(namespace) => ("namespace", namespace.binding.as_str()),
            ScalarItem::Extern(declaration) => ("extern", declaration.binding.as_str()),
            ScalarItem::Binding(binding) => ("binding", binding.name.as_str()),
            ScalarItem::Function(function) => ("function", function.name.as_str()),
            ScalarItem::Executable(_) => ("executable", ""),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        membership,
        [
            ("namespace", "first"),
            ("extern", "env"),
            ("binding", "global"),
            ("function", "identity"),
            ("function", "pick"),
            ("namespace", "first"),
            ("function", "ordinary"),
            ("executable", "")
        ]
    );
    let namespaces = declarations.namespaces().collect::<Vec<_>>();
    assert_eq!(namespaces.len(), 2);
    for (index, position, package, path) in [
        (0, 0, "app", "\"first.w\""),
        (1, 5, "other", "\"second.w\""),
    ] {
        let ScalarItem::Namespace(namespace) = &declarations.items[position] else {
            panic!("namespace")
        };
        assert!(std::ptr::eq(namespaces[index], namespace));
        assert_eq!(namespace.binding, "first");
        assert_eq!(namespace.package, package);
        assert_eq!(namespace.path, path);
        assert_eq!(
            &text[namespace.binding_span.start as usize..namespace.binding_span.end as usize],
            "first"
        );
        assert_eq!(
            &text[namespace.package_span.start as usize..namespace.package_span.end as usize],
            package
        );
        assert_eq!(
            &text[namespace.path_span.start as usize..namespace.path_span.end as usize],
            path
        );
    }
    assert_eq!(declarations.structs.len(), 2);
    for (index, (structure, name)) in declarations
        .structs
        .iter()
        .zip(["Outer", "Inner"])
        .enumerate()
    {
        assert_eq!(structure.name, name);
        assert_eq!(
            structure.id,
            ScalarStructId {
                source: source(),
                index
            }
        );
        for (field_index, field) in structure.fields.iter().enumerate() {
            assert_eq!(
                field.id,
                ScalarStructFieldId {
                    structure: structure.id.clone(),
                    index: field_index
                }
            );
        }
    }
    assert_eq!(
        declarations.structs[0]
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["inner", "pointer"]
    );
    assert!(
        matches!(&declarations.structs[0].fields[0].ty, ScalarType::Named { name, .. } if name == "Inner")
    );
    assert!(
        matches!(&declarations.structs[0].fields[1].ty, ScalarType::RawPointer(inner) if matches!(inner.as_ref(), ScalarType::Named { name, .. } if name == "Inner"))
    );
    assert_eq!(declarations.structs[1].fields[0].name, "value");
    assert_eq!(declarations.structs[1].fields[0].ty, ScalarType::U32);
    assert_eq!(declarations.enums.len(), 1);
    assert_eq!(
        declarations.enums[0].id,
        ScalarEnumId {
            source: source(),
            index: 0
        }
    );
    assert_eq!(declarations.enums[0].name, "Mode");
    assert_eq!(
        declarations.enums[0]
            .variants
            .iter()
            .map(|variant| (variant.name.as_str(), variant.tag))
            .collect::<Vec<_>>(),
        [("pending", 0), ("ready", 1)]
    );
    let ScalarItem::Extern(extern_decl) = &declarations.items[1] else {
        panic!("extern")
    };
    assert_eq!(
        extern_decl.actual_module,
        ScalarExternModule::Valid("helper".into())
    );
    assert_eq!(
        extern_decl
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        ["echo", "flush"]
    );
    let ScalarItem::Binding(binding) = &declarations.items[2] else {
        panic!("binding")
    };
    assert!(matches!(&binding.declared_type, ScalarType::Named { name, .. } if name == "Inner"));
    let ScalarExpression::StructLiteral { fields, .. } = &binding.value else {
        panic!("literal")
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "value");
    let ScalarItem::Function(generic) = &declarations.items[3] else {
        panic!("generic")
    };
    assert_eq!(generic.generic_parameters.len(), 1);
    assert_eq!(generic.generic_parameters[0].name, "T");
    assert_eq!(generic.parameters, ["value"]);
    assert_eq!(generic.body.items.len(), 1);
    let ScalarItem::Function(overload) = &declarations.items[4] else {
        panic!("overload")
    };
    assert_eq!(overload.overload_arms.len(), 2);
    for (arm, ty) in overload
        .overload_arms
        .iter()
        .zip([ScalarType::I32, ScalarType::Bool])
    {
        assert!(
            matches!(&arm.signature, ScalarType::Callable { parameters, outputs } if parameters == &[ty.clone()] && outputs.outputs[0].ty == ty)
        );
        assert_eq!(arm.body.items.len(), 1);
    }
    let ScalarItem::Function(ordinary) = &declarations.items[6] else {
        panic!("ordinary")
    };
    assert!(
        matches!(&ordinary.signature, ScalarType::Callable { parameters, outputs } if matches!(&parameters[0], ScalarType::Named { name, .. } if name == "Inner") && matches!(&outputs.outputs[0].ty, ScalarType::Named { name, .. } if name == "Inner"))
    );
    assert_eq!(ordinary.body.items.len(), 2);
    assert!(
        matches!(&ordinary.body.items[0], ScalarBlockItem::Assignment(assignment) if assignment.targets[0].target == "next")
    );
    let ScalarItem::Executable(ScalarBlockItem::Expression(ScalarExpression::Call {
        receiver,
        name,
        overload_selection,
        ..
    })) = &declarations.items[7]
    else {
        panic!("qualified executable")
    };
    assert_eq!(receiver.as_deref(), Some("first"));
    assert_eq!(name, "run");
    assert!(overload_selection.is_none());
    for item in &declarations.items {
        if let ScalarItem::Function(function) = item {
            assert_empty_cfg(function);
        }
    }
}

#[test]
fn extraction_uses_exact_existing_diagnostic_order_and_preserves_error_items() {
    let text = "%%start\nu8[-1] array = [0];\nf64 floating = 1e400;\ni32 integer = 1__2;\nchar letter = 'ab';\nstd.utf8 string = \"\\q\";\n%%end";
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let canonical = parsed.result.canonical_cst();
    let expected = derive_scalar_diagnostics_from_cst(canonical);
    let scope = layout::observations::Scope::new();
    let declarations = derive_scalar_declarations_from_cst(canonical);
    assert_eq!(scope.counts(), layout::observations::Counts::default());
    drop(scope);
    assert_eq!(declarations.diagnostics(), expected);
    assert_eq!(
        declarations
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>(),
        [
            "invalid string literal",
            "invalid character literal",
            "invalid integer literal",
            "invalid finite floating-point literal",
            "fixed array length must be an unsigned u64"
        ]
    );
    assert!(declarations
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic
            .labels
            .iter()
            .all(|label| label.span.source == source())));
    assert_eq!(declarations.items.len(), 5);
    let ScalarItem::Binding(array) = &declarations.items[0] else {
        panic!("array")
    };
    assert_eq!(array.declared_type, ScalarType::Error);
    assert!(declarations.namespaces().next().is_none());
    let standalone = derive_scalar_program_from_cst(canonical);
    assert_eq!(
        &standalone.diagnostics[..expected.len()],
        expected.as_slice()
    );
}

#[test]
fn extraction_leaves_cfg_empty_and_standalone_retains_ownership_rejection() {
    let text = "%%start\nstruct Payload { u8 value; }\nPayload(Payload) consume = fn(value) { next = value; next; next };\n%%end";
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty());
    let scope = layout::observations::Scope::new();
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert_eq!(scope.counts(), layout::observations::Counts::default());
    drop(scope);
    assert!(declarations.diagnostics().is_empty());
    let ScalarItem::Function(function) = &declarations.items[0] else {
        panic!("function")
    };
    assert_empty_cfg(function);
    assert_eq!(function.body.items.len(), 3);
    let wasm = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
    let native = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);
    assert_eq!(wasm.diagnostics, native.diagnostics);
    for validation in [&wasm, &native] {
        assert_eq!(validation.program.resolved_assignment_outputs.len(), 1);
        let diagnostic = validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "read or move of unavailable place")
            .expect("ownership rejection");
        assert_eq!(diagnostic.code, "B0003");
        let start = text.rfind("next };").unwrap() as u32;
        assert_eq!(
            diagnostic.labels[0].span,
            SourceSpan::new(source(), ByteSpan::new(start, start + 4))
        );
    }
    assert_empty_cfg(function);
}

#[test]
fn target_independent_declarations_feed_actual_standalone_entries_with_layout_parity_and_difference(
) {
    let text = "%%start\nstruct Pair { *?u8 address; u32 count; }\nu32(u32) identity = fn(value) { value };\n%%end";
    let parsed = parse_source(source(), text.into(), &[]);
    assert!(parsed.diagnostics.is_empty());
    let scope = layout::observations::Scope::new();
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert_eq!(scope.counts(), layout::observations::Counts::default());
    drop(scope);
    assert!(declarations.diagnostics().is_empty());
    let wasm = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::WASM32);
    let native = derive_scalar_program_with_layout(&parsed.result, ScalarTargetLayout::NATIVE64);
    assert!(wasm.diagnostics.is_empty(), "{:?}", wasm.diagnostics);
    assert!(native.diagnostics.is_empty(), "{:?}", native.diagnostics);
    assert_eq!(wasm.program.source, *declarations.source());
    assert_eq!(native.program.source, *declarations.source());
    assert_eq!(wasm.program.structs, declarations.structs);
    assert_eq!(native.program.structs, declarations.structs);
    assert_eq!(wasm.program.enums, declarations.enums);
    assert_eq!(native.program.enums, declarations.enums);
    assert_eq!(wasm.program.items, native.program.items);
    let id = &declarations.structs[0].id;
    assert_eq!(
        wasm.program.layouts.aggregate(id).unwrap().layout,
        Some(ScalarLayout {
            size: 8,
            alignment: 4
        })
    );
    assert_eq!(
        native.program.layouts.aggregate(id).unwrap().layout,
        Some(ScalarLayout {
            size: 16,
            alignment: 8
        })
    );
    assert_eq!(
        wasm.program
            .layouts
            .field(&declarations.structs[0].fields[1].id)
            .unwrap()
            .offset,
        Some(4)
    );
    assert_eq!(
        native
            .program
            .layouts
            .field(&declarations.structs[0].fields[1].id)
            .unwrap()
            .offset,
        Some(8)
    );
    let ScalarItem::Function(extracted) = &declarations.items[0] else {
        panic!("extracted function")
    };
    let ScalarItem::Function(completed) = &wasm.program.items[0] else {
        panic!("completed function")
    };
    assert_eq!(extracted.body, completed.body);
    assert_empty_cfg(extracted);
    assert!(!completed.reference_cfg.points.is_empty());
}
