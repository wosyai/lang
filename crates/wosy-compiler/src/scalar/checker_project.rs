use super::{
    checker_directive::CheckerOperation,
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
            }
        }
    }
}

mod tests {
    use super::*;
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
        let failing = script(&format!("u8 local = 7; if (local == 7) {{}}; core.require(local == 8, {OUTPUT});"));
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
                let ProjectCheck::Failed { source, operation, directive, condition, state, axes } = result else {
                    panic!("expected exact failure, found {result:?}");
                };
                assert_eq!(source, app);
                assert_eq!(operation, CheckerOperation::Require);
                assert_eq!(&failing[condition.start as usize..condition.end as usize], "local == 8");
                assert_eq!(
                    &failing[directive.start as usize..directive.end as usize],
                    format!("core.require(local == 8, {OUTPUT})")
                );
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
}
