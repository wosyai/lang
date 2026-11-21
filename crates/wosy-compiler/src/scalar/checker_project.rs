use super::{
    checker_directive::{CheckedLiteralOutput, CheckerOperation},
    checker_flow::{check_integer_requirements, IntegerInputs},
    project_from_declarations,
    static_calls::prepare_project_occurrences,
    typecheck_scalar_project_in_mode, ByteSpan, ReferenceBindingId, ScalarItem,
    ScalarNamespaceBinding, ScalarPreparationInputs, ScalarSourceDeclarations, ScalarType,
};
use crate::Diagnostic;
use wosy_syntax::SourceIdentity;

#[derive(Debug, PartialEq)]
enum ProjectCheck {
    Invalid(Vec<Diagnostic>),
    MissingRoot,
    AmbiguousRoot,
    RootSourceOutsideInitOrder,
    MissingRootOccurrences,
    Unsupported,
    NoChecks,
    Proved,
    Failed {
        source: SourceIdentity,
        operation: CheckerOperation,
        directive: ByteSpan,
        condition: ByteSpan,
        state: IntegerInputs,
        axes: Vec<ReferenceBindingId>,
        output: CheckedLiteralOutput,
    },
}

fn check_closed_project_callable(
    declarations: Vec<(ScalarSourceDeclarations, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<SourceIdentity>,
    inputs: ScalarPreparationInputs,
    root_source: &SourceIdentity,
    root_name: &str,
) -> ProjectCheck {
    let prepared = project_from_declarations(declarations, initialization_order, inputs)
        .and_then(|project| prepare_project_occurrences(typecheck_scalar_project_in_mode(project, true)));
    let (project, calls) = match prepared {
        Ok(prepared) => prepared,
        Err(rejected) => return ProjectCheck::Invalid(rejected.into_inspection().diagnostics),
    };
    let mut roots = project
        .modules
        .iter()
        .enumerate()
        .filter(|(_, module)| &module.source == root_source)
        .flat_map(|(module, source)| {
            source.items.iter().enumerate().filter_map(move |(item, value)| {
                let ScalarItem::Function(function) = value else {
                    return None;
                };
                (function.name == root_name).then_some((module, item, function))
            })
        });
    let Some((module, item, function)) = roots.next() else {
        return ProjectCheck::MissingRoot;
    };
    if roots.next().is_some() {
        return ProjectCheck::AmbiguousRoot;
    }
    if !project.initialization_order.contains(root_source) {
        return ProjectCheck::RootSourceOutsideInitOrder;
    }
    let ScalarType::Callable { outputs, parameters } = &function.signature else {
        return ProjectCheck::Unsupported;
    };
    if !parameters.is_empty()
        || !matches!(outputs.outputs.as_slice(), [output] if output.ty == ScalarType::Unit)
        || !function.generic_parameters.is_empty()
        || !function.overload_arms.is_empty()
    {
        return ProjectCheck::Unsupported;
    }
    let Some(body) = calls.function_body(module, item, None) else {
        return ProjectCheck::MissingRootOccurrences;
    };
    match check_integer_requirements(body) {
        None => ProjectCheck::Unsupported,
        Some(Ok(())) if body.structure.checker_directives.is_empty() => ProjectCheck::NoChecks,
        Some(Ok(())) => ProjectCheck::Proved,
        Some(Err(failure)) => {
            let directive = &body.structure.checker_directives[&failure.directive];
            ProjectCheck::Failed {
                source: root_source.clone(),
                operation: directive.operation,
                directive: body.occurrences[failure.directive.0].span,
                condition: directive.condition,
                state: failure.state,
                axes: failure.axes.into_iter().cloned().collect(),
                output: directive.output.clone(),
            }
        }
    }
}

mod tests {
    use super::*;
    use num_bigint::BigInt;
    use crate::{derive_scalar_declarations_from_cst, parse_source, prepare_scalar_project, ScalarTargetLayout};

    const OUTPUT: &str = r#"{ .stdout = ""; .stderr = "unexpected state"; .code = 1; .files = []; }"#;

    fn source(path: &str, revision: &str) -> SourceIdentity {
        SourceIdentity::new("local".into(), "demo".into(), path.into(), revision.into())
    }

    fn declarations(source: &SourceIdentity, text: &str) -> ScalarSourceDeclarations {
        let parsed = parse_source(source.clone(), text.into(), &[]);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        derive_scalar_declarations_from_cst(parsed.result.canonical_cst())
    }

    fn inputs() -> ScalarPreparationInputs {
        ScalarPreparationInputs { target_layout: ScalarTargetLayout::WASM32 }
    }

    fn script(body: &str) -> String {
        format!("%%start\nunit() proof = fn {{ {body} }};\n%%end")
    }

    fn check(source: &SourceIdentity, text: &str, name: &str) -> ProjectCheck {
        check_closed_project_callable(
            vec![(declarations(source, text), Vec::new())],
            vec![source.clone()],
            inputs(),
            source,
            name,
        )
    }

    #[test]
    fn checks_the_selected_project_callable_and_preserves_its_failure_spans() {
        let app = source("src/main.w", "app");
        let other = source("src/other.w", "dependency");
        let passing = script(&format!("u8 local = 7; if (local == 7) {{}}; core.require(local == 7, {OUTPUT});"));
        let failure_output = r#"{ .stdout = "saída\n"; .stderr = "erro\t☕"; .code = 340282366920938463463374607431768211456; .files = []; }"#;
        let failing = script(&format!("u8 local = 7; if (local == 7) {{}}; core.require(local == 8, {failure_output});"));
        for (source, expected_proof) in [(&app, false), (&other, true)] {
            let result = check_closed_project_callable(
                vec![
                    (declarations(&other, &passing), Vec::new()),
                    (declarations(&app, &failing), Vec::new()),
                ],
                vec![other.clone(), app.clone()],
                inputs(),
                source,
                "proof",
            );
            if expected_proof {
                assert_eq!(result, ProjectCheck::Proved);
            } else {
                let ProjectCheck::Failed { source, operation, directive, condition, state, axes, output } = result else {
                    panic!("expected exact failure, found {result:?}");
                };
                assert_eq!(source, app);
                assert_eq!(operation, CheckerOperation::Require);
                assert_eq!(&failing[condition.start as usize..condition.end as usize], "local == 8");
                assert_eq!(
                    &failing[directive.start as usize..directive.end as usize],
                    format!("core.require(local == 8, {failure_output})")
                );
                let authored = |span: ByteSpan| &failing[span.start as usize..span.end as usize];
                assert_eq!(output.stdout_bytes.as_ref(), "saída\n".as_bytes());
                assert_eq!(output.stderr_bytes.as_ref(), "erro\t☕".as_bytes());
                assert_eq!(output.code_value, BigInt::from(u128::MAX) + 1);
                assert_eq!(authored(output.stdout), r#""saída\n""#);
                assert_eq!(authored(output.stderr), r#""erro\t☕""#);
                assert_eq!(authored(output.code), "340282366920938463463374607431768211456");
                assert_eq!(authored(output.files), "[]");
                assert_eq!(axes.len(), 1);
                assert!(state.includes(&[7]));
                assert!(!state.includes(&[8]));
            }
        }

        let ordinary = prepare_scalar_project(
            vec![(declarations(&app, &failing), Vec::new())],
            vec![app],
            inputs(),
        );
        assert!(ordinary.is_err());
    }

    #[test]
    fn distinguishes_missing_invalid_unsupported_and_vacuous_obligations() {
        let app = source("src/main.w", "app");
        assert_eq!(check(&app, &script(""), "missing"), ProjectCheck::MissingRoot);
        assert_eq!(check(&app, &script(""), "proof"), ProjectCheck::NoChecks);
        let unsupported = "%%start\nunit() effect = fn {};\nunit() proof = fn { effect(); };\n%%end";
        assert_eq!(check(&app, unsupported, "proof"), ProjectCheck::Unsupported);
        let invalid = script("core.require(true, 0);");
        assert!(matches!(check(&app, &invalid, "proof"), ProjectCheck::Invalid(_)));
        let dead = script(&format!("if (false) {{ core.require(false, {OUTPUT}); }};"));
        assert_eq!(check(&app, &dead, "proof"), ProjectCheck::Proved);
        assert_eq!(
            check_closed_project_callable(
                vec![(declarations(&app, &dead), Vec::new())],
                Vec::new(),
                inputs(),
                &app,
                "proof",
            ),
            ProjectCheck::RootSourceOutsideInitOrder
        );
        assert_eq!(
            check_closed_project_callable(
                vec![(declarations(&app, &dead), Vec::new())],
                vec![app.clone()],
                inputs(),
                &source("src/main.w", "wrong-revision"),
                "proof",
            ),
            ProjectCheck::MissingRoot
        );
    }
    #[test]
    fn compiler_test_dependency_accepts_only_the_explicit_proof_fragment_entry() {
        use wosy_syntax::{parse_proof_fragment, SyntaxKind};

        let identity = source("src/main.w", "proof-fragment");
        let fragment = "proof clamp_bounded for clamp(x) -> (out) where (x <= 10) ensures (out <= 10) {}";
        let parsed = parse_proof_fragment(identity.clone(), fragment.to_owned(), &[]);
        assert!(parsed.is_valid(), "{:?}", parsed.errors);
        assert_eq!(parsed.canonical_cst().source, identity);
        assert_eq!(parsed.reconstruct(), fragment);
        let declaration = parsed
            .canonical_cst()
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::ProofDecl)
            .expect("canonical fragment proof");
        assert!(declaration.children().any(|node| node.kind() == SyntaxKind::ProofWhere));
        assert!(declaration.children().any(|node| node.kind() == SyntaxKind::ProofEnsures));

        let ordinary = crate::parse_source(
            identity,
            format!("%%start\n{fragment}\n%%end"),
            &[],
        );
        assert!(!ordinary.diagnostics.is_empty());
        assert!(!ordinary
            .result
            .canonical_cst()
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::ProofDecl));
    }

}
