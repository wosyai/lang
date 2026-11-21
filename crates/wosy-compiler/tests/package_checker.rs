#![cfg(feature = "internal-checker")]

use wosy_compiler::{
    check_scalar_module_proof, check_scalar_package, derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project,
    ScalarCheckerOperation, ScalarCheckerOutcome, ScalarCheckerScope, ScalarNamespaceBinding,
    ScalarModuleProofOutcome, ScalarPreparationInputs, ScalarProofModuleInput, ScalarTargetLayout,
};
use wosy_syntax::SourceIdentity;

const OUTPUT: &str = r#"{ .stdout = "given-out"; .stderr = "given-err"; .code = 23; .files = []; }"#;

fn source() -> SourceIdentity {
    SourceIdentity::new("local".into(), "demo".into(), "src/main.w".into(), "phase4".into())
}

fn check(text: &str) -> ScalarCheckerOutcome {
    check_scoped(text, ScalarCheckerScope::default())
}

fn check_scoped(text: &str, scope: ScalarCheckerScope) -> ScalarCheckerOutcome {
    let identity = source();
    let text = format!("%%start\n{text}\n%%end");
    let parsed = parse_source(identity.clone(), text, &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    check_scalar_package(
        vec![(declarations, Vec::new())],
        vec![identity],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        scope,
    )
}

#[test]
fn internal_checker_works_without_emission_and_keeps_authored_output() {
    let text = format!(
        "unit(u8) selected = fn(value) {{ core.require(value <= 7, {OUTPUT}); core.require(false, {OUTPUT}); }}; selected(8);",
    );
    let ScalarCheckerOutcome::Failed(failure) = check(&text) else {
        panic!("expected first authored failure");
    };
    assert_eq!(failure.source, source());
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    assert_eq!(&*failure.stdout, b"given-out");
    assert_eq!(&*failure.stderr, b"given-err");
    assert_eq!(failure.code, 23.into());
    let canonical = format!("%%start\n{text}\n%%end");
    let at = |span: wosy_syntax::ByteSpan| &canonical[span.start as usize..span.end as usize];
    assert_eq!(at(failure.condition), "value <= 7");
    assert_eq!(at(failure.stdout_span), "\"given-out\"");
    assert_eq!(at(failure.stderr_span), "\"given-err\"");
    assert_eq!(at(failure.code_span), "23");
    assert_eq!(at(failure.files_span), "[]");
    assert!(failure.files.is_empty());
    assert!(at(failure.directive).starts_with("core.require(value <= 7,"));
}

#[test]
fn internal_checker_keeps_pass_invalid_unsupported_and_no_checks_distinct() {
    assert_eq!(
        check(r#"f64(f64) irrelevant = fn(value) { value }; u8 written = 1;"#),
        ScalarCheckerOutcome::NoChecks,
    );
    assert_eq!(
        check(&format!("f64(f64) irrelevant = fn(value) {{ value }}; unit(u8) selected = fn(value) {{ core.narrow(value <= 7, {OUTPUT}); core.require(value <= 7, {OUTPUT}); }};")),
        ScalarCheckerOutcome::Passed,
    );
    assert_eq!(
        check(&format!("unit(u8) selected = fn(value) {{ core.require(!(value == 7), {OUTPUT}); }};")),
        ScalarCheckerOutcome::Unsupported,
    );
    assert!(matches!(
        check("unit() selected = fn { core.require(true, {}); };"),
        ScalarCheckerOutcome::Invalid(_),
    ));
    assert_eq!(
        check(&format!("unit() selected = fn {{ if (false) {{ core.require(false, {OUTPUT}); }}; }}; selected();")),
        ScalarCheckerOutcome::NoChecks,
    );
    assert_eq!(
        check(&format!("unit() unused = fn {{ if (false) {{ core.require(false, {OUTPUT}); }}; }};")),
        ScalarCheckerOutcome::NoChecks,
    );
    assert_eq!(check(&format!("core.require(!(true), {OUTPUT});")), ScalarCheckerOutcome::Unsupported);
    let narrowed = check(&format!("unit(u8) selected = fn(value) {{ core.narrow(value > 7, {OUTPUT}); }}; selected(7);"));
    assert!(matches!(narrowed, ScalarCheckerOutcome::Failed(failure) if failure.operation == ScalarCheckerOperation::Narrow));
}

#[test]
fn build_preparation_does_not_enable_checker_directives() {
    let identity = source();
    let text = format!("unit() selected = fn {{ core.require(true, {OUTPUT}); }}; selected();");
    let parsed = parse_source(identity.clone(), format!("%%start\n{text}\n%%end"), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let declarations = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
    assert!(prepare_scalar_project(
        vec![(declarations, Vec::new())],
        vec![identity],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ).is_err());
    assert_eq!(check(&text), ScalarCheckerOutcome::Passed);
}


#[test]
fn malformed_initialization_order_cannot_be_reported_as_no_checks() {
    let app = source();
    let other = SourceIdentity::new("local".into(), "demo".into(), "src/other.w".into(), "phase4".into());
    let declarations = |source: &SourceIdentity| {
        let parsed = parse_source(source.clone(), "%%start\nu8 inert = 1;\n%%end".into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    assert_eq!(
        check_scalar_package(
            vec![(declarations(&app), vec![]), (declarations(&other), vec![])],
            vec![app.clone(), other.clone(), app],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
            ScalarCheckerScope::default(),
        ),
        ScalarCheckerOutcome::Unsupported,
    );
}


#[test]
fn selected_failure_returns_only_authored_utf8_text_files_and_their_spans() {
    let text = r#"unit() required = fn {
        core.require(true, { .stdout = ""; .stderr = ""; .code = 1;
            .files = [{ .path = "ignore.txt"; .text = "not emitted"; }]; });
        core.narrow(false, { .stdout = ""; .stderr = ""; .code = 9;
            .files = [
                { .path = "Out/Report.txt"; .text = "ação\nline 2\t☕"; },
                { .path = "empty.txt"; .text = ""; }
            ]; });
    }; required();"#;
    let ScalarCheckerOutcome::Failed(failure) = check(text) else {
        panic!("expected concrete text output from the selected failure");
    };
    assert_eq!(failure.operation, ScalarCheckerOperation::Narrow);
    let [first, second] = failure.files.as_ref() else { panic!("exactly two authored files"); };
    assert_eq!(&*first.path, "Out/Report.txt");
    assert_eq!(&*first.text, "ação\nline 2\t☕".as_bytes());
    assert_eq!(&*second.path, "empty.txt");
    assert!(second.text.is_empty());
    let canonical = format!("%%start\n{text}\n%%end");
    let at = |span: wosy_syntax::ByteSpan| &canonical[span.start as usize..span.end as usize];
    assert_eq!(at(first.path_span), r#""Out/Report.txt""#);
    assert_eq!(at(first.text_span), r#""ação\nline 2\t☕""#);
    assert!(at(failure.files_span).contains("empty.txt"));
}


#[test]
fn main_flow_scope_omits_unused_roots_but_preserves_reachable_failures() {
    let unused = format!("unit() never_called = fn {{ core.require(false, {OUTPUT}); }};");
    let main = "unit() start = fn {}; start();";
    for text in [unused.clone(), format!("{unused} {main}")] {
        assert!(matches!(check(&text), ScalarCheckerOutcome::Failed(_)));
        assert_eq!(check_scoped(&text, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::NoChecks);
    }
    let main = format!("unit() start = fn {{ core.require(false, {OUTPUT}); }}; start();");
    let combined = format!("{unused} {main}");
    let default = check(&combined);
    assert!(matches!(default, ScalarCheckerOutcome::Failed(ref failure) if failure.directive.start > unused.len() as u32));
    assert_eq!(check_scoped(&combined, ScalarCheckerScope::MainFlowOnly), default);
}


#[test]
fn main_flow_scope_still_checks_selected_namespace_dispatch() {
    let app = source();
    let library = SourceIdentity::new("local".into(), "demo".into(), "src/lib.w".into(), "phase4".into());
    let app_text = format!(
        "lib = namespace demo \"src/lib.w\"; unit() never_called = fn {{ core.require(false, {OUTPUT}); }};",
    );
    let load = |source: &SourceIdentity, text: &str, target: Option<&SourceIdentity>| {
        let parsed = parse_source(source.clone(), format!("%%start\n{text}\n%%end"), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let parsed = derive_scalar_declarations_from_cst(parsed.result.canonical_cst());
        let bindings = target.map(|target| {
            let namespace = parsed.namespaces().into_iter().next().unwrap();
            ScalarNamespaceBinding {
                binding: namespace.binding.clone(),
                target: target.clone(),
                span: namespace.span,
            }
        }).into_iter().collect();
        (parsed, bindings)
    };
    let observe = |target, lib_text, lib_target| check_scalar_package(
        vec![load(&app, &app_text, target), load(&library, lib_text, lib_target)],
        vec![app.clone(), library.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ScalarCheckerScope::MainFlowOnly,
    );
    let callable = "unit() harmless = fn {};";
    assert_eq!(observe(Some(&library), callable, None), ScalarCheckerOutcome::NoChecks);
    assert!(matches!(observe(None, callable, None), ScalarCheckerOutcome::Invalid(_) | ScalarCheckerOutcome::Unsupported));
    assert!(matches!(
        observe(Some(&library), "root = namespace demo \"src/main.w\";", Some(&app)),
        ScalarCheckerOutcome::Invalid(_) | ScalarCheckerOutcome::Unsupported,
    ));
}


#[test]
fn main_initializer_reaches_concrete_top_level_directives_in_source_order() {
    for scope in [ScalarCheckerScope::default(), ScalarCheckerScope::MainFlowOnly] {
        let passing = format!("core.require(true, {OUTPUT}); core.narrow(true, {OUTPUT});");
        assert_eq!(check_scoped(&passing, scope), ScalarCheckerOutcome::Passed);
        for (name, operation) in [
            ("require", ScalarCheckerOperation::Require),
            ("narrow", ScalarCheckerOperation::Narrow),
        ] {
            let first = r#"{ .stdout = "first"; .stderr = "authored"; .code = 23; .files = []; }"#;
            let text = format!("{passing} core.{name}(false, {first}); core.require(false, {OUTPUT});");
            let ScalarCheckerOutcome::Failed(failure) = check_scoped(&text, scope) else {
                panic!("expected selected first concrete top-level directive");
            };
            assert_eq!(failure.operation, operation);
            assert_eq!(&*failure.stdout, b"first");
            assert_eq!(&*failure.stderr, b"authored");
            assert_eq!(failure.code, 23.into());
            let canonical = format!("%%start\n{text}\n%%end");
            let at = |span: wosy_syntax::ByteSpan| &canonical[span.start as usize..span.end as usize];
            assert_eq!(at(failure.condition), "false");
            assert_eq!(at(failure.directive), format!("core.{name}(false, {first})"));
        }
    }
}

#[test]
fn direct_initializer_failure_across_namespace_dispatch_keeps_its_own_source() {
    let app = source();
    let lib = SourceIdentity::new("local".into(), "demo".into(), "src/lib.w".into(), "phase4".into());
    let app_text = format!("%%start\ncore.require(true, {OUTPUT}); lib = namespace demo \"src/lib.w\"; core.narrow(false, {OUTPUT});\n%%end");
    let lib_text = format!("%%start\ncore.require(false, {OUTPUT});\n%%end");
    let load = |source: &SourceIdentity, text: &str| {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    let root = load(&app, &app_text);
    let namespace = root.namespaces().into_iter().next().unwrap();
    let binding = ScalarNamespaceBinding { binding: namespace.binding.clone(), target: lib.clone(), span: namespace.span };
    let ScalarCheckerOutcome::Failed(failure) = check_scalar_package(
        vec![(root, vec![binding]), (load(&lib, &lib_text), vec![])],
        vec![app, lib.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ScalarCheckerScope::MainFlowOnly,
    ) else {
        panic!("expected the earlier direct directive in dependency initializer");
    };
    assert_eq!(failure.source, lib);
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
}


#[test]
fn main_initializer_compares_only_canonical_typed_integer_literals() {
    for condition in [
        "2 == 2", "-3 != -2", "-2147483648 < -1", "4 <= 4", "7 > 6", "2147483647 >= 0",
    ] {
        assert_eq!(
            check_scoped(&format!("core.require({condition}, {OUTPUT});"), ScalarCheckerScope::MainFlowOnly),
            ScalarCheckerOutcome::Passed,
            "true condition {condition}",
        );
    }
    for condition in ["-3 == 4", "2 != 2", "7 < 6", "4 <= 3", "-9 > -8", "6 >= 7"] {
        let text = format!("core.narrow({condition}, {OUTPUT});");
        let ScalarCheckerOutcome::Failed(failure) = check(&text) else {
            panic!("expected first failure for {condition}");
        };
        assert_eq!(failure.operation, ScalarCheckerOperation::Narrow);
        let canonical = format!("%%start\n{text}\n%%end");
        assert_eq!(&canonical[failure.condition.start as usize..failure.condition.end as usize], condition);
    }
    for condition in ["1 + 1 == 2", "1.0 < 2.0", "!(1 < 2)"] {
        match check(&format!("core.require({condition}, {OUTPUT});")) {
            ScalarCheckerOutcome::Unsupported => (),
            ScalarCheckerOutcome::Invalid(diagnostics) => {
                assert!(diagnostics.iter().any(|diagnostic| diagnostic.code == "B0003"), "{condition}: {diagnostics:?}");
            }
            other => panic!("expected {condition} to fail closed, got {other:?}"),
        }
    }
    assert!(matches!(check(&format!("core.require(2147483648 >= 0, {OUTPUT});")), ScalarCheckerOutcome::Invalid(_)));
}

#[test]
fn main_initializer_reads_checked_bool_installations() {
    assert_eq!(
        check_scoped(
            &format!("bool ready = true; core.require(ready, {OUTPUT}); core.narrow(ready, {OUTPUT});"),
            ScalarCheckerScope::MainFlowOnly,
        ),
        ScalarCheckerOutcome::Passed,
    );
    let canonical = format!("%%start\nbool ready = false; core.require(ready, {OUTPUT});\n%%end");
    let ScalarCheckerOutcome::Failed(failure) =
        check(&format!("bool ready = false; core.require(ready, {OUTPUT});")) else {
            panic!("expected failure from the installed false value");
        };
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    assert_eq!(&canonical[failure.condition.start as usize..failure.condition.end as usize], "ready");
    assert_eq!(
        check(&format!("bool ready = true; ready = false; core.require(ready, {OUTPUT});")),
        ScalarCheckerOutcome::Unsupported,
    );
}

#[test]
fn main_initializer_keeps_bool_storage_separate_across_namespaces() {
    let app = source();
    let lib = SourceIdentity::new("local".into(), "demo".into(), "src/lib.w".into(), "phase4".into());
    let app_text = format!("%%start\nbool ready = true; lib = namespace demo \"src/lib.w\"; core.require(ready, {OUTPUT});\n%%end");
    let lib_text = format!("%%start\nbool ready = false; core.narrow(ready, {OUTPUT});\n%%end");
    let load = |source: &SourceIdentity, text: &str| {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    let root = load(&app, &app_text);
    let namespace = root.namespaces().into_iter().next().unwrap();
    let binding = ScalarNamespaceBinding { binding: namespace.binding.clone(), target: lib.clone(), span: namespace.span };
    let ScalarCheckerOutcome::Failed(failure) = check_scalar_package(
        vec![(root, vec![binding]), (load(&lib, &lib_text), vec![])],
        vec![app, lib.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ScalarCheckerScope::MainFlowOnly,
    ) else {
        panic!("expected the namespace to use its own false value");
    };
    assert_eq!(failure.source, lib);
    assert_eq!(failure.operation, ScalarCheckerOperation::Narrow);
}

#[test]
fn main_initializer_follows_only_a_selected_unit_conditional_arm() {
    assert_eq!(
        check_scoped(
            &format!("bool enabled = false; if (enabled) {{ core.require(false, {OUTPUT}); }};"),
            ScalarCheckerScope::MainFlowOnly,
        ),
        ScalarCheckerOutcome::NoChecks,
    );
    assert_eq!(
        check_scoped(
            &format!("bool enabled = true; if (enabled) {{ if (2 <= 2) {{ core.require(true, {OUTPUT}); }}; }};"),
            ScalarCheckerScope::MainFlowOnly,
        ),
        ScalarCheckerOutcome::Passed,
    );
    let text = format!("if (false) {{ core.narrow(false, {OUTPUT}); }}; if (true) {{ core.require(false, {OUTPUT}); }}; core.narrow(false, {OUTPUT});");
    let ScalarCheckerOutcome::Failed(failure) = check(&text) else {
        panic!("expected the selected conditional arm to fail first");
    };
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    let canonical = format!("%%start\n{text}\n%%end");
    assert_eq!(&canonical[failure.condition.start as usize..failure.condition.end as usize], "false");
    assert_eq!(failure.condition.start as usize, canonical.find("core.require(false").unwrap() + "core.require(".len());
}

#[test]
fn main_initializer_does_not_assume_unknown_branch_state_or_skip_reached_mutation() {
    let body = format!("core.require(false, {OUTPUT});");
    assert_eq!(
        check(&format!("bool enabled = true; if (enabled) {{ enabled = false; {body} }};")),
        ScalarCheckerOutcome::Unsupported,
    );
    assert_eq!(
        check(&format!("bool enabled = true; if (!enabled) {{ {body} }};")),
        ScalarCheckerOutcome::Unsupported,
    );
}

#[test]
fn main_initializer_compares_canonical_installed_integer_literals() {
    let text = format!(
        "i8 low = -128; i8 high = 127; core.require(low < high, {OUTPUT}); core.narrow(low == -128, {OUTPUT}); core.require(high >= 127, {OUTPUT});",
    );
    assert_eq!(check_scoped(&text, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);
    let text = format!("u8 current = 255; core.require(current < 255, {OUTPUT});");
    let ScalarCheckerOutcome::Failed(failure) = check(&text) else {
        panic!("expected failure from the canonically installed u8");
    };
    let canonical = format!("%%start\n{text}\n%%end");
    assert_eq!(&canonical[failure.condition.start as usize..failure.condition.end as usize], "current < 255");
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    assert_eq!(
        check(&format!("i8 value = -1; value = 1; core.require(value < 0, {OUTPUT});")),
        ScalarCheckerOutcome::Unsupported,
    );
    assert!(matches!(
        check(&format!("u8 value = 256; core.require(value > 0, {OUTPUT});")),
        ScalarCheckerOutcome::Invalid(_),
    ));
}

#[test]
fn explicit_proof_module_checks_unused_pure_callable_over_arbitrary_inputs() {
    let identity = source();
    let base = "u8(u8) identity = fn(value) { value };";
    let declarations = format!(
        "{base}\nproof bounded for identity(input) -> (out) ensures (out <= 255) {{}}\nproof too_small for identity(input) -> (out) ensures (out <= 10) {{}}\nproof restricted for identity(input) -> (out) where (input <= 10) ensures (out <= 10) {{}}",
    );
    let proof = |name: &str, text: &str| {
        check_scalar_module_proof(
            identity.clone(),
            name,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!("%%start\n{text}\n%%end"),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(proof("bounded", &declarations), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("too_small", &declarations), ScalarModuleProofOutcome::Refuted);
    assert_eq!(proof("restricted", &declarations), ScalarModuleProofOutcome::Proved);
    assert_eq!(
        proof("bounded", &format!("{declarations}\nu8 real = identity(7);")),
        ScalarModuleProofOutcome::Unsupported,
    );
    assert!(matches!(
        proof("invalid", &format!("{base}\nproof invalid for identity(input) -> (out) ensures (out <= 256) {{}}")),
        ScalarModuleProofOutcome::Invalid(_),
    ));
    let ordinary = parse_source(identity.clone(), format!("%%start\n{base}\n%%end"), &[]);
    assert!(ordinary.diagnostics.is_empty());
    assert!(prepare_scalar_project(
        vec![(derive_scalar_declarations_from_cst(ordinary.result.canonical_cst()), Vec::new())],
        vec![identity.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ).is_ok());
    let ordinary_proof = parse_source(identity, format!("%%start\n{declarations}\n%%end"), &[]);
    assert!(!ordinary_proof.diagnostics.is_empty(), "the ordinary parse/Build entry must remain proof-free");
}

#[test]
fn explicit_proof_module_resolves_named_callable_through_checked_namespace() {
    let app = source();
    let lib = SourceIdentity::new("local".into(), "demo".into(), "src/lib.w".into(), "phase4".into());
    let app_text = "%%start\nlib = namespace demo \"src/lib.w\";\nproof all_paths for lib.choose(input) -> (out) ensures (out <= 8) {}\nproof one_path for lib.choose(input) -> (out) ensures (out <= 7) {}\n%%end";
    let lib_text = "%%start\nu8(u8) choose = fn(input) { if (input == 0) { 7 } else { 8 } };\n%%end";
    let ns_start = app_text.find("lib =").unwrap() as u32;
    let ns_end = app_text.find(';').unwrap() as u32 + 1;
    let proof = |name| check_scalar_module_proof(
        app.clone(),
        name,
        vec![
            ScalarProofModuleInput {
                source: app.clone(),
                text: app_text.into(),
                namespaces: vec![ScalarNamespaceBinding {
                    binding: "lib".into(),
                    target: lib.clone(),
                    span: wosy_syntax::ByteSpan::new(ns_start, ns_end),
                }],
            },
            ScalarProofModuleInput {
                source: lib.clone(),
                text: lib_text.into(),
                namespaces: Vec::new(),
            },
        ],
        vec![app.clone(), lib.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    assert_eq!(proof("all_paths"), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("one_path"), ScalarModuleProofOutcome::Refuted);
}

#[test]
fn explicit_unused_module_proof_where_filters_typed_entry_only() {
    let identity = source();
    let proof = |guard: &str, claim: &str| check_scalar_module_proof(
        identity.clone(),
        "bounded",
        vec![ScalarProofModuleInput {
            source: identity.clone(),
            text: format!(
                "%%start\ni8(i8) identity = fn(value) {{ value }};\nproof bounded for identity(input) -> (out) where ({guard}) ensures ({claim}) {{}}\n%%end",
            ),
            namespaces: Vec::new(),
        }],
        vec![identity.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    assert_eq!(
        proof("input <= -5 && ! (input < -128)", "out <= -5"),
        ScalarModuleProofOutcome::Proved,
    );
    assert_eq!(proof("input <= -5", "out < -5"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(
        proof("input <= -5 && input > -5", "out == 0"),
        ScalarModuleProofOutcome::NoApplicableInputs,
    );
    assert_eq!(proof("false", "out <= 0"), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(
        proof("input == -128 || input == 0", "out <= 0"),
        ScalarModuleProofOutcome::Unsupported,
    );
    assert!(matches!(proof("out <= 0", "out <= 0"), ScalarModuleProofOutcome::Invalid(diagnostics) if !diagnostics.is_empty()));
}

#[test]
fn reached_initializer_cycles_and_depth_fail_closed_when_there_are_obligations() {
    assert_eq!(
        check("unit() cycle = fn { cycle(); }; cycle();"),
        ScalarCheckerOutcome::NoChecks,
    );
    assert_eq!(
        check(&format!("unit() cycle = fn {{ core.require(true, {OUTPUT}); cycle(); }}; cycle();")),
        ScalarCheckerOutcome::Unsupported,
    );
    let chain = |count| {
        let mut text = format!("unit() f0 = fn {{ core.require(true, {OUTPUT}); }};");
        for index in 1..count {
            text.push_str(&format!("\nunit() f{index} = fn {{ f{}(); }};", index - 1));
        }
        text.push_str(&format!("\nf{}();", count - 1));
        text
    };
    assert_eq!(check(&chain(64)), ScalarCheckerOutcome::Passed);
    assert_eq!(check(&chain(65)), ScalarCheckerOutcome::Unsupported);
}

#[test]
fn explicit_module_proof_uses_all_and_only_resolved_actual_call_contexts() {
    let identity = source();
    let proof = |invocations: &str, guard: &str, claim: &str| check_scalar_module_proof(
        identity.clone(),
        "observed",
        vec![ScalarProofModuleInput {
            source: identity.clone(),
            text: format!(
                "%%start\n                u8(u8) project = fn(value) {{ value }};\n                unit(u8) use = fn(input) {{ u8 result = project(input); core.require(result <= 255, {OUTPUT}); }};\n                {invocations}\n                proof observed for project(input) -> (out) {guard} ensures ({claim}) {{}}\n                %%end",
            ),
            namespaces: Vec::new(),
        }],
        vec![identity.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    assert_eq!(proof("use(5); use(7);", "", "out <= 7"), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("", "", "out <= 7"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(proof("use(5); use(7);", "", "out <= 6"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(
        proof("use(5); use(7);", "where (input <= 4)", "out <= 4"),
        ScalarModuleProofOutcome::NoApplicableInputs,
    );
    assert_eq!(
        proof("use(5); use(7);", "where (input <= 6)", "out <= 6"),
        ScalarModuleProofOutcome::Proved,
    );
    assert_eq!(
        proof("use(5); u8 unknown = project(1);", "", "out <= 5"),
        ScalarModuleProofOutcome::Unsupported,
    );
}
