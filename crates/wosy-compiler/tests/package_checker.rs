#![cfg(feature = "internal-checker")]

use wosy_compiler::{
    check_scalar_package, derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project,
    ScalarCheckerOperation, ScalarCheckerOutcome, ScalarCheckerScope, ScalarNamespaceBinding,
    ScalarPreparationInputs, ScalarTargetLayout,
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
    assert_eq!(check(&format!("core.require(true, {OUTPUT});")), ScalarCheckerOutcome::Unsupported);
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
