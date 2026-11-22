#![cfg(feature = "internal-checker")]

use wosy_compiler::{
    check_loaded_scalar_module_proof_call, check_scalar_module_proof, check_scalar_module_proofs, check_scalar_package, check_scalar_package_with_dormant_sources, check_scalar_package_ordered_with_dormant_sources, derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project,
    ScalarCheckerOperation, ScalarCheckerOutcome, ScalarCheckerScope, ScalarOrderedCheckerOutcome, ScalarNamespaceBinding,
    ScalarModuleProofOutcome, ScalarProofCallOutcome, ScalarPreparationInputs, ScalarProofModuleInput, ScalarTargetLayout,
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

fn check_ordered(
    identity: &SourceIdentity, text: &str, dormant: Vec<(SourceIdentity, &str)>, scope: ScalarCheckerScope,
) -> ScalarOrderedCheckerOutcome {
    let parse = |source: &SourceIdentity, text: &str| {
        let parsed = parse_source(source.clone(), text.to_owned(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    };
    check_scalar_package_ordered_with_dormant_sources(
        vec![(parse(identity, text), Vec::new())], vec![identity.clone()],
        dormant.into_iter().map(|(identity, text)| parse(&identity, text)).collect(),
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 }, scope,
    )
}

#[test]
fn ordered_checker_facade_keeps_authored_reports_codes_bytes_and_typed_witnesses() {
    let identity = source();
    for codes in [[0, 23, 9], [0, 0, 0]] {
        let [first, second, third] = codes;
        let text = format!(r#"%%start
            unit(u8) unused = fn(value) {{
                core.require(value < 4, {{ .stdout = "third"; .stderr = ""; .code = {third}; .files = []; }});
            }};
            unit(u8) actual = fn(value) {{
                core.require(value < 2, {{ .stdout = ""; .stderr = "second"; .code = {second}; .files = []; }});
            }};
            core.require(false, {{ .stdout = "first"; .stderr = ""; .code = {first};
                .files = [{{ .path = "out.txt"; .text = "ação\n"; }}]; }});
            actual(3);
        %%end"#);
        let ScalarOrderedCheckerOutcome::Failed(reports) = check_ordered(
            &identity, &text, Vec::new(), ScalarCheckerScope::CurrentPackage,
        ) else { panic!("expected complete reports even when every code is zero"); };
        let [root, actual, unused] = reports.as_slice() else { panic!("expected exactly three reports"); };
        assert!(reports.iter().all(|report| report.source == identity && report.operation == ScalarCheckerOperation::Require));
        assert_eq!(reports.iter().map(|report| report.code().clone()).collect::<Vec<_>>(), codes.map(num_bigint::BigInt::from));
        assert_eq!((root.stdout(), actual.stderr(), unused.stdout()), (&b"first"[..], &b"second"[..], &b"third"[..]));
        let at = |span: wosy_syntax::ByteSpan| &text[span.start as usize..span.end as usize];
        assert_eq!((at(root.condition), at(actual.condition), at(unused.condition)), ("false", "value < 2", "value < 4"));
        assert_eq!(at(actual.code_span()), second.to_string());
        assert_eq!(at(actual.stderr_span()), r#""second""#);
        let [file] = root.files() else { panic!("expected only the authored file"); };
        assert_eq!(&*file.path, "out.txt");
        assert_eq!(&*file.text, "ação\n".as_bytes());
        assert_eq!(at(file.path_span), r#""out.txt""#);
        assert!(root.witness.axes().next().is_none());
        assert_eq!(actual.witness.axes().next().unwrap().1, &wosy_compiler::ScalarType::U8);
        for (report, lo, hi) in [(actual, 3, 3), (unused, 4, 255)] {
            assert_eq!(report.witness.row_count(), 1);
            assert_eq!(report.witness.intervals_at(0, 0).unwrap().map(|(lo, hi)| (lo.clone(), hi.clone())).collect::<Vec<_>>(),
                [(num_bigint::BigInt::from(lo), num_bigint::BigInt::from(hi))]);
        }
    }
}

#[test]
fn ordered_checker_facade_keeps_validation_and_distinguishes_incomplete_results() {
    let identity = source();
    let scope = ScalarCheckerScope::CurrentPackage;
    let no_checks = "%%start\nu8 value = 1;\n%%end";
    assert_eq!(check_ordered(&identity, no_checks, Vec::new(), scope), ScalarOrderedCheckerOutcome::NoChecks);
    assert_eq!(check_ordered(&identity, &format!("%%start\ncore.require(true, {OUTPUT});\n%%end"), Vec::new(), scope),
        ScalarOrderedCheckerOutcome::Passed);
    assert!(matches!(check_ordered(&identity, "%%start\ncore.require(true, {});\n%%end", Vec::new(), scope),
        ScalarOrderedCheckerOutcome::Invalid(_)));
    let unsupported = format!(r#"%%start
        unit() leaf = fn {{ leaf(); }};
        unit() parent = fn {{ core.require(false, {OUTPUT}); leaf(); }};
        core.require(false, {OUTPUT});
        parent();
    %%end"#);
    assert_eq!(check_ordered(&identity, &unsupported, Vec::new(), scope), ScalarOrderedCheckerOutcome::Unsupported);
    assert_eq!(check_ordered(&identity, no_checks, vec![(identity.clone(), no_checks)], scope),
        ScalarOrderedCheckerOutcome::Unsupported);
    assert_eq!(check_ordered(&identity, no_checks, vec![(identity.clone(), no_checks)], ScalarCheckerScope::MainFlowOnly),
        ScalarOrderedCheckerOutcome::NoChecks);
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
        ScalarModuleProofOutcome::Proved,
    );
    assert!(matches!(
        proof("invalid", &format!("{base}\nproof invalid for identity(input) -> (out) ensures (out <= 256) {{}}")),
        ScalarModuleProofOutcome::Invalid(_),
    ));
    assert!(matches!(
        proof("bounded", &format!("{base}\nproof bounded for identity(input) -> (out) ensures (out <= 255) {{ core.show(out == 1__0); }}")),
        ScalarModuleProofOutcome::Invalid(diagnostics) if diagnostics.iter().any(|diagnostic| diagnostic.code == "B0010"),
    ));
    let ordinary = parse_source(identity.clone(), format!("%%start\n{base}\n%%end"), &[]);
    assert!(ordinary.diagnostics.is_empty());
    assert!(prepare_scalar_project(
        vec![(derive_scalar_declarations_from_cst(ordinary.result.canonical_cst()), Vec::new())],
        vec![identity.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ).is_ok());
    let ordinary_proof = parse_source(identity.clone(), format!("%%start\n{declarations}\n%%end"), &[]);
    assert!(ordinary_proof.diagnostics.is_empty(), "ordinary parsing recognizes inert module proof declarations");
    assert!(prepare_scalar_project(
        vec![(derive_scalar_declarations_from_cst(ordinary_proof.result.canonical_cst()), Vec::new())],
        vec![identity],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ).is_ok());
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
        ScalarModuleProofOutcome::Proved,
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
    assert_eq!(proof("u8 first = project(5); u8 second = project(7);", "", "out <= 7"), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("u8 first = project(5); u8 second = project(7);", "", "out <= 6"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(
        proof("u8 first = project(5); u8 second = project(first);", "", "out <= 5"),
        ScalarModuleProofOutcome::Proved,
    );
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
        proof("use(5); u8 unknown = project(1); unknown = 2;", "", "out <= 5"),
        ScalarModuleProofOutcome::Unsupported,
    );
}

#[test]
fn explicit_unused_proof_checks_show_claims_against_each_return_partition() {
    let identity = source();
    let proof = |steps: &str, invocation: &str, goal: u8| check_scalar_module_proof(
        identity.clone(),
        "bounded",
        vec![ScalarProofModuleInput {
            source: identity.clone(),
            text: format!(
                "%%start\nu8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};\n                unit() start = fn {{ u8 r = clamp(7); core.require(r <= 10, {OUTPUT}); }};\n                {invocation}\n                proof bounded for clamp(input) -> (out) ensures (out <= {goal}) {{ {steps} }}\n%%end",
            ),
            namespaces: Vec::new(),
        }],
        vec![identity.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    let valid = "if (input > 10) { core.show(out == 10); } else { core.show(out == input); }";
    assert_eq!(proof(valid, "", 10), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof(&valid.replace("> 10", "> 11"), "", 10), ScalarModuleProofOutcome::Refuted);
    assert_eq!(proof(&valid.replace("out == 10", "out == 9"), "", 10), ScalarModuleProofOutcome::Refuted);
    assert_eq!(
        proof(&valid.replace("core.show(out == input);", "core.show(out == input); core.show(out == input);"), "", 10),
        ScalarModuleProofOutcome::Unsupported,
    );
    assert_eq!(proof(valid, "start();", 10), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(proof(valid, "", 9), ScalarModuleProofOutcome::Refuted);
}

#[test]
fn main_initializer_installs_pure_scalar_call_results_from_prepared_bindings() {
    let base = "i8(i8) bounded = fn(value) { if (value < 0) { value } else { 0 } };";
    let text = format!("{base} i8 result = bounded(-128); core.require(result == -128, {OUTPUT});");
    assert_eq!(check_scoped(&text, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);
    let text = format!("{base} i8 result = bounded(10); core.narrow(result == 10, {OUTPUT});");
    assert!(matches!(
        check(&text),
        ScalarCheckerOutcome::Failed(failure) if failure.operation == ScalarCheckerOperation::Narrow,
    ));
    let text = format!("{base} i8 argument = -128; i8 first = bounded(argument); i8 result = bounded(first); core.require(result == -128, {OUTPUT});");
    assert_eq!(check(&text), ScalarCheckerOutcome::Passed);
    let text = format!("{base} i8 argument = -128; argument = 0; i8 result = bounded(argument); core.require(result == -128, {OUTPUT});");
    assert_eq!(check(&text), ScalarCheckerOutcome::Unsupported);
    let text = format!("{base} u8 argument = 128; i8 result = bounded(argument); core.require(result == -128, {OUTPUT});");
    assert!(matches!(check(&text), ScalarCheckerOutcome::Invalid(_)));
}

#[test]
fn named_proof_batch_preserves_order_applicability_and_independent_outcomes() {
    let identity = source();
    let checks = |proof_names: &[&str], tail: &str| check_scalar_module_proofs(
        identity.clone(),
        proof_names,
        vec![ScalarProofModuleInput {
            source: identity.clone(),
            text: format!(r#"%%start
                u8(u8) called = fn(input) {{ input }};
                u8(u8) unused = fn(input) {{ input }};
                u8 actual = called(7);
                {tail}
                proof valid for called(input) -> (out) ensures (out <= 7) {{}}
                proof falsified for called(input) -> (out) ensures (out < 7) {{}}
                proof excluded for called(input) -> (out) where (input < 7) ensures (out < 7) {{}}
                proof invalid for called(input) -> (out) ensures (out <= 256) {{}}
                proof every for unused(input) -> (out) ensures (out <= 255) {{}}
                proof used_show for called(input) -> (out) ensures (out <= 7) {{
                    if (input == 7) {{ core.show(out == 7); }} else {{ core.show(out == input); }}
                }}
            %%end"#),
            namespaces: Vec::new(),
        }],
        vec![identity.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    let names = ["excluded", "valid", "falsified", "used_show", "missing", "every", "invalid", "valid"];
    assert_eq!(checks(&names, "").unwrap(), [
        ScalarModuleProofOutcome::NoApplicableInputs,
        ScalarModuleProofOutcome::Proved,
        ScalarModuleProofOutcome::Refuted,
        ScalarModuleProofOutcome::Unsupported,
        ScalarModuleProofOutcome::Unsupported,
        ScalarModuleProofOutcome::Proved,
        ScalarModuleProofOutcome::Invalid(Vec::new()),
        ScalarModuleProofOutcome::Proved,
    ]);
    assert_eq!(checks(&["valid", "invalid", "every"], "bool changed = true; changed = false;").unwrap(), [
        ScalarModuleProofOutcome::Unsupported,
        ScalarModuleProofOutcome::Invalid(Vec::new()),
        ScalarModuleProofOutcome::Unsupported,
    ]);
    assert_eq!(check_scalar_module_proofs(
        identity,
        &[],
        Vec::new(),
        Vec::new(),
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), Ok(Vec::new()));
}

#[test]
fn verified_module_bound_is_used_only_for_the_selected_canonical_call_and_all_guards() {
    let identity = source();
    let text = |initialization: &str| format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        u8(u8) other = fn(value) {{ value }};
        unit() accepted = fn {{
            u8 input = 7;
            u8 value = producer(input);
            core.require(value <= 10, {OUTPUT});
        }};
        unit() insufficient = fn {{
            u8 value = producer(7);
            core.require(value < 7, {OUTPUT});
        }};
        unit() excluded = fn {{
            u8 value = producer(3);
            core.require(value <= 10, {OUTPUT});
        }};
        unit(u8) partial = fn(input) {{
            u8 value = producer(input);
            core.require(value <= 10, {OUTPUT});
        }};
        unit() wrong_target = fn {{
            u8 value = other(7);
            core.require(value <= 10, {OUTPUT});
        }};
        unit() extra = fn {{
            u8 value = producer(7);
            core.require(value <= 10, {OUTPUT});
            core.require(true, {OUTPUT});
        }};
        proof cap for producer(input) -> (out) where (input > 4 && input <= 10) ensures (out <= 10) {{}}
        proof false_claim for producer(input) -> (out) where (input <= 10) ensures (out < 10) {{}}
        proof empty for producer(input) -> (out) where (false) ensures (out <= 10) {{}}
        proof steps for producer(input) -> (out) where (input <= 10) ensures (out <= 10) {{
            if (input > 10) {{ core.show(out == 10); }} else {{ core.show(out == input); }}
        }}
        proof standalone for producer(input) -> (out) where (input <= 10) ensures (out <= 10) {{
            core.show(out == input);
        }}
        {initialization}
    %%end"#);
    let use_bound = |proof: &str, caller: &str, initialization: &str| {
        let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text(initialization), &[])
            .ok().expect("valid proof syntax");
        check_loaded_scalar_module_proof_call(
            (&identity, proof), (&identity, caller), Vec::new(), (catalog, Vec::new()),
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(use_bound("cap", "accepted", ""), ScalarProofCallOutcome::Established);
    assert_eq!(use_bound("cap", "insufficient", ""), ScalarProofCallOutcome::NotEstablished);
    for (proof, caller, initialization) in [
        ("cap", "excluded", ""),
        ("cap", "partial", ""),
        ("cap", "wrong_target", ""),
        ("cap", "extra", ""),
        ("cap", "absent", ""),
        ("absent", "accepted", ""),
        ("false_claim", "accepted", ""),
        ("empty", "accepted", ""),
        ("steps", "accepted", ""),
        ("standalone", "accepted", ""),
        ("cap", "accepted", "u8 reached = producer(7);"),
        ("cap", "accepted", "accepted();"),
    ] {
        assert_eq!(use_bound(proof, caller, initialization), ScalarProofCallOutcome::Unavailable, "{proof}/{caller}/{initialization}");
    }
}

#[test]
fn verified_module_bound_cannot_be_issued_for_an_unvisited_proof_source() {
    let app = source();
    let hidden = SourceIdentity::new("local".into(), "demo".into(), "src/disconnected.w".into(), "phase4".into());
    let base = parse_source(app.clone(), "%%start\nbool unused = false;\n%%end".into(), &[]);
    assert!(base.diagnostics.is_empty());
    let (_, proof) = wosy_compiler::parse_proof_source(
        hidden.clone(),
        format!(r#"%%start
            u8(u8) producer = fn(input) {{ input }};
            unit() caller = fn {{
                u8 value = producer(7);
                core.require(value <= 10, {OUTPUT});
            }};
            proof cap for producer(input) -> (out) where (input <= 10) ensures (out <= 10) {{}}
        %%end"#),
        &[],
    ).ok().expect("valid disconnected proof syntax");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&hidden, "cap"), (&hidden, "caller"),
        vec![(derive_scalar_declarations_from_cst(base.result.canonical_cst()), Vec::new())],
        (proof, Vec::new()), vec![app, hidden.clone()],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn main_initializer_follows_only_the_selected_unit_if_else_arm() {
    let text = format!(
        "bool enabled = true; if (enabled) {{ core.require(true, {OUTPUT}); }} else {{ enabled = false; }}; core.require(enabled, {OUTPUT});",
    );
    assert_eq!(check_scoped(&text, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);

    let text = format!(
        "if (false) {{ core.narrow(false, {OUTPUT}); }} else {{ core.require(false, {OUTPUT}); }}; core.narrow(false, {OUTPUT});",
    );
    let ScalarCheckerOutcome::Failed(failure) = check(&text) else {
        panic!("expected the first failure in the selected else arm");
    };
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    let canonical = format!("%%start\n{text}\n%%end");
    assert_eq!(failure.condition.start as usize, canonical.find("core.require(false").unwrap() + "core.require(".len());

    let text = format!(
        "bool enabled = true; if (false) {{ core.require(true, {OUTPUT}); }} else {{ enabled = false; }}; core.require(enabled, {OUTPUT});",
    );
    assert_eq!(check(&text), ScalarCheckerOutcome::Unsupported);

    let text = format!(
        "if (true) {{ 1 }} else {{ 2 }}; core.require(true, {OUTPUT});",
    );
    assert!(matches!(check(&text), ScalarCheckerOutcome::Unsupported | ScalarCheckerOutcome::Invalid(_)));
}

#[test]
fn standalone_show_is_a_verified_equality_in_the_selected_unused_entry_domain() {
    let identity = source();
    let proof = |target: &str, guard: &str, goal: &str, show: &str, invocation: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "standalone",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) clamp = fn(value) {{ if (value > 10) {{ 10 }} else {{ value }} }};
                    u8(u8) fixed = fn(value) {{ if (value == 0) {{}}; 7 }};
                    proof standalone for {target}(input) -> (out) where ({guard}) ensures ({goal}) {{
                        core.show(out == {show});
                    }}
                    {invocation}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(proof("clamp", "input <= 10", "out <= 10", "input", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("clamp", "input <= 11", "out <= 10", "input", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(proof("clamp", "false", "out <= 10", "input", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(proof("fixed", "input >= 0", "out <= 7", "7", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("fixed", "input >= 0", "out <= 7", "8", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(proof("fixed", "input >= 0", "out <= 6", "7", ""), ScalarModuleProofOutcome::Refuted);
    assert!(matches!(proof("fixed", "input >= 0", "out <= 7", "256", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(proof("clamp", "input <= 10", "out <= 10", "input + 0", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(
        proof("clamp", "input <= 10", "out <= 10", "input", "u8 started = clamp(7);"),
        ScalarModuleProofOutcome::Unsupported,
    );
}

#[test]
fn named_proof_show_disjunction_checks_the_entire_return_domain() {
    let identity = source();
    let check = |target: &str, guard: &str, ensures: &str, show: &str, startup: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    i16(i16) branches = fn(value) {{ if (value < 0) {{ value }} else {{ value }} }};
                    {startup}
                    proof demonstrate for {target}(input) -> (out) where ({guard}) ensures ({ensures}) {{
                        core.show({show});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let nonzero = "out < 0 || out > 0";
    for target in ["echo", "branches"] {
        assert_eq!(check(target, "input != 0", "out <= 32767", nonzero, ""), ScalarModuleProofOutcome::Proved);
        assert_eq!(check(target, "input >= -8 && input <= 8", "out <= 32767", nonzero, ""), ScalarModuleProofOutcome::Refuted);
    }
    assert_eq!(check("echo", "input != 0", "out < 0", nonzero, ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", "false", "out <= 32767", nonzero, ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("echo", "input != 0", "out <= 32767", "input < 0 || out > 0", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("echo", "input != 0", "out <= 32767", "out < 0 || out > 32768", ""),
        ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("echo", "input != 0", "out <= 32767", nonzero, "i16 live = echo(7);"),
        ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_show_relation_checks_the_typed_bound_without_replacing_ensures() {
    let identity = source();
    let check = |guard: &str, ensures: &str, claim: &str, startup: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    {startup}
                    proof demonstrate for echo(input) -> (out) where ({guard}) ensures ({ensures}) {{
                        core.show({claim});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check("input >= -8 && input < 0", "out < 0", "out >= -8", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input >= -8 && input < 0", "out < 0", "out > -8", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("input == 5", "out >= 5", "out <= 5", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input == 5", "out >= 5", "out > 5", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("input != 0", "out <= 32767", "out != 0", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input != 0", "out != 0", "out != 0", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input == 5", "out < 5", "out <= 5", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("false", "out >= 5", "out <= 5", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("input == 5", "out >= 5", "input <= 5", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("input == 5", "out >= 5", "out < 32768", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("input == 5", "out >= 5", "out <= 5", "i16 actual = echo(5);"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_show_conjunction_normalizes_exact_bounds_and_preserves_holes() {
    let identity = source();
    let check = |guard: &str, ensures: &str, claim: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof demonstrate for echo(input) -> (out) where ({guard}) ensures ({ensures}) {{
                        core.show({claim});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let disconnected = "input >= -8 && input <= 8 && input != 0";
    let continuous = "input >= -8 && input <= 8";
    let repeated = "out <= 8 && out >= -8 && out != 0 && out <= 8 && out >= -8";
    assert_eq!(check(disconnected, "out <= 8", repeated), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(continuous, "out <= 8", repeated), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(continuous, "out <= 8", "out <= 8 && out <= 8"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(continuous, "out < 8", "out <= 8 && out <= 8"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(continuous, "out <= 8", "out >= -8 && out < -8"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("false", "out <= 8", "out >= -8 && out < -8"), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check(disconnected, "out <= 8", "out >= -8 && (out < 0 || out > 0)"), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check(disconnected, "out <= 8", "out >= -8 && out <= 32768"), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_proof_show_old_checks_immutable_entry_and_independent_output_bound() {
    let identity = source();
    let check = |target: &str, guard: &str, ensures: &str, claim: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) echo = fn(tag, value) {{ if (tag == 0) {{}}; value }};
                    i16(i16, i16) fixed = fn(tag, value) {{ if (tag == 0) {{}}; if (value == 0) {{}}; 7 }};
                    i16(u8, i16) width = fn(tag, value) {{ if (tag == 0) {{}}; value }};
                    i16(i16, i16) literal_then_alias = fn(tag, value) {{
                        if (value == 0) {{}}; if (tag == 0) {{ 7 }} else {{ i16 local = 3; local }}
                    }};
                    i16(i16, i16) alias_then_literal = fn(tag, value) {{
                        if (value == 0) {{}}; if (tag == 0) {{ i16 local = 3; local }} else {{ 7 }}
                    }};
                    i16(i16, i16) changed = fn(tag, value) {{ if (tag == 0) {{}}; if (value == 0) {{}}; value = 7; value }};
                    {actual}
                    proof demonstrate for {target}(tag, original) -> (out) where ({guard}) ensures ({ensures}) {{
                        core.show({claim});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let compare = "out >= core.old(original)";
    let strict = "out > core.old(original)";
    assert_eq!(check("echo", "original <= 7", "out <= 7", compare, ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", "original <= 7", "out <= 7", strict, ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed", "original <= 6", "out <= 7", strict, ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed", "original <= 7", "out <= 7", strict, ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed", "original <= 6", "out < 7", strict, ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed", "false", "out <= 7", strict, ""), ScalarModuleProofOutcome::NoApplicableInputs);
    for target in ["literal_then_alias", "alias_then_literal"] {
        assert_eq!(check(target, "original == 7", "out < 7", "out != core.old(original)", ""), ScalarModuleProofOutcome::Unsupported);
    }
    assert_eq!(check("changed", "original <= 7", "out <= 7", compare, ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("width", "original <= 7", "out <= 7", "out >= core.old(tag)", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("echo", "true", "out <= 7", "out >= core.old(null)", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("echo", "original <= 7", "out <= 7", compare, "i16 live = echo(0, 7);"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_show_old_orders_same_width_formals_without_crossing_return_rows() {
    let identity = source();
    let check = |target: &str, where_guard: &str, ensures: &str, claim: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) echo = fn(value, limit) {{ if (limit == 0) {{}}; value }};
                    i16(i16, i16) correlated = fn(value, limit) {{
                        if (limit < 20) {{
                            if (value < 10) {{ value }} else {{ 0 }}
                        }} else {{
                            if (value < 20) {{ value }} else {{ 0 }}
                        }}
                    }};
                    proof demonstrate for {target}(value, limit) -> (out) where ({where_guard}) ensures ({ensures}) {{
                        core.show({claim});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let disconnected = "value >= -2 && value <= 0 && value != -1";
    let strict = "out < core.old(limit)";
    assert_eq!(check("echo", &format!("{disconnected} && limit >= 1"), "out <= 0", strict), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", &format!("{disconnected} && limit >= 0"), "out <= 0", strict), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", &format!("{disconnected} && limit >= 0"), "out <= 0", "out <= core.old(limit)"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", &format!("{disconnected} && limit <= -2"), "out <= 0", "out >= core.old(limit)"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", &format!("{disconnected} && limit >= 1"), "out < 0", strict), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("correlated", "limit >= 10 && limit <= 30", "out < 20", strict), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", "false", "out < 0", strict), ScalarModuleProofOutcome::NoApplicableInputs);
    let equal = "out == core.old(limit)";
    let distinct = "out != core.old(limit)";
    assert_eq!(check("echo", "value == 0 && limit == 0", "out <= 0", equal), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", "value == 0 && limit == 1", "out <= 0", equal), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", "value >= 0 && value <= 1 && limit >= 0 && limit <= 1", "out <= 1", equal), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", &format!("{disconnected} && limit == -1"), "out <= 0", distinct), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", &format!("{disconnected} && limit >= -1 && limit <= 0"), "out <= 0", distinct), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", "value >= 0 && value <= 2 && value != 1 && limit >= 1 && limit <= 3 && limit != 2",
        "out <= 2", distinct), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", "value >= 0 && value <= 2 && value != 1 && limit >= 1 && limit <= 3 && limit != 2",
        "out < 2", distinct), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("correlated", "limit >= 10 && limit <= 30", "out < 20", distinct), ScalarModuleProofOutcome::Proved);
}

#[test]
fn named_proof_ensures_compares_exact_immutable_formals_in_same_row() {
    let identity = source();
    let check = |target: &str, where_guard: &str, operator: &str| {
        check_scalar_module_proof(
            identity.clone(), "verify",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) echo = fn(value, limit) {{ if (limit == 0) {{}}; value }};
                    i16(i16, i16) ordered_rows = fn(value, limit) {{
                        if (limit < 20) {{
                            if (value < 10) {{ value }} else {{ 0 }}
                        }} else {{
                            if (value < 20) {{ value }} else {{ 0 }}
                        }}
                    }};
                    i16(i16, i16) const_first = fn(value, limit) {{
                        if (value == 0) {{}}; if (limit == 0) {{ 7 }} else {{ i16 local = 3; local }}
                    }};
                    i16(i16, i16) local_first = fn(value, limit) {{
                        if (value == 0) {{}}; if (limit == 0) {{ i16 local = 3; local }} else {{ 7 }}
                    }};
                    proof verify for {target}(value, limit) -> (out) where ({where_guard})
                        ensures (out {operator} core.old(limit)) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Refuted, Unsupported};
    let ascending = "value >= -2 && value <= 0 && value != -1 && limit == 1";
    assert_eq!(check("echo", ascending, "<"), Proved);
    assert_eq!(check("echo", "value == 1 && limit == 1", "<="), Proved);
    assert_eq!(check("echo", ascending, ">"), Refuted);
    assert_eq!(check("echo", "value == 2 && limit == 2", ">="), Proved);
    assert_eq!(check("echo", "value >= 0 && value <= 1 && limit >= 0 && limit <= 1", "=="), Refuted);
    assert_eq!(check("echo", "value == 1 && limit == 1", "=="), Proved);
    assert_eq!(check("echo", "value >= -2 && value <= 0 && value != -1 && limit == -1", "!="), Proved);
    assert_eq!(check("echo", "value >= -2 && value <= 0 && value != -1 && limit == 0", "!="), Refuted);
    assert_eq!(check("ordered_rows", "limit >= 10 && limit <= 30", "<"), Proved);
    assert_eq!(check("ordered_rows", "limit >= 10 && limit <= 30", "!="), Proved);
    assert_eq!(check("echo", "false", "<"), NoApplicableInputs);
    for target in ["const_first", "local_first"] {
        assert_eq!(check(target, "limit >= 0 && limit <= 1", "<"), Unsupported);
    }
}

#[test]
fn named_proof_ensures_mixes_exact_literal_and_immutable_entry_conjuncts() {
    let identity = source();
    let check = |target: &str, guard: &str, ensures: &str, startup: &str| {
        check_scalar_module_proof(
            identity.clone(), "verify",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) echo = fn(value, limit) {{ if (limit == 0) {{}}; value }};
                    i16(i16, i16) const_first = fn(value, limit) {{
                        if (value == 0) {{}}; if (limit == 0) {{ 7 }} else {{ i16 local = 3; local }}
                    }};
                    i16(i16, i16) local_first = fn(value, limit) {{
                        if (value == 0) {{}}; if (limit == 0) {{ i16 local = 3; local }} else {{ 7 }}
                    }};
                    {startup}
                    proof verify for {target}(value, limit) -> (out) where ({guard}) ensures ({ensures}) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let scoped = "value >= -2 && value <= 0 && value != -1 && limit >= 1";
    let valid = "out >= -2 && out <= core.old(limit) && out != -1 && out != core.old(limit) && out == core.old(value)";
    assert_eq!(check("echo", scoped, valid, ""), Proved);
    assert_eq!(check("echo", scoped, "out >= -2 && out < 0 && out != core.old(limit)", ""), Refuted);
    assert_eq!(check("echo", scoped, "out >= -2 && out > core.old(limit) && out <= core.old(limit)", ""), Refuted);
    let contradiction = "out < 0 && out >= 0 && out != core.old(limit)";
    assert_eq!(check("echo", "false", contradiction, ""), NoApplicableInputs);
    assert_eq!(check("echo", scoped, contradiction, ""), Refuted);
    assert!(matches!(check("echo", scoped, &format!("{contradiction} && out < 32768"), ""), Invalid(_)));
    for target in ["const_first", "local_first"] {
        assert_eq!(check(target, "limit >= 0 && limit <= 1", contradiction, ""), Unsupported);
    }
    assert_eq!(check("echo", scoped, valid, "i16 seen = echo(0, 1);"), Proved);
    assert_eq!(check("echo", scoped, "out >= -2 && out <= 0 && out != -1", ""), Proved);
    assert_eq!(check("echo", scoped, "out <= 0 && out >= 0", ""), Refuted);
}

#[test]
fn named_proof_ensures_mixed_uses_only_exact_actual_invocation_tuples() {
    let identity = source();
    let check = |startup: &str, body: &str, guard: &str, ensures: &str| {
        check_scalar_module_proof(
            identity.clone(), "verify",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(u8, i16) choose = fn(tag, original) {{
                        if (original == 0) {{}};
                        {body}
                    }};
                    {startup}
                    proof verify for choose(tag, entry) -> (out) where ({guard}) ensures ({ensures}) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Refuted, Unsupported};
    let body = "if (tag == 0) { 7 } else { original }";
    let guard = "tag <= 2";
    let goal = "out <= 8 && out <= core.old(entry) && out >= -8";
    assert_eq!(check("i16 first = choose(0, 7); i16 second = choose(1, -3);", body, guard, goal), Proved);
    assert_eq!(check("i16 first = choose(0, 7); i16 bad = choose(0, 6); i16 last = choose(1, 7);",
        body, guard, goal), Refuted);
    assert_eq!(check("i16 bad = choose(0, 6); i16 last = choose(1, 7);", body, guard, goal), Refuted);
    assert_eq!(check("i16 seen = choose(3, 9);", body, guard, goal), NoApplicableInputs);
    assert_eq!(check("i16 seen = choose(1, 9);", body, guard, goal), Refuted);
    assert_eq!(check("i16 seen = choose(0, 7);", "if (tag == 0) {}; 3 + 4", guard, goal), Unsupported);
}

#[test]
fn named_proof_actual_flat_show_checks_only_authenticated_integer_returns() {
    let identity = source();
    let check = |startup: &str, body: &str, guard: &str, ensures: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(), "verify",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i8(i8) echo = fn(value) {{ {body} }};
                    {startup}
                    proof verify for echo(input) -> (out) where ({guard}) ensures ({ensures}) {{ {steps} }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let guard = "input >= 0";
    let bound = "out <= 8";
    let shows = "core.show(out >= 0); core.show(out < 8);";
    let one = "i8 seen = echo(7);";
    assert_eq!(check(one, "value", guard, bound, shows), Proved);
    assert_eq!(check(one, "value", guard, "out < 7", shows), Refuted);
    assert_eq!(check(one, "value", guard, "out <= 7", "core.show(out == 7); core.show(out == 7);"), Proved);
    assert_eq!(check(one, "value", guard, bound, "core.show(out == 7); core.show(out == 6);"), Refuted);
    assert_eq!(check(one, "value", guard, bound, "core.show(out == 7); core.show(out == 6); core.show(out == input);"), Unsupported);
    assert_eq!(check(one, "value", guard, bound, "core.show(out <= 6);"), Refuted);
    assert_eq!(check("i8 first = echo(8); i8 last = echo(7);", "value", guard, bound, shows), Refuted);
    assert_eq!(check("i8 miss = echo(-1);", "value", guard, bound, shows), NoApplicableInputs);
    assert!(matches!(check("i8 miss = echo(-1);", "value", guard, bound, "core.show(out <= 128);"), Invalid(_)));
    assert_eq!(check(one, "value", guard, bound, "core.show(out == core.old(input));"), Proved);
    assert_eq!(check(one, "value", guard, bound, "core.show(out == input);"), Unsupported);
    assert_eq!(check(one, "value", guard, bound,
        "if (input >= 0) { core.show(out == 7); } else { core.show(out == 7); }"), Unsupported);
    assert_eq!(check(one, "if (value == 0) {}; 3 + 4", guard, bound, shows), Unsupported);
}

#[test]
fn named_proof_actual_flat_old_shows_check_exact_positional_formals() {
    let identity = source();
    let check = |signature: &str, startup: &str, guard: &str, ensures: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(), "verify",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16({signature}) choose = fn(tag, original) {{
                        if (original == 0) {{}};
                        if (tag == 0) {{ 7 }} else {{ original }}
                    }};
                    {startup}
                    proof verify for choose(flag, entry) -> (out) where ({guard}) ensures ({ensures}) {{ {steps} }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let good = "i16 first = choose(1, -3); i16 second = choose(0, 7);";
    let bad = "i16 bad = choose(0, 6); i16 last = choose(0, 7);";
    let old = "core.show(out <= core.old(entry));";
    assert_eq!(check("u8, i16", good, "true", "out <= 7", old), Proved);
    assert_eq!(check("u8, i16", bad, "true", "out <= 7", old), Refuted);
    assert_eq!(check("u8, i16", good, "true", "out < 7", old), Refuted);
    assert_eq!(check("u8, i16", good, "false", "out <= 7", old), NoApplicableInputs);
    assert_eq!(check("u8, i16", good, "true", "out <= 7", "core.show(out <= core.old(flag));"), Unsupported);
    assert!(matches!(check("u8, i16", good, "false", "out <= 7", "core.show(out <= core.old(unknown));"), Invalid(_)));
    let mixed = "core.show(out <= core.old(entry)); core.show(out >= core.old(entry));
        core.show(out <= core.old(flag)); core.show(out != 0);";
    assert_eq!(check("i16, i16", "i16 one = choose(10, 7); i16 two = choose(8, 6);", "true", "out <= 7", mixed), Proved);
    assert_eq!(check("i16, i16", "i16 one = choose(6, 7); i16 two = choose(8, 6);", "true", "out <= 7", mixed), Refuted);
    assert_eq!(check("i16, i16", "i16 one = choose(10, 7);", "true", "out <= 7",
        "core.show(out == entry); core.show(out <= core.old(flag));"), Unsupported);
}

#[test]
fn named_proof_actual_guarded_show_uses_only_the_checked_actual_arm() {
    let identity = source();
    let check = |startup: &str, where_guard: &str, ensures: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(), "verify",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i8(i8) echo = fn(value) {{ value }};
                    {startup}
                    proof verify for echo(input) -> (out) where ({where_guard}) ensures ({ensures}) {{ {steps} }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let yes = "i8 first = echo(7);";
    let no = "i8 first = echo(6);";
    let both = "i8 first = echo(7); i8 second = echo(6);";
    let literal = "if (input >= 7) { core.show(out <= 7); } else { core.show(out > 7); }";
    assert_eq!(check(yes, "true", "out <= 7", literal), Proved);
    assert_eq!(check(no, "true", "out <= 7", literal), Refuted);
    assert_eq!(check(both, "true", "out <= 7", literal), Refuted);
    assert_eq!(check(yes, "true", "out < 7", literal), Refuted);
    assert_eq!(check(yes, "false", "out <= 7", literal), NoApplicableInputs);
    assert!(matches!(check(yes, "false", "out <= 7", &literal.replace("out > 7", "out > 128")), Invalid(_)));
    let old = "if (input >= 7) { core.show(out == core.old(input)); } else { core.show(out > core.old(input)); }";
    assert_eq!(check(yes, "true", "out <= 7", old), Proved);
    assert_eq!(check(no, "true", "out <= 7", old), Refuted);
    assert_eq!(check(yes, "true", "out <= 7", &old.replace("old(input)", "old(missing)")), Invalid(Vec::new()));
    assert_eq!(check(yes, "true", "out <= 7",
        "if (input >= 7) { core.show(out == 7); } else { core.show(out == 6); }"), Unsupported);
}

#[test]
fn named_proof_show_mixed_sequence_holds_all_exact_obligations() {
    let identity = source();
    let check = |target: &str, where_guard: &str, ensures: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) echo = fn(value, limit) {{ if (limit == 0) {{}}; value }};
                    i16(i16, i16) fixed = fn(value, limit) {{ if (value == 0) {{}}; if (limit == 0) {{}}; 7 }};
                    i16(i16, i16) const_first = fn(value, limit) {{
                        if (value == 0) {{}}; if (limit == 0) {{ 7 }} else {{ i16 local = 3; local }}
                    }};
                    i16(i16, i16) local_first = fn(value, limit) {{
                        if (value == 0) {{}}; if (limit == 0) {{ i16 local = 3; local }} else {{ 7 }}
                    }};
                    proof demonstrate for {target}(value, limit) -> (out) where ({where_guard}) ensures ({ensures}) {{
                        {steps}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let scope = "value >= -2 && value <= 0 && value != -1 && limit >= 1";
    let steps = "core.show(out >= -2); core.show(out != -1); core.show(out != -1);
        core.show(out <= core.old(limit)); core.show(out != core.old(limit)); core.show(out == core.old(value));";
    assert_eq!(check("echo", scope, "out <= 0", steps), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", scope, "out < 0", steps), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", "value >= -2 && value <= 0 && limit >= 1", "out <= 0", steps), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", "value >= -2 && value <= 0 && value != -1 && limit >= 0", "out <= 0", steps),
        ScalarModuleProofOutcome::Refuted);
    let literal_only = "core.show(out == 7); core.show(out >= 7); core.show(out != 6);";
    assert_eq!(check("fixed", "true", "out <= 7", literal_only), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed", "true", "out < 7", literal_only), ScalarModuleProofOutcome::Refuted);
    let low = "core.show(out == 7); core.show(out >= 7); core.show(out <= core.old(limit)); core.show(out >= core.old(value));";
    assert_eq!(check("fixed", "value <= 7 && limit >= 7", "out <= 7", low), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed", "value <= 7 && limit >= 6", "out <= 7", low), ScalarModuleProofOutcome::Refuted);
    let impossible_old = "core.show(out < core.old(limit)); core.show(out >= core.old(limit));";
    assert_eq!(check("echo", scope, "out <= 0", impossible_old), ScalarModuleProofOutcome::Refuted);
    let impossible = "core.show(out < 0); core.show(out >= 0); core.show(out > core.old(limit));";
    assert_eq!(check("echo", "false", "out <= 0", impossible), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("echo", scope, "out <= 0", impossible), ScalarModuleProofOutcome::Refuted);
    assert!(matches!(check("echo", scope, "out <= 0", &format!("{impossible} core.show(out > 32768);")),
        ScalarModuleProofOutcome::Invalid(_)));
    for target in ["const_first", "local_first"] {
        assert_eq!(check(target, "limit >= 0", "out < 6", impossible), ScalarModuleProofOutcome::Unsupported);
    }
}

#[test]
fn named_proof_show_guarded_implications_preserve_exact_integer_paths() {
    let identity = source();
    let check = |target: &str, where_guard: &str, ensures: &str, guard: &str, yes: &str, no: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    i16(i16) fixed = fn(value) {{ if (value == 0) {{}}; 7 }};
                    i16(i16, i16) const_first = fn(tag, original) {{
                        if (original == 0) {{}}; if (tag == 0) {{ 7 }} else {{ tag }}
                    }};
                    i16(i16, i16) alias_first = fn(tag, original) {{
                        if (original == 0) {{}}; if (tag == 0) {{ tag }} else {{ 7 }}
                    }};
                    proof demonstrate for {target} -> (out) where ({where_guard}) ensures ({ensures}) {{
                        if ({guard}) {{ core.show({yes}); }} else {{ core.show({no}); }}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let scope = "value >= -8 && value <= 7";
    assert_eq!(check("echo(value)", scope, "out <= 7", "value > -1", "out >= 0", "out < 0"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo(value)", scope, "out <= 7", "value > -2", "out >= 0", "out < 0"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo(value)", "value != -1", "out <= 32767",
        "value > -2", "out >= 0", "out < 0"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo(value)", scope, "out < 7", "value > -1", "out >= 0", "out < 0"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed(value)", "value <= -1", "out <= 7", "value > -1", "out < 0", "out <= 7"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed(value)", "false", "out <= 7", "value > -1", "out < 0", "out <= 7"), ScalarModuleProofOutcome::NoApplicableInputs);
    for target in ["const_first(tag, value)", "alias_first(tag, value)"] {
        assert_eq!(check(target, "value == 0", "out <= 6", "value > -1", "out < 0", "out <= 7"), ScalarModuleProofOutcome::Unsupported);
    }
    assert!(matches!(check("echo(value)", scope, "out <= 7", "value > -1", "out >= 32768", "out < 0"), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_proof_show_guarded_old_uses_only_the_immutable_guard_axis() {
    let identity = source();
    let check = |target: &str, where_guard: &str, ensures: &str, guard: &str, yes: &str, no: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    i16(i16) fixed = fn(value) {{ if (value == 0) {{}}; 7 }};
                    i16(i16, i16) const_first = fn(tag, original) {{
                        if (original == 0) {{}}; if (tag == 0) {{ 7 }} else {{ tag }}
                    }};
                    i16(i16, i16) alias_first = fn(tag, original) {{
                        if (original == 0) {{}}; if (tag == 0) {{ tag }} else {{ 7 }}
                    }};
                    proof demonstrate for {target} -> (out) where ({where_guard}) ensures ({ensures}) {{
                        if ({guard}) {{ core.show({yes}); }} else {{ core.show({no}); }}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let equal = "out == core.old(value)";
    let above = "out > core.old(value)";
    let at_most = "out <= core.old(value)";
    let scope = "value >= -8 && value <= 7";
    assert_eq!(check("echo(value)", scope, "out <= 7", "value > 7", above, equal), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo(value)", scope, "out <= 7", "value > 6", above, equal), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo(value)", scope, "out < 7", "value > 7", above, equal), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed(value)", "true", "out <= 7", "value >= 7", at_most, above), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed(value)", "true", "out <= 7", "value >= 8", at_most, above), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed(value)", "value != 7", "out <= 7", "value >= 8", at_most, above), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed(value)", "false", "out <= 7", "value >= 8", at_most, above), ScalarModuleProofOutcome::NoApplicableInputs);
    for target in ["const_first(tag, value)", "alias_first(tag, value)"] {
        assert_eq!(check(target, "value == 0", "out <= 6", "value > -1", equal, above), ScalarModuleProofOutcome::Unsupported);
        assert_eq!(check(target, "tag == 0", "out <= 7", "tag == 0", equal, above), ScalarModuleProofOutcome::Unsupported);
    }
}

#[test]
fn named_proof_show_mixed_alternatives_preserve_clause_precedence_and_holes() {
    let identity = source();
    let check = |guard: &str, ensures: &str, claim: &str| {
        check_scalar_module_proof(
            identity.clone(), "demonstrate",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof demonstrate for echo(input) -> (out) where ({guard}) ensures ({ensures}) {{
                        core.show({claim});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let split = "input >= 1 && input <= 7 && input != 4";
    let unparenthesized = "out >= 1 && out <= 3 || out >= 5 && out <= 7";
    let parenthesized = "(out >= 1 && out <= 3) || (out >= 5 && out <= 7)";
    for predicate in [unparenthesized, parenthesized] {
        assert_eq!(check(split, "out <= 7", predicate), ScalarModuleProofOutcome::Proved);
        assert_eq!(check("input >= 1 && input <= 7", "out <= 7", predicate), ScalarModuleProofOutcome::Refuted);
        assert_eq!(check(split, "out < 7", predicate), ScalarModuleProofOutcome::Refuted);
        assert_eq!(check("false", "out <= 7", predicate), ScalarModuleProofOutcome::NoApplicableInputs);
    }
    let with_hole = "(out >= 1 && out <= 7 && out != 4) || (out >= 4 && out < 4) || out == -8";
    assert_eq!(check(split, "out <= 7", with_hole), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input == 4", "out <= 7", with_hole), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("input == -8", "out <= 7", with_hole), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(split, "out <= 7", "out >= 1 && (out <= 3 || out >= 5)"), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check(split, "out <= 7", "(out >= 1 && out <= 32768) || out == -8"), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn sequential_named_proof_shows_verify_each_claim_without_replacing_ensures() {
    let identity = source();
    let check = |target: &str, guard: &str, goal: &str, steps: &str, startup: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "sequence",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8, u8) fixed = fn(x, y) {{ if (x == 0) {{}}; if (y == 0) {{}}; 7 }};
                    u8(u8, u8) alias = fn(x, y) {{ if (y == 0) {{}}; x }};
                    {startup}
                    proof sequence for {target}(first, second) -> (out) where ({guard}) ensures ({goal}) {{
                        {steps}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let valid = "core.show(out == first); core.show(out == 7); core.show(out == first);";
    assert_eq!(check("fixed", "first == 7", "out == 7", valid, ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("fixed", "first == 7", "out < 7", valid, ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("fixed", "false", "out == 7", valid, ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("fixed", "first == 7", "out == 7", valid, "u8 observed = fixed(7, 0);"),
        ScalarModuleProofOutcome::Unsupported);
    for failed in ["core.show(out == 8); core.show(out == 7);", "core.show(out == 7); core.show(out == 8);"] {
        assert_eq!(check("fixed", "first == 7", "out == 7", failed, ""), ScalarModuleProofOutcome::Refuted);
    }
    for outside in ["core.show(out == 8); core.show(out == second);", "core.show(out == second); core.show(out == 8);"] {
        assert_eq!(check("alias", "first == 7", "out == 7", outside, ""), ScalarModuleProofOutcome::Unsupported);
    }
    assert!(matches!(check("fixed", "first == 7", "out == 7",
        "core.show(out == 7); core.show(out == 256);", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("fixed", "first == 7", "out == 7",
        "core.show(out == 7); core.witness(7);", ""), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn sequential_named_proof_shows_reject_the_intrinsic_null_as_an_integer_alias() {
    let identity = source();
    let check = |parameters: &str, output: &str, first_claim: &str, second_claim: &str| {
        check_scalar_module_proof(
            identity.clone(), "sequence",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) fixed = fn(value) {{ if (value == 0) {{}}; 7 }};
                    proof sequence for fixed({parameters}) -> ({output}) ensures ({output} == 7) {{
                        core.show({first_claim});
                        core.show({second_claim});
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert!(matches!(check("null", "out", "out == null", "out == 7"), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("entry", "null", "null == 7", "null == 7"), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_proof_peer_use_checks_one_independent_same_domain_leaf() {
    let identity = source();
    let check = |leaf: &str, consumer_guard: &str, goal: &str, use_step: &str, startup: &str, names: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), names,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8, u8) fixed = fn(x, y) {{ if (x == 0) {{}}; if (y == 0) {{}}; 7 }};
                    u8(u8, u8) other = fn(x, y) {{ if (x == 0) {{}}; if (y == 0) {{}}; 7 }};
                    {startup}
                    {leaf}
                    proof consumer for fixed(first, second) -> (out) where ({consumer_guard}) ensures ({goal}) {{
                        {use_step}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    let leaf = "proof leaf for fixed(a, b) -> (value) where (a == 7) ensures (value == 7) {}";
    let good = "core.use(leaf(first, second));";
    let guard = "first == 7";
    let one = |leaf, guard, goal, step, startup| check(leaf, guard, goal, step, startup, &["consumer"]);
    assert_eq!(one(leaf, guard, "out <= 7", good, ""), [ScalarModuleProofOutcome::Proved]);
    assert_eq!(check(leaf, guard, "out <= 7", good, "", &["consumer", "leaf"]), [
        ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved,
    ]);
    assert_eq!(one(leaf, guard, "out < 7", good, ""), [ScalarModuleProofOutcome::Refuted]);
    assert_eq!(one(leaf, "first <= 7", "out <= 7", good, ""), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(one(leaf, guard, "out <= 7", "core.use(leaf(second, first));", ""), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(one(leaf, guard, "out <= 7", "core.use(consumer(first, second));", ""), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(one("", guard, "out <= 7", good, ""), [ScalarModuleProofOutcome::Unsupported]);
    let wrong_target = leaf.replace("for fixed", "for other");
    let false_leaf = leaf.replace("value == 7", "value == 8");
    assert_eq!(one(&wrong_target, guard, "out <= 7", good, ""), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(one(&false_leaf, guard, "out <= 7", good, ""),
        [ScalarModuleProofOutcome::Unsupported]);
    let invalid = leaf.replace("value == 7", "value == 256");
    assert!(matches!(check(&invalid, guard, "out <= 7", good, "", &["consumer", "leaf"]).as_slice(),
        [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Invalid(_)]));
    let cycle = leaf.replace("{}", "{ core.use(consumer(a, b)); }");
    assert_eq!(check(&cycle, guard, "out <= 7", good, "", &["consumer", "leaf"]), [
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Unsupported,
    ]);
    let empty_leaf = leaf.replace("a == 7", "false");
    assert_eq!(one(&empty_leaf, "false", "out <= 7", good, ""), [ScalarModuleProofOutcome::NoApplicableInputs]);
    assert_eq!(one(leaf, guard, "out <= 7", good, "u8 actual = fixed(7, 0);"), [ScalarModuleProofOutcome::Unsupported]);
}

#[test]
fn named_proof_peer_use_consumes_independently_verified_typed_predicates() {
    let identity = source();
    let check = |leaf_guard: &str, root_guard: &str, premise: &str, intermediate: &str, goal: &str, leaf_steps: &str| {
        check_scalar_module_proofs(
            identity.clone(), &["root", "leaf"],
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i8(i8) echo = fn(value) {{ value }};
                    proof leaf for echo(entry) -> (out) where ({leaf_guard}) ensures ({premise}) {{ {leaf_steps} }}
                    proof middle for echo(entry) -> (out) where ({leaf_guard}) ensures ({intermediate}) {{
                        core.use(leaf(entry));
                    }}
                    proof root for echo(input) -> (out) where ({root_guard}) ensures ({goal}) {{
                        core.use(middle(input));
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Refuted, Unsupported};
    assert_eq!(check("entry < 8", "input < 8", "out < 8", "out < 8", "out <= 7", ""), [Proved, Proved]);
    assert_eq!(check("entry < 8", "input < 8", "out < 8", "out < 8", "out < 7", ""), [Unsupported, Proved]);
    assert_eq!(check("entry < 8", "input < 8", "out < 8", "out <= 8", "out <= 8", ""), [Unsupported, Proved]);
    assert_eq!(check("entry < 8", "input < 8", "out < 7", "out < 7", "out <= 7", ""), [Unsupported, Refuted]);
    assert_eq!(check("entry < 8", "input == 6", "out < 8", "out < 8", "out <= 7", ""), [Unsupported, Proved]);
    assert_eq!(check("entry != 0", "input != 0", "out != 0", "out != 0", "out != 0", ""), [Proved, Proved]);
    assert_eq!(check("entry != 0", "input != 0", "out != 0", "out != 0", "out != 1", ""), [Unsupported, Proved]);
    assert_eq!(check("entry != -128", "input != -128", "out != -128", "out != -128", "out >= -127", ""), [Proved, Proved]);
    assert_eq!(check("entry != 127", "input != 127", "out != 127", "out != 127", "out < 127", ""), [Proved, Proved]);
    assert_eq!(check("entry < 8", "input < 8", "out < 8", "out < 8", "out <= 7", "core.use(middle(entry));"),
        [Unsupported, Unsupported]);
    assert_eq!(check("false", "false", "out < 8", "out < 8", "out <= 7", ""), [NoApplicableInputs, NoApplicableInputs]);
}

#[test]
fn named_proof_peer_use_requires_each_authored_show_and_independent_ensures() {
    let identity = source();
    let check = |leaf_guard: &str, consumer_guard: &str, ensures: &str, steps: &str, names: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), names,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i8(i8) echo = fn(value) {{ value }};
                    proof leaf for echo(entry) -> (out) where ({leaf_guard}) ensures ({ensures}) {{
                        {steps}
                    }}
                    proof consumer for echo(arg) -> (out) where ({consumer_guard}) ensures (out <= 7) {{
                        core.use(leaf(arg));
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Refuted, Unsupported};
    let both = ["consumer", "leaf"];
    let good = "core.show(out <= 7); core.show(out == core.old(entry));";
    assert_eq!(check("entry < 8", "arg < 8", "out < 8", good, &both), [Proved, Proved]);
    assert_eq!(check("entry < 8", "arg < 8", "out < 8", good, &["consumer"]), [Proved]);
    assert_eq!(check("entry < 8", "arg < 8", "out < 8", "core.show(out <= 6); core.show(out <= 7);", &both),
        [Unsupported, Refuted]);
    assert_eq!(check("entry < 8", "arg < 8", "out < 7", good, &both), [Unsupported, Refuted]);
    assert_eq!(check("entry < 8", "arg < 8", "out < 8", "core.show(out + 1 <= 8);", &both),
        [Unsupported, Unsupported]);
    assert_eq!(check("false", "false", "out < 8", good, &both), [Unsupported, NoApplicableInputs]);
    assert_eq!(check("entry < 8", "arg == 6", "out < 8", good, &both), [Unsupported, Proved]);
}

#[test]
fn named_proof_peer_use_applies_a_direct_non_equality_leaf_to_a_strict_subset() {
    let identity = source();
    let check = |leaf_guard: &str, caller_guard: &str, premise: &str, goal: &str| {
        check_scalar_module_proofs(
            identity.clone(), &["caller", "leaf"],
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i8(i8) echo = fn(value) {{ value }};
                    proof leaf for echo(entry) -> (out) where ({leaf_guard}) ensures ({premise}) {{}}
                    proof caller for echo(input) -> (out) where ({caller_guard}) ensures ({goal}) {{
                        core.use(leaf(input));
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    use ScalarModuleProofOutcome::{NoApplicableInputs, Proved, Refuted, Unsupported};
    assert_eq!(check("entry < 8", "input == 6", "out < 8", "out <= 7"), [Proved, Proved]);
    assert_eq!(check("entry < 8", "input >= 5 && input <= 6", "out < 8", "out <= 7"), [Proved, Proved]);
    assert_eq!(check("entry < 8", "input == 6", "out < 8", "out == 6"), [Unsupported, Proved]);
    assert_eq!(check("entry < 8", "input == 6", "out <= 6", "out <= 6"), [Unsupported, Refuted]);
    assert_eq!(check("entry < 8", "input == 8", "out < 8", "out <= 7"), [Unsupported, Proved]);
    assert_eq!(check("entry != 5", "input >= 4 && input <= 6", "out != 5", "out != 5"), [Unsupported, Proved]);
    assert_eq!(check("entry < 4 || entry > 5", "input >= 6 && input <= 7", "out != 5", "out != 5"), [Proved, Proved]);
    assert_eq!(check("entry < 4 || entry > 5", "input >= 3 && input <= 6", "out != 5", "out != 5"), [Unsupported, Proved]);
    assert_eq!(check("false", "input == 6", "out < 8", "out <= 7"), [Unsupported, NoApplicableInputs]);
    assert_eq!(check("entry < 8", "input >= 7 && input <= 6", "out < 8", "out <= 7"), [Unsupported, Proved]);
}

#[test]
fn named_proof_peer_use_checks_exact_singleton_where_subsets_without_filling_holes() {
    let identity = source();
    let check = |peer_where: &str, caller_where: &str, goal: &str, peers: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), peers,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) fixed = fn(value) {{ if (value > 7) {{ 7 }} else {{ 7 }} }};
                    proof leaf for fixed(input) -> (out) where ({peer_where}) ensures (out == 7) {{}}
                    proof consumer for fixed(entry) -> (out) where ({caller_where}) ensures ({goal}) {{
                        core.use(leaf(entry));
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    let selections = ["consumer", "leaf"];
    assert_eq!(check("input <= 7", "entry == 6", "out <= 7", &selections), [
        ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved,
    ]);
    assert_eq!(check("input < 3 || input > 5", "entry == 6", "out <= 7", &selections), [
        ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved,
    ]);
    assert_eq!(check("input < 3 || input > 5", "entry == 4", "out <= 7", &selections), [
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved,
    ]);
    assert_eq!(check("input <= 7", "entry == 6", "out < 7", &selections), [
        ScalarModuleProofOutcome::Refuted, ScalarModuleProofOutcome::Proved,
    ]);
    assert_eq!(check("input <= 7", "entry == 6", "out <= 7", &["consumer"]),
        [ScalarModuleProofOutcome::Proved]);
    assert_eq!(check("input < 3 || input > 5", "entry == 4", "out <= 7", &["consumer"]),
        [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(check("input <= 7", "entry <= 6", "out <= 7", &selections), [
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved,
    ]);
}

#[test]
fn named_proof_peer_use_checks_a_closed_interval_in_one_returned_entry_row() {
    let identity = source();
    let check = |leaf_where: &str, caller_where: &str, goal: &str| {
        check_scalar_module_proof(
            identity.clone(), "consumer",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) fixed = fn(value) {{ if (value < 0) {{ 7 }} else {{ 7 }} }};
                    proof leaf for fixed(input) -> (out) where ({leaf_where}) ensures (out == 7) {{}}
                    proof consumer for fixed(entry) -> (out) where ({caller_where}) ensures ({goal}) {{
                        core.use(leaf(entry));
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check("input <= 10", "entry <= -1 && entry >= -4", "out == 7"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input <= 10", "entry >= -4 && entry <= -1", "out < 7"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("input <= 10", "entry >= -4 && entry <= 1", "out == 7"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("input <= 10", "entry >= 3 && entry <= 2", "out == 7"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("input < 3 || input > 5", "entry >= 6 && entry <= 8", "out == 7"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("input < 3 || input > 5", "entry >= 2 && entry <= 6", "out == 7"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("input != 4", "entry >= 3 && entry <= 5", "out == 7"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_peer_use_checks_a_mixed_width_exact_entry_tuple() {
    let identity = source();
    let check = |guard: &str, goal: &str, step: &str| {
        check_scalar_module_proof(
            identity.clone(), "consumer",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(u8, i16) fixed = fn(byte, signed) {{
                        if (byte < 5) {{}};
                        if (signed < 0) {{ 7 }} else {{ 7 }}
                    }};
                    proof leaf for fixed(a, b) -> (out) where (a >= 5 && (b < -3 || b > 3)) ensures (out == 7) {{}}
                    proof consumer for fixed(first, second) -> (out) where ({guard}) ensures ({goal}) {{
                        {step}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let use_leaf = "core.use(leaf(first, second));";
    let good = "second == -8 && first == 6";
    assert_eq!(check(good, "out == 7", use_leaf), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("first == 6 && second == -8", "out < 7", use_leaf), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("first == 4 && second == -8", "out == 7", use_leaf), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("first == 6 && second == 0", "out == 7", use_leaf), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("first == 6 && first == 6", "out == 7", use_leaf), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(good, "out == 7", "core.use(leaf(second, first));"), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("first == 256 && second == -8", "out == 7", use_leaf), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_proof_peer_chain_is_iterative_and_shares_explicit_and_implicit_dependencies() {
    let identity = source();
    let check = |text: &str, names: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), names,
            vec![ScalarProofModuleInput {
                source: identity.clone(), text: format!("%%start\n{text}\n%%end"), namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    let mut chain = String::from(
        "u8(u8) fixed = fn(value) { if (value == 0) {}; 7 };
         proof stage0 for fixed(alias) -> (out) where (alias <= 7) ensures (out == 7) {}
"
    );
    for i in 1..=80 {
        chain.push_str(&format!(
            "proof stage{i} for fixed(input) -> (answer) where (input <= 7) ensures (answer == 7) {{ core.use(stage{}(input)); }}\n",
            i - 1,
        ));
    }
    assert_eq!(check(&chain, &["stage80"]), [ScalarModuleProofOutcome::Proved]);
    assert_eq!(check(&chain, &["stage80", "stage0", "stage40"]), [
        ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved,
    ]);
    let actual = format!("{chain} u8 live = fixed(7);");
    assert_eq!(check(&actual, &["stage80", "stage0", "stage40"]), [
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Unsupported,
    ]);
}

#[test]
fn named_proof_peer_chain_rejects_cyclic_invalid_and_unproved_intermediates() {
    let identity = source();
    let check = |proofs: &str, requested: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), requested,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) fixed = fn(value) {{ if (value == 0) {{}}; 7 }};
                    proof leaf for fixed(input) -> (out) where (input <= 7) ensures (out == 7) {{}}
                    {proofs}
                    proof outer for fixed(entry) -> (out) where (entry <= 7) ensures (out <= 7) {{
                        core.use(middle(entry));
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    let invalid = "proof middle for fixed(x) -> (result) where (x <= 7) ensures (result == 256) { core.use(leaf(x)); }";
    assert!(matches!(check(invalid, &["outer", "middle"]).as_slice(),
        [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Invalid(_)]));
    let false_middle = invalid.replace("result == 256", "result == 8");
    assert_eq!(check(&false_middle, &["outer", "middle", "leaf"]), [
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Refuted, ScalarModuleProofOutcome::Proved,
    ]);
    let cycle = r#"
        proof middle for fixed(x) -> (result) where (x <= 7) ensures (result == 7) { core.use(looped(x)); }
        proof looped for fixed(y) -> (answer) where (y <= 7) ensures (answer == 7) { core.use(middle(y)); }
    "#;
    assert_eq!(check(cycle, &["outer", "middle", "looped", "leaf"]), [
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Unsupported,
        ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Proved,
    ]);
    let unique = invalid.replace("result == 256", "result == 7");
    let duplicate = format!("{unique} {unique}");
    assert_eq!(check(&duplicate, &["outer"]), [ScalarModuleProofOutcome::Unsupported]);
    let mismatched = unique.replace("x <= 7", "x <= 6");
    assert_eq!(check(&mismatched, &["outer"]), [ScalarModuleProofOutcome::Unsupported]);
}

#[test]
fn verified_bound_checks_both_ordered_typed_call_arguments_and_repeated_aliases() {
    let identity = source();
    let use_bound = |proof: &str, caller: &str, extra: &str| {
        let text = format!(r#"%%start
            u16(u8, u16) pair = fn(low, high) {{ if (low == 0) {{}}; high }};
            u8(u8, u8) same = fn(low, high) {{ if (low == 0) {{}}; high }};
            unit() accepted = fn {{ u16 result = pair(7, 3); core.require(result <= 3, {OUTPUT}); }};
            unit() swapped = fn {{ u16 result = pair(3, 7); core.require(result <= 3, {OUTPUT}); }};
            unit() first_hole = fn {{ u16 result = pair(4, 3); core.require(result <= 3, {OUTPUT}); }};
            unit() second_hole = fn {{ u16 result = pair(7, 4); core.require(result <= 3, {OUTPUT}); }};
            unit() exact_alias_pair = fn {{
                u8 seed = 7;
                u8 result = same(seed, seed);
                core.require(result <= 10, {OUTPUT});
            }};
            unit() alias_second_hole = fn {{
                u8 seed = 11;
                u8 result = same(seed, seed);
                core.require(result <= 10, {OUTPUT});
            }};
            proof bounds for pair(first, second) -> (out) where (first >= 5 && second <= 3) ensures (out <= 3) {{}}
            proof aliases for same(first, second) -> (out) where (first >= 5 && second <= 10) ensures (out <= 10) {{}}
            {extra}
        %%end"#);
        let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
            .ok().expect("valid named proof module");
        check_loaded_scalar_module_proof_call(
            (&identity, proof), (&identity, caller), Vec::new(), (catalog, Vec::new()),
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(use_bound("bounds", "accepted", ""), ScalarProofCallOutcome::Established);
    for caller in ["swapped", "first_hole", "second_hole"] {
        assert_eq!(use_bound("bounds", caller, ""), ScalarProofCallOutcome::Unavailable, "{caller}");
    }
    assert_eq!(use_bound("aliases", "exact_alias_pair", ""), ScalarProofCallOutcome::Established);
    assert_eq!(use_bound("aliases", "alias_second_hole", ""), ScalarProofCallOutcome::Unavailable);
    assert_eq!(
        use_bound("bounds", "accepted", "u16 reached = pair(7, 3);"),
        ScalarProofCallOutcome::Unavailable,
    );
    let wrong_width = format!(
        "unit() wrong_width = fn {{ u16 low = 7; u16 result = pair(low, 3); core.require(result <= 3, {OUTPUT}); }};",
    );
    assert_eq!(use_bound("bounds", "wrong_width", &wrong_width), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn paired_actual_proofs_observe_only_exact_initialized_tuples_in_slot_order() {
    let identity = source();
    let proof = |target: &str, initializers: &str, guard: &str, goal: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "observed",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(u8, i16) pair = fn(low, high) {{ if (low == 0) {{}}; high }};
                    i16(i16, i16) aliases = fn(low, high) {{ if (low == 0) {{}}; high }};
                    unit(u8, i16) unit_pair = fn(low, high) {{
                        if (low == 0) {{}};
                        core.require(high <= 0, {OUTPUT});
                    }};
                    unit() nested = fn {{
                        i16 out = pair(7, 3);
                        core.require(out <= 3, {OUTPUT});
                    }};
                    {initializers}
                    proof observed for {target}(first, second) -> (out) where ({guard}) ensures ({goal}) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };

    let exact = "i16 early = pair(7, -128); i16 late = pair(8, -4);";
    assert_eq!(proof("pair", exact, "first >= 7 && second < 0", "out <= -4"), ScalarModuleProofOutcome::Proved);
    assert_eq!(proof("pair", exact, "first >= 9 && second < 0", "out <= -4"), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(proof("pair", exact, "first <= 7 && second < 0", "out <= -128"), ScalarModuleProofOutcome::Proved);
    let bad_then_good = "i16 early = pair(7, -2); i16 late = pair(8, -4);";
    assert_eq!(proof("pair", bad_then_good, "first >= 7 && second < 0", "out <= -4"), ScalarModuleProofOutcome::Refuted);
    let named = "u8 low_seed = 7; i16 high_seed = -128; i16 selected = pair(low_seed, high_seed);";
    assert_eq!(proof("pair", named, "first >= 7 && second < 0", "out == -128"), ScalarModuleProofOutcome::Proved);
    let repeated = "i16 known = -4; i16 selected = aliases(known, known);";
    assert_eq!(proof("aliases", repeated, "first >= -4 && second <= -4", "out == -4"), ScalarModuleProofOutcome::Proved);
    for extra in ["bool state = true; state = false;", "nested();", "unit_pair(7, -128);"] {
        let incomplete = format!("{exact} {extra}");
        assert_eq!(proof("pair", &incomplete, "first >= 7", "out <= -4"), ScalarModuleProofOutcome::Unsupported, "{extra}");
    }
}

#[test]
fn selected_initializer_arm_uses_known_typed_integer_storage_and_exact_results() {
    let passed = format!(r#"
        i16(u8, i16) pair = fn(low, high) {{ if (low == 0) {{}}; high }};
        i16 ceiling = -4;
        i16 measured = pair(7, -128);
        bool untouched = true;
        if (measured <= ceiling) {{
            if (measured == -128) {{ core.require(untouched, {OUTPUT}); }};
        }} else {{ untouched = false; }};
        core.require(untouched, {OUTPUT});
    "#);
    assert_eq!(check_scoped(&passed, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);

    let failed = format!(r#"
        i16 measured = -4;
        if (measured < -128) {{ core.narrow(false, {OUTPUT}); }}
        else {{ core.require(false, {OUTPUT}); }};
        core.narrow(false, {OUTPUT});
    "#);
    let ScalarCheckerOutcome::Failed(failure) = check(&failed) else {
        panic!("expected the first requirement failure in the selected else arm");
    };
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    let canonical = format!("%%start\n{failed}\n%%end");
    assert_eq!(failure.condition.start as usize, canonical.find("core.require(false").unwrap() + "core.require(".len());

    let nonliteral = format!("i16 measured = -4; if (!(measured < 0)) {{ core.require(true, {OUTPUT}); }};");
    assert_eq!(check(&nonliteral), ScalarCheckerOutcome::Unsupported);
}

fn dormant_source(identity: SourceIdentity, contents: &str) -> wosy_compiler::ScalarSourceDeclarations {
    let contents = if contents.is_empty() { "unit() placeholder = fn {};" } else { contents };
    let parsed = parse_source(identity, format!("%%start\n{contents}\n%%end"), &[]);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
}

#[test]
fn explicit_dormant_same_package_functions_are_symbolic_without_running_their_initializer() {
    let main = source();
    let dormant = SourceIdentity::new("local".into(), "demo".into(), "src/extra.w".into(), "phase4".into());
    let run = |contents: &str, scope| check_scalar_package_with_dormant_sources(
        vec![(dormant_source(main.clone(), "unit() started = fn {}; started();"), Vec::new())],
        vec![main.clone()],
        vec![dormant_source(dormant.clone(), contents)],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        scope,
    );
    let top_only = format!("core.require(false, {OUTPUT});");
    assert_eq!(run(&top_only, ScalarCheckerScope::CurrentPackage), ScalarCheckerOutcome::NoChecks);
    let pass = format!("{top_only} unit(u8) dormant_check = fn(value) {{ core.require(value >= 0, {OUTPUT}); }};");
    assert_eq!(run(&pass, ScalarCheckerScope::CurrentPackage), ScalarCheckerOutcome::Passed);
    let fail = format!("{top_only} unit(u8) dormant_check = fn(value) {{ core.require(value < 7, {OUTPUT}); }};");
    let ScalarCheckerOutcome::Failed(failure) = run(&fail, ScalarCheckerScope::CurrentPackage) else {
        panic!("expected unused call to fail for arbitrary typed inputs");
    };
    assert_eq!(failure.source, dormant);
    let authored = format!("%%start\n{fail}\n%%end");
    assert_eq!(&authored[failure.condition.start as usize..failure.condition.end as usize], "value < 7");
    assert_eq!(failure.witness.axes().next().unwrap().1, &wosy_compiler::ScalarType::U8);
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&7.into(), &255.into())));
    assert_eq!(run(&fail, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::NoChecks);

    let needs_state = format!("u8 installed = 1; unit() dormant_check = fn {{ core.require(installed == 1, {OUTPUT}); }};");
    assert_eq!(run(&needs_state, ScalarCheckerScope::CurrentPackage), ScalarCheckerOutcome::Unsupported);
}

#[test]
fn explicit_dormant_selection_cannot_alias_startup_or_import_a_namespace_graph() {
    let main = source();
    let dormant = SourceIdentity::new("local".into(), "demo".into(), "src/extra.w".into(), "phase4".into());
    let other_package = SourceIdentity::new("local".into(), "other".into(), "src/extra.w".into(), "phase4".into());
    let other_revision = SourceIdentity::new("local".into(), "demo".into(), main.path.clone(), "elsewhere".into());
    let run = |active: &str, links, selected, scope| check_scalar_package_with_dormant_sources(
        vec![(dormant_source(main.clone(), active), links)],
        vec![main.clone()],
        selected,
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        scope,
    );
    for selection in [
        vec![dormant_source(other_package, "")],
        vec![dormant_source(other_revision, "")],
        vec![dormant_source(dormant.clone(), ""), dormant_source(dormant.clone(), "")],
        vec![dormant_source(dormant.clone(), r#"owner = namespace demo "main.w";"#)],
    ] {
        assert_eq!(run("", Vec::new(), selection, ScalarCheckerScope::CurrentPackage), ScalarCheckerOutcome::Unsupported);
    }
    let active = r#"aux = namespace demo "extra.w";"#;
    let main_declarations = dormant_source(main.clone(), active);
    let span = main_declarations.namespaces().next().unwrap().binding_span;
    assert_eq!(run(
        active,
        vec![ScalarNamespaceBinding { binding: "aux".into(), target: dormant.clone(), span }],
        vec![dormant_source(dormant.clone(), "")],
        ScalarCheckerScope::CurrentPackage,
    ), ScalarCheckerOutcome::Unsupported);
    assert_eq!(run(
        "", Vec::new(),
        vec![dormant_source(dormant, &format!("unit() invalid = fn {{ core.require(7, {OUTPUT}); }};"))],
        ScalarCheckerScope::MainFlowOnly,
    ), ScalarCheckerOutcome::NoChecks);
}

#[test]
fn unused_symbolic_roots_forward_only_exact_nested_inputs_and_keep_actual_scope_separate() {
    let body = |guard: &str, called: &str, actual: &str| format!(r#"
        unit(u8) symbolic = fn(value) {{
            {guard}
            {called}
            core.require(true, {OUTPUT});
        }};
        unit(u8) leaf = fn(value) {{ core.require(value < 2, {OUTPUT}); }};
        {actual}
    "#);
    let known = body(&format!("core.narrow(value == 2, {OUTPUT});"), "leaf(value);", "leaf(1);");
    let ScalarCheckerOutcome::Failed(failure) = check(&known) else {
        panic!("expected nested failure at exact symbolic input");
    };
    let at = format!("%%start\n{known}\n%%end");
    assert_eq!(&at[failure.condition.start as usize..failure.condition.end as usize], "value < 2");
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&2.into(), &2.into())));
    assert_eq!(check_scoped(&known, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);

    let unknown = body("", "leaf(value);", "leaf(1);");
    assert_eq!(check(&unknown), ScalarCheckerOutcome::Unsupported);
    let not_visited = body("", "if (false) { leaf(value); };", "leaf(1);");
    assert_eq!(check(&not_visited), ScalarCheckerOutcome::Passed);

    let still_unused = body("if (value == 0) {};", "leaf(1);", "");
    let result = check(&still_unused);
    let ScalarCheckerOutcome::Failed(failure) = result else {
        panic!("a symbolic-only callee still needs its own arbitrary unused-root check, got {result:?}");
    };
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&2.into(), &255.into())));

    let transitively_reaches = format!(r#"
        unit() outer = fn {{ wrapper(); }};
        unit() wrapper = fn {{ leaf(1); leaf(2); }};
        unit(u8) leaf = fn(value) {{ core.require(value < 2, {OUTPUT}); }};
        leaf(1);
    "#);
    let ScalarCheckerOutcome::Failed(failure) = check(&transitively_reaches) else {
        panic!("transitive unused roots reach authored obligations without dummy directives");
    };
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&2.into(), &2.into())));
    assert_eq!(check_scoped(&transitively_reaches, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);

    let recursive = body("if (value == 0) {};", "symbolic(1);", "leaf(1);");
    assert_eq!(check(&recursive), ScalarCheckerOutcome::Unsupported);
}

#[test]
fn dormant_symbolic_roots_can_check_a_nested_exact_unit_call_without_initialization() {
    let main = source();
    let dormant = SourceIdentity::new("local".into(), "demo".into(), "src/extra.w".into(), "phase4".into());
    let extra = format!(r#"
        core.narrow(false, {OUTPUT});
        unit() caller = fn {{ leaf(2); }};
        unit(u8) leaf = fn(input) {{ core.require(input < 2, {OUTPUT}); }};
    "#);
    let result = check_scalar_package_with_dormant_sources(
        vec![(dormant_source(main.clone(), ""), Vec::new())],
        vec![main],
        vec![dormant_source(dormant.clone(), &extra)],
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ScalarCheckerScope::CurrentPackage,
    );
    let ScalarCheckerOutcome::Failed(failure) = result else {
        panic!("expected same-source dormant nested obligation");
    };
    assert_eq!(failure.source, dormant);
    assert_eq!(failure.operation, ScalarCheckerOperation::Require);
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&2.into(), &2.into())));
}

#[test]
fn callable_unit_if_else_preserves_structural_failure_and_selected_actual_paths() {
    let callable = |start: &str| format!(r#"
        unit(u8) select = fn(value) {{
            if (value < 8) {{
                core.require(value <= 6, {OUTPUT});
            }} else {{
                core.require(value >= 9, {OUTPUT});
            }};
            core.require(false, {OUTPUT});
        }};
        {start}
    "#);
    let unused = callable("");
    let ScalarCheckerOutcome::Failed(failure) = check(&unused) else {
        panic!("expected the first failing symbolic arm");
    };
    let authored = format!("%%start\n{unused}\n%%end");
    assert_eq!(&authored[failure.condition.start as usize..failure.condition.end as usize], "value <= 6");
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&7.into(), &7.into())));

    let actual = callable("select(8);");
    let ScalarCheckerOutcome::Failed(failure) = check_scoped(&actual, ScalarCheckerScope::MainFlowOnly) else {
        panic!("expected first failure in the selected actual else arm");
    };
    let authored = format!("%%start\n{actual}\n%%end");
    assert_eq!(&authored[failure.condition.start as usize..failure.condition.end as usize], "value >= 9");
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&8.into(), &8.into())));
}

#[test]
fn callable_unit_if_else_joins_only_the_remaining_feasible_rows() {
    let joined = format!(r#"
        unit(u8) retained = fn(value) {{
            if (value < 8) {{
                core.narrow(value == 7, {OUTPUT});
            }} else {{
                core.narrow(value == 9, {OUTPUT});
            }};
            core.require(value < 10, {OUTPUT});
            core.require(value != 7, {OUTPUT});
        }};
    "#);
    let ScalarCheckerOutcome::Failed(failure) = check(&joined) else {
        panic!("expected only the final failing joined state");
    };
    let authored = format!("%%start\n{joined}\n%%end");
    assert_eq!(&authored[failure.condition.start as usize..failure.condition.end as usize], "value != 7");
    assert_eq!(failure.witness.intervals_at(0, 0).unwrap().next(), Some((&7.into(), &7.into())));

    let selected = format!(r#"
        unit(u8) retained = fn(value) {{
            if (value < 8) {{
                core.require(!(value <= 100), {OUTPUT});
            }} else {{
                core.require(value >= 8, {OUTPUT});
            }};
        }};
        retained(9);
    "#);
    assert_eq!(check_scoped(&selected, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Passed);
    assert_eq!(check(&selected.replace("retained(9);", "retained(7);")), ScalarCheckerOutcome::Unsupported);
}

#[test]
fn observed_module_proof_old_compares_the_exact_original_entry_of_each_call() {
    let identity = source();
    let check_old = |target: &str, argument: &str, guard: &str, start: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "snapshot",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) identity_value = fn(value) {{ value }};
                    i16(i16) constant_value = fn(value) {{ if (value == 0) {{}}; 7 }};
                    u16(u8) different_width = fn(value) {{ if (value == 0) {{}}; 7 }};
                    {start}
                    proof snapshot for {target}(input) -> (out) where ({guard}) ensures (out == core.old({argument})) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };

    let separate = "i16 early = identity_value(-128); i16 late = identity_value(7);";
    assert_eq!(check_old("identity_value", "input", "true", separate), ScalarModuleProofOutcome::Proved);
    let same = "i16 early = constant_value(7); i16 late = constant_value(7);";
    assert_eq!(check_old("constant_value", "input", "true", same), ScalarModuleProofOutcome::Proved);
    let wrong_then_right = "i16 early = constant_value(6); i16 late = constant_value(7);";
    assert_eq!(check_old("constant_value", "input", "true", wrong_then_right), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check_old("constant_value", "input", "input == 7", wrong_then_right), ScalarModuleProofOutcome::Proved);
    assert_eq!(check_old("constant_value", "input", "input > 7", wrong_then_right), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check_old("identity_value", "input", "true", ""), ScalarModuleProofOutcome::Proved);
    assert!(matches!(check_old("identity_value", "out", "true", separate), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check_old("identity_value", "input + 0", "true", separate), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check_old("different_width", "input", "true", "u16 observed = different_width(7);"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn observed_module_proof_old_selects_exact_typed_entry_slot_for_relations() {
    let identity = source();
    let check = |goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "snapshot",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(u8, i16) echo = fn(tag, value) {{ if (tag == 0) {{}}; value }};
                    {actual}
                    proof snapshot for echo(tag, original) -> (out) where ({guard}) ensures ({goal}) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let two = "i16 a = echo(7, -128); i16 b = echo(0, 7);";
    assert_eq!(check("out >= core.old(original)", "true", two), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("out == core.old(original)", "tag == 0", two), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("out < core.old(original)", "true", two), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("out != core.old(original)", "tag == 1", two), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("out >= core.old(original)", "true", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("out >= core.old(tag)", "true", two), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("out >= core.old(out)", "true", two), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_proof_reflexive_branch_prunes_only_typed_immutable_aliases() {
    let identity = source();
    let check = |condition: &str, guard: &str, goal: &str, initial: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "exact",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8, u8) target = fn(x, y) {{
                        if (y == 0) {{}};
                        if ({condition}) {{ x + 1 }} else {{ x }}
                    }};
                    {initial}
                    proof exact for target(a, b) -> (out) where ({guard}) ensures ({goal}) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check("x < x", "a <= 5", "out <= 5", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("x != x", "a <= 5", "out < 5", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("x != x", "false", "out <= 5", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("x == x", "a <= 5", "out <= 5", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("x < y", "a <= 5", "out <= 5", ""), ScalarModuleProofOutcome::Unsupported);
    let startup = "u8 live = target(4, 0);";
    assert_eq!(check("x < x", "a <= 5", "out <= 5", startup), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_reflexive_branch_does_not_change_other_proofs_startup() {
    let identity = source();
    let check = |added: &str, selections: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), selections,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) plain = fn(value) {{ value }};
                    u8(u8) other = fn(value) {{
                        if (value != value) {{ value + 1 }} else {{ value }}
                    }};
                    u8 startup = other(7);
                    proof p for plain(input) -> (out) where (input == 7) ensures (out == 7) {{}}
                    {added}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    let q = "proof q for other(input) -> (out) ensures (out <= 255) {}";
    let invalid = "proof q for other(input) -> (out) ensures (out <= 256) {}";
    let p = check("", &["p"]);
    assert_eq!(p, [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(check(q, &["p"]), p);
    assert_eq!(check(q, &["p", "q"]), [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Unsupported]);
    assert!(matches!(check(invalid, &["p", "q"]).as_slice(),
        [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Invalid(_)]));
}

#[test]
fn unused_module_proof_old_uses_only_immutable_entry_authority() {
    let identity = source();
    let check = |target: &str, goal: &str, guard: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "snapshot",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) echo = fn(tag, value) {{ if (tag == 0) {{}}; value }};
                    i16(i16, i16) constant = fn(tag, value) {{ if (tag == 0) {{}}; if (value == 0) {{}}; 7 }};
                    i16(i16, i16) changed = fn(tag, value) {{ if (tag == 0) {{}}; if (value == 0) {{}}; value = 7; value }};
                    i16(i16, i16) literal_then_other = fn(tag, value) {{
                        if (value == 0) {{}}; if (tag == 0) {{ 7 }} else {{ tag }}
                    }};
                    i16(i16, i16) other_then_literal = fn(tag, value) {{
                        if (value == 0) {{}}; if (tag == 0) {{ tag }} else {{ 7 }}
                    }};
                    proof snapshot for {target}(tag, original) -> (out) where ({guard}) ensures ({goal}) {{}}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check("echo", "out == core.old(original)", "true"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("echo", "out < core.old(original)", "true"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("echo", "out == core.old(tag)", "true"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("constant", "out > core.old(original)", "original < 7"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("constant", "out > core.old(original)", "original <= 7"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("constant", "out > core.old(original)", "original == 7"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("constant", "out > core.old(original)", "original != 7 && original == 7"),
        ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("changed", "out == core.old(original)", "true"), ScalarModuleProofOutcome::Unsupported);
    for target in ["literal_then_other", "other_then_literal"] {
        assert_eq!(check(target, "out > core.old(original)", "original == 7"), ScalarModuleProofOutcome::Unsupported);
    }
}

#[test]
fn observed_entry_comparison_does_not_supply_a_reusable_module_bound() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        unit() caller = fn {{
            u8 answer = producer(7);
            core.require(answer <= 7, {OUTPUT});
        }};
        proof snapshot for producer(input) -> (out) where (input <= 7) ensures (out == core.old(input)) {{}}
    %%end"#);
    let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
        .ok().expect("valid named proof syntax");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&identity, "snapshot"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn proof_decreases_is_typed_but_does_not_admit_unchecked_induction() {
    let identity = source();
    let check_rank = |clause: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "ranked",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u16(u16) state = fn(value) {{ value }};
                    proof ranked for state(entry) -> (out) where (entry <= 7)
                        ensures (out <= 7) {clause} {{ {steps} }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check_rank("", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check_rank("decreases (entry)", "core.use(ranked(entry - 1));"), ScalarModuleProofOutcome::Unsupported);
    let guarded = "if (entry == 0) { core.show(out == 0); } else { core.use(ranked(entry - 1)); }";
    assert_eq!(check_rank("decreases (entry)", guarded), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check_rank("decreases (entry, entry)", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check_rank("decreases (out)", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check_rank("decreases (missing)", ""), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_induction_proves_only_the_complete_canonical_unused_return_case() {
    let identity = source();
    let check = |base: u8, filter: &str, recursive: &str, start: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "rank",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) descend = fn(value) {{
                        if (value == 0) {{ {base} }} else {{ {recursive} }}
                    }};
                    proof rank for descend(entry) -> (out) where ({filter}) ensures (out == 0) decreases (entry) {{
                        if (entry == 0) {{ core.show(out == 0); }} else {{ core.use(rank(entry - 1)); }}
                    }}
                    {start}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let tail = "descend(value - 1)";
    assert_eq!(check(0, "true", tail, ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(0, "entry == 0", tail, ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(1, "true", tail, ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(1, "false", tail, ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check(0, "entry > 0", tail, ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(0, "true", "descend(value - 1) + 1", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(0, "true", "descend(value - 2)", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(0, "true", tail, "u8 actual = descend(7);"), ScalarModuleProofOutcome::Proved);
}

#[test]
fn proved_named_induction_supplies_exact_equality_to_an_independent_applicable_call_only() {
    let identity = source();
    let check = |base: u8, goal: &str, guard: &str, call: &str, requirement: &str, start: &str| {
        let text = format!(r#"%%start
            u8(u8) descend = fn(value) {{ if (value == 0) {{ {base} }} else {{ descend(value - 1) }} }};
            u8(u8) other = fn(value) {{ value }};
            unit() caller = fn {{
                u8 result = {call};
                core.require({requirement}, {OUTPUT});
            }};
            proof rank for descend(entry) -> (out) where ({guard}) ensures ({goal}) decreases (entry) {{
                if (entry == 0) {{ core.show(out == 0); }} else {{ core.use(rank(entry - 1)); }}
            }}
            {start}
        %%end"#);
        let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
            .ok().expect("valid named recursive proof syntax");
        check_loaded_scalar_module_proof_call(
            (&identity, "rank"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check(0, "out == 0", "entry <= 7", "descend(7)", "result == 0", ""), ScalarProofCallOutcome::Established);
    assert_eq!(check(0, "out == 0", "entry <= 7", "descend(7)", "result == 1", ""), ScalarProofCallOutcome::NotEstablished);
    assert_eq!(check(0, "out == 0", "entry <= 7", "descend(8)", "result == 0", ""), ScalarProofCallOutcome::Unavailable);
    assert_eq!(check(0, "out == 0", "entry <= 7", "other(7)", "result == 0", ""), ScalarProofCallOutcome::Unavailable);
    assert_eq!(check(1, "out == 0", "entry <= 7", "descend(7)", "result == 0", ""), ScalarProofCallOutcome::Unavailable);
    assert_eq!(check(0, "out <= 1", "entry <= 7", "descend(7)", "result == 0", ""), ScalarProofCallOutcome::Unavailable);
    assert_eq!(check(0, "out == 0", "false", "descend(7)", "result == 0", ""), ScalarProofCallOutcome::Unavailable);
    assert_eq!(check(0, "out == 0", "entry <= 7", "descend(7)", "result == 0", "u8 actual = descend(3);"), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn named_lexicographic_induction_checks_both_typed_axes_and_exact_recursive_arguments() {
    let identity = source();
    let check = |later: &str, limit: &str, filter: &str, rank: &str, actual_first: &str, proof_first: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "rank",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    {later}(u16, {later}) descend = fn(prefix, value) {{
                        if (value == {limit}) {{ 0 }} else {{ descend({actual_first}, value - 1) }}
                    }};
                    proof rank for descend(offset, entry) -> (out) where ({filter}) ensures (out == 0) decreases ({rank}) {{
                        if (entry == {limit}) {{ core.show(out == 0); }} else {{ core.use(rank({proof_first}, entry - 1)); }}
                    }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()],
            ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check("u8", "0", "offset >= 500 && entry <= 7", "offset, entry", "prefix", "offset"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("i8", "-128", "offset >= 500 && entry < 0", "offset, entry", "prefix", "offset"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("u8", "0", "offset < 0 && entry <= 7", "offset, entry", "prefix", "offset"), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("u8", "0", "offset >= 500 && entry > 0", "offset, entry", "prefix", "offset"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("u8", "0", "true", "offset, entry", "prefix + 1", "offset"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("u8", "0", "true", "offset, entry", "prefix", "offset + 1"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("u8", "0", "true", "entry, offset", "prefix", "offset"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn one_independently_verified_equality_supplies_only_applicable_actual_startup_returns() {
    let identity = source();
    let check = |base: u8, goal: &str, filter: &str, initializers: &str, names: &[&str]| {
        check_scalar_module_proofs(
            identity.clone(), names,
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    u8(u8) descend = fn(value) {{ if (value == 0) {{ {base} }} else {{ descend(value - 1) }} }};
                    proof rank for descend(entry) -> (out) where ({filter}) ensures ({goal}) decreases (entry) {{
                        if (entry == 0) {{ core.show(out == 0); }} else {{ core.use(rank(entry - 1)); }}
                    }}
                    proof literal for descend(input) -> (out) ensures (out <= 7) {{}}
                    proof invalid for descend(input) -> (out) ensures (unknown <= 7) {{}}
                    {initializers}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        ).unwrap()
    };
    let exact = format!("u8 first = descend(2); u8 second = descend(7); core.require(first == second, {OUTPUT});");
    assert_eq!(check(0, "out == 0", "entry <= 7", &exact, &["rank"]), [ScalarModuleProofOutcome::Proved]);
    let different = "u8 first = descend(2); u8 second = descend(8);";
    assert_eq!(check(0, "out == 0", "entry <= 7", different, &["rank"]), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(check(1, "out == 0", "entry <= 7", &exact, &["rank"]), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(check(0, "out <= 1", "entry <= 7", &exact, &["rank"]), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(check(0, "out == 0", "false", &exact, &["rank"]), [ScalarModuleProofOutcome::Unsupported]);
    assert_eq!(check(0, "out == 0", "entry <= 7", &exact, &["rank", "literal"]),
        [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Unsupported]);

    assert_eq!(check(0, "out == 0", "entry <= 7", &exact, &["rank", "missing"]),
        [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Unsupported]);
    assert!(matches!(check(0, "out == 0", "entry <= 7", &exact, &["rank", "invalid"]).as_slice(),
        [ScalarModuleProofOutcome::Unsupported, ScalarModuleProofOutcome::Invalid(_)]));
    assert_eq!(check(0, "out == 0", "entry <= 7", &exact, &["rank", "rank"]),
        [ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Proved]);

    let ordinary = format!("u8(u8) descend = fn(value) {{ if (value == 0) {{ 0 }} else {{ descend(value - 1) }} }}; {exact}");
    assert_eq!(check_scoped(&ordinary, ScalarCheckerScope::MainFlowOnly), ScalarCheckerOutcome::Unsupported);
}

#[test]
fn named_proof_output_conjunction_checks_every_atom_on_shared_return_paths() {
    let identity = source();
    let check = |goal: &str, guard: &str, actual: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> (out) where ({guard}) ensures ({goal}) {{ {steps} }}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let band = "(out >= -8 && out <= 7) && out != 0";
    assert_eq!(check(band, "input >= -8 && input <= 7 && input != 0", "", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(band, "input >= -8 && input <= 7", "", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(band, "false", "", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check(band, "true", "i16 first = echo(-8); i16 second = echo(7);", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(band, "true", "i16 first = echo(0); i16 second = echo(7);", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("out == 7 && out != 7", "input == 7", "", ""), ScalarModuleProofOutcome::Refuted);
    assert!(matches!(check("out == 300 && missing == 7", "input == 7", "", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("out <= 32768 && out >= -8", "input == 7", "", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("out >= -8 && input <= 7", "true", "", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("out >= -8 && out <= 7", "input == 7", "", "core.show(out == 0);"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_where_disjunction_admits_the_exact_domain_in_actual_and_unused_contexts() {
    let identity = source();
    let check = |guard: &str, goal: &str, actual: &str, steps: &str| {
        check_scalar_module_proof(
            identity.clone(), "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> (out) where ({guard}) ensures ({goal}) {{ {steps} }}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let guard = "(input < -2 || input > 2) && input >= -4 && input <= 4";
    let goal = "out <= -3 || out >= 3";
    assert_eq!(check(guard, goal, "", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(guard, "out <= 4", "", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(guard, "out < 4", "", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(guard, goal, "i16 a = echo(0); i16 b = echo(5);", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check(guard, goal, "i16 a = echo(-4); i16 b = echo(0); i16 c = echo(4);", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(guard, "out < 4", "i16 a = echo(0); i16 b = echo(4);", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("(input < -2 || input > 2) && input == 0", goal, "", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("input == -3 || true", goal, "", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("input == -3 || input == 3 || input == 4", goal, "", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("input < 0 || input == 32768", goal, "", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check(guard, "out <= 4", "", "core.show(out == 0);"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_where_disjunction_reuses_only_independently_proved_exact_domain() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        u8(u8, u8) mixed_axis = fn(a, b) {{ if (b == 0) {{ a }} else {{ a }} }};
        unit() caller = fn {{
            u8 answer = producer(7);
            core.require(answer <= 7, {OUTPUT});
        }};
        proof bounded for producer(input) -> (out) where (input == 1 || input == 7) ensures (out <= 7) {{}}
        proof mixed for mixed_axis(a, b) -> (out) where (a == 1 || b == 7) ensures (out <= 7) {{}}
    %%end"#);
    let outcomes = check_scalar_module_proofs(
        identity.clone(), &["bounded", "mixed"],
        vec![ScalarProofModuleInput { source: identity.clone(), text: text.clone(), namespaces: Vec::new() }],
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    assert_eq!(outcomes.unwrap(), [ScalarModuleProofOutcome::Proved, ScalarModuleProofOutcome::Unsupported]);
    let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
        .ok().expect("canonical proof source");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&identity, "bounded"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Established);
}

#[test]
fn named_proof_output_disjunction_reuse_preserves_proved_gaps() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        unit() gap_excluded = fn {{
            u8 answer = producer(7);
            core.require(answer != 4, {OUTPUT});
        }};
        unit() no_correlation = fn {{
            u8 answer = producer(7);
            core.require(answer == 7, {OUTPUT});
        }};
        proof exact for producer(input) -> (out)
            where (input == 1 || input == 7) ensures (out == 1 || out == 7) {{}}
    %%end"#);
    let check = |caller: &str| {
        let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text.clone(), &[])
            .ok().expect("canonical proof source");
        check_loaded_scalar_module_proof_call(
            (&identity, "exact"), (&identity, caller), Vec::new(), (catalog, Vec::new()),
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    assert_eq!(check("gap_excluded"), ScalarProofCallOutcome::Established);
    assert_eq!(check("no_correlation"), ScalarProofCallOutcome::NotEstablished);
}

#[test]
fn named_proof_output_disjunction_checks_the_exact_union_on_every_return() {
    let identity = source();
    let check = |goal: &str, guard: &str, actual: &str, steps: &str, alias: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> ({alias}) where ({guard}) ensures ({goal}) {{ {steps} }}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let split = "out < 0 || out > 0";
    assert_eq!(check(split, "input != 0", "", "", "out"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(split, "input >= -3 && input <= 3", "", "", "out"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(split, "false", "", "", "out"), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check(split, "true", "i16 a = echo(-32768); i16 b = echo(32767);", "", "out"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(split, "true", "i16 a = echo(-32768); i16 b = echo(0); i16 c = echo(32767);", "", "out"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check("out <= -1 || out >= 0", "true", "", "", "out"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("out <= 2 || out >= -2", "true", "", "", "out"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("out <= -3 || out >= 3", "input <= -2", "", "", "out"), ScalarModuleProofOutcome::Refuted);
    assert!(matches!(check("out >= -32768 || out <= 32768", "true", "", "", "out"), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("null < 0 || null > 0", "true", "", "", "null"), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("out < 0 || input > 0", "true", "", "", "out"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("out < 0 || true", "true", "", "", "out"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(split, "input == 7", "", "core.show(out == 0);", "out"), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_proof_output_disjunction_does_not_issue_bound_authority() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        unit() caller = fn {{
            u8 answer = producer(7);
            core.require(answer <= 7, {OUTPUT});
        }};
        proof bounded for producer(input) -> (out) where (input <= 7) ensures (out == 7 || out < 7) {{}}
    %%end"#);
    assert_eq!(check_scalar_module_proof(
        identity.clone(), "bounded",
        vec![ScalarProofModuleInput { source: identity.clone(), text: text.clone(), namespaces: Vec::new() }],
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarModuleProofOutcome::Proved);
    let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
        .ok().expect("canonical proof source");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&identity, "bounded"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn verified_output_conjunction_does_not_issue_a_reusable_bound() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        unit() caller = fn {{
            u8 answer = producer(7);
            core.require(answer <= 7, {OUTPUT});
        }};
        proof band for producer(input) -> (out) where (input <= 7) ensures (out >= 0 && out <= 7) {{}}
    %%end"#);
    let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
        .ok().expect("valid named proof syntax");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&identity, "band"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn named_universal_conjunction_checks_exact_order_and_empty_inner_vacuity() {
    let identity = source();
    let check = |body: &str, domain: &str, goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ {body} }};
                    proof bounded for echo(input) -> (out) where ({guard}) forall (i16 k where {domain})
                        ensures ({goal}) {{}}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let lower = "k >= -7 && k != -7";
    let holes = "out <= k && (out >= -9 && out != -7)";
    assert_eq!(check("value", lower, holes, "input >= -9 && input <= -6 && input != -7", ""), Proved);
    assert_eq!(check("value", lower, holes, "input >= -9 && input <= -6", ""), Refuted);
    assert_eq!(check("value", lower, holes, "true", "i16 a = echo(-9); i16 b = echo(-6);"), Proved);
    assert_eq!(check("value", lower, holes, "true", "i16 a = echo(-9); i16 b = echo(-5);"), Refuted);
    assert_eq!(check("value", "k <= 7 && k != 7", "out >= k && out <= 8", "input >= 6 && input <= 8", ""), Proved);
    let same = "out >= k && out <= k && out != 0";
    assert_eq!(check("value", "k == 7", same, "input == 7", ""), Proved);
    assert_eq!(check("value", "k >= 6 && k <= 7", same, "input == 7", ""), Refuted);
    assert_eq!(check("value", "k == 7", "out < k && out >= k", "input == 7", ""), Refuted);
    assert_eq!(check("value", "false", "out < k && out >= k && out < -1 && out > 1", "true", ""), Proved);
    assert_eq!(check("value", "false", same, "true", "i16 a = echo(0);"), Proved);
    assert_eq!(check("value", "false", same, "false", ""), NoApplicableInputs);
    assert_eq!(check("value", "false", same, "false", "i16 a = echo(0);"), NoApplicableInputs);
    assert_eq!(check("if (value == 0) {}; 3 + 4", "false", same, "true", "i16 a = echo(0);"), Unsupported);
    assert_eq!(check("value", "false", "out <= k", "true", ""), Unsupported);
    assert_eq!(check("value", "false", "out <= k && out == core.old(input)", "true", ""), Unsupported);
    assert!(matches!(check("value", "false", "out <= k && out < 32768", "true", ""), Invalid(_)));
}

#[test]
fn named_universal_exact_equality_and_disjointness_use_nonempty_typed_domains() {
    let identity = source();
    let check = |body: &str, domain: &str, goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ {body} }};
                    proof bounded for echo(input) -> (out) where ({guard}) forall (i16 k where {domain})
                        ensures ({goal}) {{}}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    assert_eq!(check("value", "k == -7", "out == k", "input == -7", ""), Proved);
    assert_eq!(check("value", "k == -7", "out == k", "input == -6", ""), Refuted);
    assert_eq!(check("value", "k >= -8 && k <= -6 && k != -7", "out == k", "true", "i16 a = echo(-8);"), Refuted);
    assert_eq!(check("value", "k >= -8 && k <= -6 && k != -7", "out == k", "false", "i16 a = echo(-8);"), NoApplicableInputs);
    let domain = "k >= -8 && k <= -6 && k != -7";
    assert_eq!(check("value", domain, "out != k", "input == -7", ""), Proved);
    assert_eq!(check("value", domain, "out != k", "input >= -9 && input <= -7 && input != -8", ""), Proved);
    assert_eq!(check("value", domain, "out != k", "input >= -9 && input <= -7", ""), Refuted);
    assert_eq!(check("value", domain, "out != k", "true", "i16 a = echo(-7); i16 b = echo(2);"), Proved);
    assert_eq!(check("value", domain, "out != k", "true", "i16 a = echo(-7); i16 b = echo(-6); i16 c = echo(2);"), Refuted);
    assert_eq!(check("value", "false", "out != k", "true", ""), Unsupported);
    assert_eq!(check("value", "false", "out == k", "true", ""), Unsupported);
    assert!(matches!(check("value", "k <= 32768", "out != k", "false", ""), Invalid(_)));
    assert_eq!(check("if (value == 0) {}; 3 + 4", domain, "out != k", "true", "i16 a = echo(0);"), Unsupported);
}

#[test]
fn named_universal_conjunction_combines_equality_and_disjointness_with_bounds() {
    let identity = source();
    let check = |domain: &str, goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> (out) where ({guard}) forall (i16 k where {domain})
                        ensures ({goal}) {{}}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let domain = "k >= -8 && k <= -6 && k != -7";
    let constraints = "out != k && out >= -9 && out != -7";
    assert_eq!(check(domain, constraints, "input >= -9 && input <= -5 && input != -8 && input != -7 && input != -6", ""), Proved);
    assert_eq!(check(domain, constraints, "input == -8", ""), Refuted);
    assert_eq!(check(domain, constraints, "input == -7", ""), Refuted);
    assert_eq!(check(domain, constraints, "input == -10", ""), Refuted);
    assert_eq!(check(domain, constraints, "true", "i16 a = echo(-9); i16 b = echo(-5);"), Proved);
    assert_eq!(check(domain, constraints, "true", "i16 a = echo(-9); i16 b = echo(-8);"), Refuted);
    assert_eq!(check(domain, "out != k && out != k", "input == -7", ""), Proved);
    assert_eq!(check(domain, "out <= k && out != k && out >= -10", "input == -9", ""), Proved);
    assert_eq!(check(domain, "out <= k && out != k && out >= -10", "input == -8", ""), Refuted);
    assert_eq!(check("k == -7", "out == k && out >= -7", "input == -7", ""), Proved);
    assert_eq!(check(domain, "out == k && out >= -8", "input == -8", ""), Refuted);
    assert_eq!(check("false", "out == k && out != k && out >= 1", "true", ""), Proved);
    assert_eq!(check("false", "out == k && out != k && out >= 1", "false", ""), NoApplicableInputs);
    assert_eq!(check("false", "out != k", "true", ""), Unsupported);
    assert!(matches!(check("false", "out != k && out <= 32768", "true", ""), Invalid(_)));
}

#[test]
fn named_repeated_forall_combines_independent_exact_domains_without_cross_binder_arithmetic() {
    let identity = source();
    let check = |foralls: &str, goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> (out) where ({guard}) {foralls}
                        ensures ({goal}) {{}}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let domains = "forall (i16 lower where lower <= -8) forall (i16 upper where upper >= 8 && upper != 8)";
    let bounded = "out >= lower && (out < upper && out != 0)";
    assert_eq!(check(domains, bounded, "input >= -8 && input <= 8 && input != 0", ""), Proved);
    assert_eq!(check(domains, bounded, "input >= -8 && input <= 8", ""), Refuted);
    assert_eq!(check(domains, bounded, "input <= 9 && input >= 8", ""), Refuted);
    assert_eq!(check(domains, bounded, "true", "i16 a = echo(-8); i16 b = echo(8);"), Proved);
    assert_eq!(check(domains, bounded, "true", "i16 a = echo(-8); i16 b = echo(9);"), Refuted);
    assert_eq!(check(domains, bounded, "false", "i16 a = echo(9);"), NoApplicableInputs);
    let third = format!("{domains} forall (i16 cap where cap >= 3)");
    assert_eq!(check(&third, "out >= lower && out < upper && out <= cap", "input >= -8 && input <= 3", ""), Proved);
    assert_eq!(check(domains, "out > lower && out <= lower && out < upper", "input == 1", ""), Refuted);
    assert_eq!(check(domains, "lower <= upper && out < upper", "true", ""), Unsupported);
    assert_eq!(check(domains, "out < upper && out != 0", "true", ""), Unsupported);
    assert!(matches!(check("forall (i16 k where k >= 0) forall (i16 k where k <= 1)", "out >= k && out <= k", "true", ""), Invalid(_)));
    assert_eq!(check("forall (i16 lower where false) forall (i16 upper where upper >= 8)", bounded, "true", ""), Unsupported);
    assert!(matches!(check("forall (i16 lower where false) forall (i16 upper where upper <= 32768)", bounded, "true", ""), Invalid(_)));
    assert_eq!(check("forall (u16 lower) forall (i16 upper)", bounded, "true", ""), Unsupported);
}

#[test]
fn named_repeated_forall_folds_exact_equality_and_multiple_forbidden_domains() {
    let identity = source();
    let check = |foralls: &str, goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> (out) where ({guard}) {foralls}
                        ensures ({goal}) {{}}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Refuted, Unsupported};
    let union = "forall (i16 a where a >= -8 && a <= -6 && a != -7) forall (i16 b where b >= -6 && b <= -4 && b != -5)";
    let goal = "out != a && out != b && out >= -9 && out != -9";
    assert_eq!(check(union, goal, "input >= -7 && input <= -5 && input != -6", ""), Proved);
    assert_eq!(check(union, goal, "input >= -7 && input <= -5", ""), Refuted);
    assert_eq!(check(union, goal, "true", "i16 a = echo(-7); i16 b = echo(-5);"), Proved);
    assert_eq!(check(union, goal, "true", "i16 a = echo(-7); i16 b = echo(-4);"), Refuted);
    assert_eq!(check(union, goal, "true", "i16 a = echo(-7); i16 b = echo(-9);"), Refuted);
    assert_eq!(check(union, "out != a && out != b", "input == -9", ""), Proved);
    assert_eq!(check(union, "out == a && out != b", "input == -8", ""), Refuted);
    assert_eq!(check("forall (i16 a where a == -7) forall (i16 b where b == -5)",
        "out == a && out != b", "input == -7", ""), Proved);
    assert_eq!(check("forall (i16 a where a == -7) forall (i16 b where b == -7)",
        "out == a && out != b", "input == -7", ""), Refuted);
    assert_eq!(check(union, "out <= a && out != a && out != b", "input == -9", ""), Proved);
    assert_eq!(check(union, "out <= a && out != a && out != b", "input == -8", ""), Refuted);
    assert_eq!(check(union, "out == a && out != b", "false", ""), NoApplicableInputs);
    assert_eq!(check("forall (i16 a where a >= -8) forall (i16 b where false)",
        "out == a && out != b", "true", ""), Unsupported);
    assert!(matches!(check("forall (i16 a where a >= -8) forall (i16 b where b <= 32768)",
        "out == a && out != b", "true", ""), Invalid(_)));
}

#[test]
fn named_universal_proof_reduces_exact_typed_domains_on_shared_actual_and_unused_returns() {
    let identity = source();
    let check = |quantifiers: &str, goal: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "bounded",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof bounded for echo(input) -> (out) where ({guard}) {quantifiers} ensures ({goal}) {{}}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let lower = "forall (i16 k where k >= -7 && k != -7)";
    let upper = "forall (i16 k where k <= 7 && k != 7)";
    assert_eq!(check(lower, "out <= k", "input <= -6", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(lower, "out <= k", "input <= -5", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(upper, "out >= k", "input >= 6", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(upper, "out > k", "input >= 6", ""), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(lower, "out <= k", "true", "i16 a = echo(-9); i16 b = echo(-6);"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(lower, "out <= k", "true", "i16 a = echo(-5); i16 b = echo(-6);"), ScalarModuleProofOutcome::Refuted);
    assert_eq!(check(lower, "out <= k", "false", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("forall (i16 k)", "out <= k", "input == -32768", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check("forall (i16 k where k == 1 && k != 1)", "out <= k", "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("forall (u16 k)", "out <= k", "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("forall (i16 k) forall (i16 j)", "out <= k", "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check("forall (i16 null where null >= 7)", "out <= null", "input <= 7", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("forall (i16 input)", "out <= input", "true", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("forall (i16 k where k < 32768)", "out <= k", "true", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check("forall (i16 k where k >= input)", "out <= k", "true", ""), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn universal_proof_cannot_use_the_intrinsic_null_as_its_output_alias() {
    let identity = source();
    let check = |alias: &str| check_scalar_module_proof(
        identity.clone(),
        "bounded",
        vec![ScalarProofModuleInput {
            source: identity.clone(),
            text: format!(r#"%%start
                u8(u8) echo = fn(value) {{ value }};
                proof bounded for echo(input) -> ({alias}) where (input <= 7)
                    forall (u8 k where k >= 7) ensures ({alias} <= k) {{}}
            %%end"#),
            namespaces: Vec::new(),
        }],
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    );
    assert_eq!(check("out"), ScalarModuleProofOutcome::Proved);
    assert!(matches!(check("null"), ScalarModuleProofOutcome::Invalid(_)));
}

#[test]
fn named_universal_goal_cannot_issue_a_reusable_literal_bound() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        unit() caller = fn {{
            u8 answer = producer(7);
            core.require(answer <= 7, {OUTPUT});
        }};
        proof bounded for producer(input) -> (out) where (input <= 7) forall (u8 k where k >= 7) ensures (out <= k) {{}}
    %%end"#);
    let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
        .ok().expect("valid named proof syntax");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&identity, "bounded"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Unavailable);
}

#[test]
fn named_existential_proof_verifies_all_exact_integer_predicates_for_chosen_witness() {
    let identity = source();
    let check = |signature: &str, body: &str, candidate: &str, operator: &str, where_guard: &str, startup: &str| {
        check_scalar_module_proof(
            identity.clone(), "prove",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16({signature}) choose = fn(left, right) {{ {body} }};
                    {startup}
                    proof prove for choose(first, second) -> (out) where ({where_guard})
                        ensures (exists (i16 k where k >= -8 && k != -7) {{ out {operator} k }}) {{
                            core.witness({candidate});
                        }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Unsupported};
    let left = "if (right == 0) {}; left";
    let right = "if (left == 0) {}; right";
    let before = "first <= -9 && second >= -8 && second <= -6 && second != -7";
    for operator in ["<", "<=", "!="] {
        assert_eq!(check("i16, i16", left, "second", operator, before, ""), Proved);
    }
    for operator in [">", ">=", "=="] {
        assert_eq!(check("i16, i16", left, "second", operator, before, ""), Unsupported);
    }
    let after = "first >= -5 && second >= -8 && second <= -6 && second != -7";
    for operator in [">", ">="] {
        assert_eq!(check("i16, i16", left, "second", operator, after, ""), Proved);
    }
    assert_eq!(check("i16, i16", right, "second", "!=", "second == -8", ""), Unsupported);
    assert_eq!(check("i16, i16", left, "second", "!=", "first == -7 && second >= -8 && second <= -6 && second != -7", ""), Proved);
    assert_eq!(check("i16, i16", left, "second", "!=", "first == -6 && second >= -8 && second <= -6 && second != -7", ""), Unsupported);
    assert_eq!(check("i16, i16", left, "-8", "<", "first <= -9", ""), Proved);
    assert_eq!(check("i16, i16", left, "-7", "<", "first <= -9", ""), Unsupported);
    assert_eq!(check("i16, i16", left, "second", "<", "true",
        "i16 a = choose(-9, -8); i16 b = choose(-8, -6);"), Proved);
    assert_eq!(check("i16, i16", left, "second", "<", "true",
        "i16 a = choose(-9, -8); i16 wrong = choose(-6, -6); i16 b = choose(-8, -6);"), Unsupported);
    assert_eq!(check("i16, i16", left, "second", "<", "false", "i16 a = choose(-9, -8);"), NoApplicableInputs);
    assert_eq!(check("u8, i16", right, "first", "<", "true", ""), Unsupported);
    assert!(matches!(check("u8, i16", right, "out", ">", "true", ""), Invalid(_)));
}

#[test]
fn named_existential_conjunction_verifies_one_typed_witness_against_all_atoms() {
    let identity = source();
    let check = |predicate: &str, candidate: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(), "prove",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16, i16) left = fn(value, sample) {{ if (sample == 0) {{}}; value }};
                    {actual}
                    proof prove for left(first, entry) -> (out) where ({guard})
                        ensures (exists (i16 k where k >= -8 && k != -7) {{ {predicate} }}) {{
                            core.witness({candidate});
                        }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Unsupported};
    let guard = "first >= -12 && first <= -9 && first != -10 && entry >= -8 && entry <= -6 && entry != -7";
    let valid = "out <= k && (out != k && out >= -12 && out <= -9 && out != -10)";
    assert_eq!(check(valid, "entry", guard, ""), Proved);
    assert_eq!(check(valid, "entry", "first >= -12 && first <= -9 && entry == -8", ""), Unsupported);
    assert_eq!(check(valid, "entry", "first == -9 && entry == -7", ""), Unsupported);
    assert_eq!(check(valid, "-8", guard, ""), Proved);
    assert_eq!(check(valid, "-7", guard, ""), Unsupported);
    assert_eq!(check(valid, "entry", "true", "i16 a = left(-9, -8); i16 b = left(-11, -6);"), Proved);
    assert_eq!(check(valid, "entry", "true", "i16 a = left(-9, -8); i16 b = left(-10, -6);"), Unsupported);
    assert_eq!(check(valid, "-8", "true", "i16 a = left(-9, -8); i16 b = left(-11, -6);"), Proved);
    let contradiction = "out < k && out >= k && out >= -12";
    assert_eq!(check(contradiction, "entry", guard, ""), Unsupported);
    assert_eq!(check(contradiction, "entry", "false", ""), NoApplicableInputs);
    assert_eq!(check(contradiction, "-8", "false", "i16 a = left(-9, -8);"), NoApplicableInputs);
    assert!(matches!(check("out < k && out >= k && out < 32768", "entry", "false", ""), Invalid(_)));
    assert_eq!(check("out <= -9 && out != -10", "entry", guard, ""), Unsupported);
    assert_eq!(check("out < k || out >= -12", "entry", guard, ""), Unsupported);
}

#[test]
fn named_proof_entry_witness_checks_domain_and_output_in_same_context() {
    let identity = source();
    let check = |signature: &str, body: &str, candidate: &str, where_guard: &str, startup: &str| {
        check_scalar_module_proof(
            identity.clone(), "prove",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16({signature}) choose = fn(left, right) {{ {body} }};
                    {startup}
                    proof prove for choose(first, second) -> (out) where ({where_guard})
                        ensures (exists (i16 k where k >= -8 && k != -7) {{ out == k }}) {{
                            core.witness({candidate});
                        }}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    use ScalarModuleProofOutcome::{Invalid, NoApplicableInputs, Proved, Unsupported};
    let choose_right = "if (left == 0) {}; right";
    let choose_left = "if (right == 0) {}; left";
    let two = "i16 a = choose(1, -8); i16 b = choose(2, -6);";
    let hole = "i16 a = choose(1, -8); i16 b = choose(2, -7); i16 c = choose(3, -6);";
    assert_eq!(check("u8, i16", choose_right, "second", "true", two), Proved);
    assert_eq!(check("u8, i16", choose_right, "second", "true", hole), Unsupported);
    assert_eq!(check("u8, i16", choose_right, "second", "first != 2", hole), Proved);
    assert_eq!(check("u8, i16", choose_right, "second", "false", hole), NoApplicableInputs);
    assert_eq!(check("u8, i16", choose_right, "first", "true", two), Unsupported);
    assert_eq!(check("u8, i16", choose_right, "second",
        "second >= -8 && second <= -6 && second != -7", ""), Proved);
    assert_eq!(check("u8, i16", choose_right, "second", "second >= -8 && second <= -6", ""), Unsupported);
    assert_eq!(check("i16, i16", choose_left, "second", "first == 3 && second == 3", ""), Proved);
    assert_eq!(check("i16, i16", choose_left, "second", "first == 4 && second == 3", ""), Unsupported);
    assert_eq!(check("i16, i16", choose_left, "second", "true", "i16 a = choose(3, 4);"), Unsupported);
    assert_eq!(check("i16, i16", "if (left == 0) {}; if (right == 0) {}; 3", "second", "second == 3", ""), Proved);
    assert!(matches!(check("u8, i16", choose_right, "out", "false", two), Invalid(_)));
}

#[test]
fn named_existential_proof_checks_the_authored_typed_witness_on_each_return() {
    let identity = source();
    let check = |binder: &str, predicate: &str, witness: &str, guard: &str, actual: &str| {
        check_scalar_module_proof(
            identity.clone(),
            "exists_echo",
            vec![ScalarProofModuleInput {
                source: identity.clone(),
                text: format!(r#"%%start
                    i16(i16) echo = fn(value) {{ value }};
                    proof exists_echo for echo(input) -> (out) where ({guard})
                        ensures (exists ({binder}) {{ {predicate} }}) {{ {witness} }}
                    {actual}
                %%end"#),
                namespaces: Vec::new(),
            }],
            vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
        )
    };
    let binder = "i16 k where k >= -8 && k != -7";
    let predicate = "out == k";
    let candidate = "core.witness(-8);";
    assert_eq!(check(binder, predicate, candidate, "input == -8", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(binder, predicate, candidate, "true", "i16 a = echo(-8); i16 b = echo(-8);"), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(binder, predicate, candidate, "true", "i16 a = echo(-8); i16 b = echo(-6);"), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(binder, predicate, "core.witness(-6);", "input == -6", ""), ScalarModuleProofOutcome::Proved);
    assert_eq!(check(binder, predicate, candidate, "input == -6", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(binder, predicate, "core.witness(-7);", "input == -7", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(binder, predicate, "core.witness(-7);", "false", ""), ScalarModuleProofOutcome::NoApplicableInputs);
    assert_eq!(check("i16 k where false", predicate, candidate, "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check("i16 k", predicate, "core.witness(-32768);", "input == -32768", ""), ScalarModuleProofOutcome::Proved);
    assert!(matches!(check(binder, predicate, "core.witness(32768);", "true", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("i16 null", "out == null", candidate, "true", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert!(matches!(check("i16 input", "out == input", candidate, "true", ""), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check(binder, predicate, "core.witness(input);", "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(binder, predicate, "core.witness(-8); core.witness(-6);", "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check(binder, predicate, "", "true", ""), ScalarModuleProofOutcome::Unsupported);
}

#[test]
fn named_existential_witness_is_not_a_reusable_output_bound() {
    let identity = source();
    let text = format!(r#"%%start
        u8(u8) producer = fn(value) {{ value }};
        unit() caller = fn {{
            u8 answer = producer(7);
            core.require(answer == 7, {OUTPUT});
        }};
        proof witnessed for producer(input) -> (out) where (input == 7)
            ensures (exists (u8 k where k >= 7) {{ out == k }}) {{ core.witness(7); }}
    %%end"#);
    assert_eq!(check_scalar_module_proof(
        identity.clone(), "witnessed",
        vec![ScalarProofModuleInput { source: identity.clone(), text: text.clone(), namespaces: Vec::new() }],
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarModuleProofOutcome::Proved);
    let (_, catalog) = wosy_compiler::parse_proof_source(identity.clone(), text, &[])
        .ok().expect("valid named proof syntax");
    assert_eq!(check_loaded_scalar_module_proof_call(
        (&identity, "witnessed"), (&identity, "caller"), Vec::new(), (catalog, Vec::new()),
        vec![identity.clone()], ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 },
    ), ScalarProofCallOutcome::Unavailable);
}
