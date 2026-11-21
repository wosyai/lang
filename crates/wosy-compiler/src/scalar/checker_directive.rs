use super::{
    checker_flow::IntegerPredicate, control::CheckedCallSignature, typecheck::expression_span,
    ByteSpan, ScalarExpression,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckerOperation {
    Require,
    Narrow,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PreparedCheckerDirective {
    pub(super) operation: CheckerOperation,
    pub(super) condition: ByteSpan,
    pub(super) integer_predicate: Option<IntegerPredicate>,
    pub(super) call_signature: Option<CheckedCallSignature>,
    pub(super) output: CheckedLiteralOutput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CheckedLiteralOutput {
    pub(super) stdout: ByteSpan,
    pub(super) stderr: ByteSpan,
    pub(super) code: ByteSpan,
    pub(super) files: ByteSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CheckedDirectiveMarker {
    pub(super) operation: CheckerOperation,
    pub(super) output: CheckedLiteralOutput,
}

pub(super) struct CheckerDirective<'a> {
    pub(super) operation: CheckerOperation,
    pub(super) condition: &'a ScalarExpression,
    pub(super) output: CheckerOutput<'a>,
}

pub(super) struct CheckerOutput<'a> {
    pub(super) stdout: Option<&'a ScalarExpression>,
    pub(super) stderr: Option<&'a ScalarExpression>,
    pub(super) code: Option<&'a ScalarExpression>,
    pub(super) files: Option<Vec<CheckerFileOutput<'a>>>,
    files_literal: Option<ByteSpan>,
}

pub(super) struct CheckerFileOutput<'a> {
    pub(super) path: &'a ScalarExpression,
    pub(super) content: CheckerFileContent<'a>,
}

pub(super) enum CheckerFileContent<'a> {
    State(&'a ScalarExpression),
    Text(&'a ScalarExpression),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CheckerDirectiveShapeError {
    pub(super) span: ByteSpan,
    pub(super) kind: CheckerDirectiveShapeErrorKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckerDirectiveShapeErrorKind {
    CallShape,
    ExpectedRecord,
    ExpectedList,
    UnknownField,
    DuplicateField,
    MissingPath,
    ExpectedOneContent,
}

impl<'a> CheckerDirective<'a> {
    pub(super) fn from_expression(
        expression: &'a ScalarExpression,
    ) -> Option<Result<Self, CheckerDirectiveShapeError>> {
        let ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            span,
            ..
        } = expression
        else {
            return None;
        };
        let operation = match (receiver.as_deref(), name.as_str()) {
            (Some("core"), "require") => CheckerOperation::Require,
            (Some("core"), "narrow") => CheckerOperation::Narrow,
            _ => return None,
        };
        let [condition, output] = arguments.as_slice() else {
            return Some(Err(error(*span, CheckerDirectiveShapeErrorKind::CallShape)));
        };
        if !type_arguments.is_empty() {
            return Some(Err(error(*span, CheckerDirectiveShapeErrorKind::CallShape)));
        }
        Some(CheckerOutput::from_expression(output).map(|output| Self {
            operation,
            condition,
            output,
        }))
    }
}

impl CheckerDirectiveShapeErrorKind {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::CallShape => "checker directive requires a condition and an output record",
            Self::ExpectedRecord => "checker directive requires a record literal",
            Self::ExpectedList => "checker output files require a list literal",
            Self::UnknownField => "unknown checker output field",
            Self::DuplicateField => "duplicate checker output field",
            Self::MissingPath => "checker output file requires a path",
            Self::ExpectedOneContent => "checker output file requires exactly one of state or text",
        }
    }
}

impl CheckerOutput<'_> {
    pub(super) fn explicit_literal(&self) -> Option<CheckedLiteralOutput> {
        let ScalarExpression::Utf8 { span: stdout, .. } = self.stdout? else {
            return None;
        };
        let ScalarExpression::Utf8 { span: stderr, .. } = self.stderr? else {
            return None;
        };
        let ScalarExpression::Integer { span: code, .. } = self.code? else {
            return None;
        };
        if !self.files.as_ref()?.is_empty() {
            return None;
        }
        Some(CheckedLiteralOutput {
            stdout: *stdout,
            stderr: *stderr,
            code: *code,
            files: self.files_literal?,
        })
    }
}

impl<'a> CheckerOutput<'a> {
    fn from_expression(expression: &'a ScalarExpression) -> Result<Self, CheckerDirectiveShapeError> {
        let [stdout, stderr, code, files] =
            record_fields(expression, ["stdout", "stderr", "code", "files"])?;
        let files_literal = files.map(expression_span);
        let files = files
            .map(|expression| {
                let ScalarExpression::ArrayLiteral { elements, .. } = expression else {
                    return Err(error(
                        expression_span(expression),
                        CheckerDirectiveShapeErrorKind::ExpectedList,
                    ));
                };
                elements.iter().map(CheckerFileOutput::from_expression).collect()
            })
            .transpose()?;
        Ok(Self { stdout, stderr, code, files, files_literal })
    }
}

impl<'a> CheckerFileOutput<'a> {
    fn from_expression(expression: &'a ScalarExpression) -> Result<Self, CheckerDirectiveShapeError> {
        let [path, state, text] = record_fields(expression, ["path", "state", "text"])?;
        let span = expression_span(expression);
        let path = path.ok_or_else(|| error(span, CheckerDirectiveShapeErrorKind::MissingPath))?;
        let content = match (state, text) {
            (Some(state), None) => CheckerFileContent::State(state),
            (None, Some(text)) => CheckerFileContent::Text(text),
            _ => return Err(error(span, CheckerDirectiveShapeErrorKind::ExpectedOneContent)),
        };
        Ok(Self { path, content })
    }
}

fn record_fields<'a, const N: usize>(
    expression: &'a ScalarExpression,
    names: [&str; N],
) -> Result<[Option<&'a ScalarExpression>; N], CheckerDirectiveShapeError> {
    use CheckerDirectiveShapeErrorKind::*;
    let ScalarExpression::StructLiteral { fields, .. } = expression else {
        return Err(error(expression_span(expression), ExpectedRecord));
    };
    let mut values = [None; N];
    for field in fields {
        let index = names
            .iter()
            .position(|name| *name == field.name.as_str())
            .ok_or_else(|| error(field.name_span, UnknownField))?;
        if values[index].replace(&field.value).is_some() {
            return Err(error(field.name_span, DuplicateField));
        }
    }
    Ok(values)
}

fn error(span: ByteSpan, kind: CheckerDirectiveShapeErrorKind) -> CheckerDirectiveShapeError {
    CheckerDirectiveShapeError { span, kind }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scalar::{derive_scalar_declarations_from_cst, ScalarBlockItem, ScalarItem};
    use crate::parse_source;
    use wosy_syntax::SourceIdentity;

    fn with_expression<T>(source: &str, use_expression: impl FnOnce(&ScalarExpression) -> T) -> T {
        let parse = parse_source(
            SourceIdentity::new("local".into(), "demo".into(), "src/main.w".into(), "test".into()),
            format!("%%start\n{source};\n%%end"),
            &[],
        );
        assert!(parse.diagnostics.is_empty(), "{:?}", parse.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parse.result.canonical_cst());
        assert!(declarations.diagnostics.is_empty(), "{:?}", declarations.diagnostics);
        let [ScalarItem::Executable(ScalarBlockItem::Expression(expression))] =
            declarations.items.as_slice()
        else {
            panic!("expected one expression")
        };
        use_expression(expression)
    }

    #[test]
    fn canonical_requirement_preserves_output_expression_identities() {
        with_expression(
            r#"core.require(actual == expected, {
                .stdout = "";
                .stderr = "values diverged";
                .code = 1;
                .files = [
                    { .path = "state.json"; .state = value; },
                    { .path = "text.txt"; .text = message; }
                ];
            })"#,
            |expression| {
                let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
                assert_eq!(directive.operation, CheckerOperation::Require);
                assert!(matches!(directive.condition, ScalarExpression::Binary { .. }));
                assert!(matches!(directive.output.stdout, Some(ScalarExpression::Utf8 { value, .. }) if value.is_empty()));
                assert!(matches!(directive.output.stderr, Some(ScalarExpression::Utf8 { value, .. }) if value == b"values diverged"));
                assert!(matches!(directive.output.code, Some(ScalarExpression::Integer { value, .. }) if value == &num_bigint::BigInt::from(1)));
                let files = directive.output.files.unwrap();
                assert_eq!(files.len(), 2);
                assert!(matches!(files[0].path, ScalarExpression::Utf8 { value, .. } if value == b"state.json"));
                assert!(matches!(files[0].content, CheckerFileContent::State(ScalarExpression::Name { name, .. }) if name == "value"));
                assert!(matches!(files[1].content, CheckerFileContent::Text(ScalarExpression::Name { name, .. }) if name == "message"));
            },
        );
    }

    #[test]
    fn duplicate_output_fields_and_ambiguous_file_content_are_distinct() {
        use CheckerDirectiveShapeErrorKind::*;
        for (source, kind) in [
            (r#"core.narrow(true, { .stdout = ""; .stdout = ""; })"#, DuplicateField),
            (
                r#"core.narrow(true, { .files = [{ .path = "out"; .state = x; .text = ""; }]; })"#,
                ExpectedOneContent,
            ),
        ] {
            with_expression(source, |expression| {
                let result = CheckerDirective::from_expression(expression).unwrap();
                let Err(found) = result else { panic!("expected shape error") };
                assert_eq!(found.kind, kind);
            });
        }
    }

    #[test]
    fn empty_output_and_explicitly_empty_files_have_distinct_shapes() {
        with_expression("core.require(true, {})", |expression| {
            let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
            assert!(directive.output.stdout.is_none());
            assert!(directive.output.stderr.is_none());
            assert!(directive.output.code.is_none());
            assert!(directive.output.files.is_none());
        });
        with_expression("core.narrow(true, { .files = []; })", |expression| {
            let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
            assert!(directive.output.files.unwrap().is_empty());
        });
    }

    #[test]
    fn non_core_call_is_not_a_checker_directive() {
        with_expression("other.require(actual, expected)", |expression| {
            assert!(CheckerDirective::from_expression(expression).is_none());
        });
    }

    fn checker_project(source: &str) -> crate::ScalarProject {
        let source_id = SourceIdentity::new(
            "local".into(),
            "demo".into(),
            "src/main.w".into(),
            "test".into(),
        );
        let parse = parse_source(source_id.clone(), format!("%%start\n{source}\n%%end"), &[]);
        assert!(parse.diagnostics.is_empty(), "{:?}", parse.diagnostics);
        let declarations = derive_scalar_declarations_from_cst(parse.result.canonical_cst());
        assert!(declarations.diagnostics.is_empty(), "{:?}", declarations.diagnostics);
        let module = crate::ScalarModule::new(source_id.clone(), declarations.items, Vec::new());
        crate::ScalarProject::new(vec![module], vec![source_id])
    }

    #[test]
    fn check_only_conditions_have_canonical_types_but_no_runtime_call_edges() {
        use crate::scalar::reference::{self, OccurrenceParent};
        use crate::scalar::typecheck_scalar_project_in_mode;
        use crate::ScalarType;

        let project = checker_project(
            r#"
            bool() probe = fn { true };
            u8(u8) check_value = fn(value) {
                core.require(value > 10, { .stdout = ""; .stderr = "bound"; .code = 1; .files = []; });
                core.narrow(probe(), { .stdout = ""; .stderr = "probe"; .code = 1; .files = []; });
                value
            };
            "#,
        );
        let (checked, typing) = typecheck_scalar_project_in_mode(project, true);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let ScalarItem::Function(function) = &checked.project.modules[0].items[1] else {
            panic!("expected helper function")
        };
        let ScalarBlockItem::Expression(expression) = &function.body.items[0] else {
            panic!("expected directive")
        };
        let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
        let ScalarExpression::Binary { left, right, .. } = directive.condition else {
            panic!("expected scalar comparison")
        };
        assert_eq!(
            typing[0].checked_expression_type(directive.condition),
            Some(&ScalarType::Bool),
        );
        assert_eq!(typing[0].checked_expression_type(left), Some(&ScalarType::U8));
        assert_eq!(typing[0].checked_expression_type(right), Some(&ScalarType::U8));

        let ScalarBlockItem::Expression(expression) = &function.body.items[1] else {
            panic!("expected directive")
        };
        let narrow = CheckerDirective::from_expression(expression).unwrap().unwrap();
        let signature = typing[0]
            .checked_call_signature(narrow.condition)
            .expect("probe resolved as a checked call before runtime projection");
        assert!(signature.parameters.is_empty());
        let [output] = signature.outputs.outputs.as_slice() else {
            panic!("expected one probe output")
        };
        assert_eq!(output.ty, ScalarType::Bool);
        let expected_probe = signature.clone();

        let calls = reference::resolve_project_call_sites_with_occurrences(&checked.project.modules, typing);
        assert_eq!(calls.lookup_errors().count(), 0);
        let body = calls.function(0, 1);
        let directives = &body.structure.checker_directives;
        assert_eq!(directives.len(), 2);
        assert!(directives.values().any(|directive| directive.operation == CheckerOperation::Require));
        assert!(directives.values().any(|directive| directive.operation == CheckerOperation::Narrow));
        let retained_probe = directives.values()
            .find(|directive| directive.operation == CheckerOperation::Narrow)
            .expect("narrow directive");
        assert_eq!(retained_probe.call_signature.as_ref(), Some(&expected_probe));
        assert!(body.call_occurrences.is_empty());
        for occurrence in directives.keys() {
            assert_eq!(body.structure.checked_types.expressions.get(occurrence), Some(&ScalarType::Unit));
            assert!(!body.structure.edges.keys().any(|(parent, _)| *parent == OccurrenceParent::Expression(*occurrence)));
        }
    }

    #[test]
    fn checker_condition_does_not_bypass_standard_type_errors() {
        let project = checker_project(
            r#"
            unit() check_value = fn {
                core.require(10, { .stdout = ""; .stderr = "bound"; .code = 1; .files = []; });
            };
            "#,
        );
        let (rejected, _) = crate::scalar::typecheck_scalar_project_in_mode(project, true);
        let ScalarItem::Function(function) = &rejected.project.modules[0].items[0] else {
            panic!("expected helper function")
        };
        let ScalarBlockItem::Expression(expression) = &function.body.items[0] else {
            panic!("expected directive")
        };
        let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
        let span = expression_span(directive.condition);
        assert!(
            rejected.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B0003"
                    && diagnostic.labels.iter().any(|label| label.span.range == span)
            }),
            "{:?}",
            rejected.diagnostics,
        );
    }

    #[test]
    fn canonical_branch_refines_checker_requirement_without_leaking_past_join() {
        use crate::scalar::{
            checker_flow::check_integer_requirements,
            control::ConditionalRegion,
            reference::{self, OccurrenceEdge, OccurrenceNode, OccurrenceParent},
            typecheck_scalar_project_in_mode,
        };

        let project = checker_project(
            r#"
            u8(u8) check_value = fn(value) {
                if (value > 10) {
                    core.require(value >= 11, { .stdout = ""; .stderr = "branch"; .code = 1; .files = []; });
                }
                core.require(value >= 11, { .stdout = ""; .stderr = "join"; .code = 1; .files = []; });
                core.require(value <= 255, { .stdout = ""; .stderr = "type"; .code = 1; .files = []; });
                value
            };
            "#,
        );
        let (checked, typing) = typecheck_scalar_project_in_mode(project, true);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let module = &checked.project.modules[0];
        let ScalarItem::Function(function) = &module.items[0] else {
            panic!("expected function")
        };
        let ScalarBlockItem::Expression(ScalarExpression::UnitIf { then_branch, .. }) =
            &function.body.items[0]
        else {
            panic!("expected canonical branch")
        };
        let calls = reference::resolve_project_call_sites_with_occurrences(&checked.project.modules, typing);
        assert_eq!(calls.lookup_errors().count(), 0);
        let body = calls.function(0, 0);
        let region = body.conditionals.values().next().expect("branch topology");
        let ConditionalRegion::Unit { condition: condition_ordinal, then_block, .. } = region else {
            panic!("expected unit branch topology")
        };
        let branch = &body.structure.checker_integer_branches[condition_ordinal];
        let directive_for = |block, item| {
            let OccurrenceNode::Expression(ordinal) = body.structure.edges
                [&(OccurrenceParent::Block(block), OccurrenceEdge::ExpressionItem { item })]
            else {
                panic!("expected expression item")
            };
            &body.structure.checker_directives[&ordinal]
        };
        let branch_directive = directive_for(*then_block, 0);
        let ScalarBlockItem::Expression(canonical) = &then_branch.items[0] else {
            panic!("expected canonical directive")
        };
        let syntax = CheckerDirective::from_expression(canonical).unwrap().unwrap();
        assert_eq!(branch_directive.condition, expression_span(syntax.condition));
        assert_eq!(branch_directive.output, syntax.output.explicit_literal().unwrap());

        let (then_path, else_path) = branch.root().unwrap().split(branch).unwrap();
        assert!(!then_path.is_unreachable());
        assert!(!else_path.is_unreachable());
        let predicate = branch_directive.integer_predicate.as_ref().unwrap();
        let (passing, failing) = then_path.split(predicate).unwrap();
        assert!(!passing.is_unreachable());
        assert!(failing.is_unreachable());

        let joined = then_path.join(else_path).unwrap();
        let after_join = directive_for(body.structure.root, 1);
        let (passing, failing) = joined.split(after_join.integer_predicate.as_ref().unwrap()).unwrap();
        assert!(!passing.is_unreachable());
        assert!(!failing.is_unreachable());
        let expected_failing = failing;

        let type_range = directive_for(body.structure.root, 2);
        let (_, failing) = joined.split(type_range.integer_predicate.as_ref().unwrap()).unwrap();
        assert!(failing.is_unreachable());

        let failure = check_integer_requirements(body).unwrap().unwrap_err();
        assert_eq!(&body.structure.checker_directives[&failure.directive], after_join);
        assert_eq!(failure.inputs, expected_failing);
    }

    fn check_source<T>(source: &str, inspect: impl FnOnce(&crate::scalar::reference::FunctionCallSites) -> T) -> T {
        use crate::scalar::{reference, typecheck_scalar_project_in_mode};
        let (checked, typing) = typecheck_scalar_project_in_mode(checker_project(source), true);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let calls = reference::resolve_project_call_sites_with_occurrences(&checked.project.modules, typing);
        assert_eq!(calls.lookup_errors().count(), 0);
        inspect(calls.function(0, 0))
    }

    #[test]
    fn narrowing_restricts_continuation_and_unreachable_checks_are_vacuous() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8) check_value = fn(value) {
                core.narrow(value >= 11, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                if (value < 11) {
                    core.require(value >= 200, { .stdout = ""; .stderr = "unreachable"; .code = 1; .files = []; });
                }
                core.require(value >= 11, { .stdout = ""; .stderr = "reachable"; .code = 1; .files = []; });
                value
            };
            "#,
            |body| assert_eq!(check_integer_requirements(body), Some(Ok(()))),
        );
    }

    #[test]
    fn contradictory_narrowing_uses_its_entire_incoming_state() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8) check_value = fn(value) {
                core.narrow(value >= 11, { .stdout = ""; .stderr = "first"; .code = 1; .files = []; });
                core.narrow(value < 11, { .stdout = ""; .stderr = "contradiction"; .code = 1; .files = []; });
                value
            };
            "#,
            |body| {
                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                let directives = &body.structure.checker_directives;
                let failing_directive = &directives[&failure.directive];
                assert_eq!(failing_directive.operation, CheckerOperation::Narrow);
                let initial_directive = directives
                    .iter()
                    .min_by_key(|(ordinal, _)| ordinal.0)
                    .unwrap().1;
                assert_ne!(failing_directive, initial_directive);
                let first = initial_directive.integer_predicate.as_ref().unwrap();
                let (incoming, _) = first.root().unwrap().split(first).unwrap();
                assert_eq!(failure.inputs, incoming);
            },
        );
    }

    #[test]
    fn reachable_assignment_never_reuses_a_stale_range() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8) check_value = fn(value) {
                core.narrow(value >= 11, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                value = 5;
                core.require(value >= 11, { .stdout = ""; .stderr = "changed"; .code = 1; .files = []; });
                value
            };
            "#,
            |body| assert_eq!(check_integer_requirements(body), None),
        );
    }

}
