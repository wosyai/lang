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
    assert_eq!(check_old("identity_value", "input", "true", ""), ScalarModuleProofOutcome::Unsupported);
    assert!(matches!(check_old("identity_value", "out", "true", separate), ScalarModuleProofOutcome::Invalid(_)));
    assert_eq!(check_old("identity_value", "input + 0", "true", separate), ScalarModuleProofOutcome::Unsupported);
    assert_eq!(check_old("different_width", "input", "true", "u16 observed = different_width(7);"), ScalarModuleProofOutcome::Unsupported);
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
