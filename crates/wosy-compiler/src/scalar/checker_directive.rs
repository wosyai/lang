use std::collections::{hash_map::Entry, HashMap};

use num_bigint::BigInt;

use super::{
    checker_flow::CheckerPredicate, control::CheckedCallSignature, symbolic_integer::IntegerRelation, typecheck::expression_span,
    ByteSpan, ReferenceBindingId, ScalarExpression, ScalarType,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CheckerOperation {
    Require,
    Narrow,
}

impl CheckerOperation {
    pub(super) fn rejects(self, any_passing: bool, any_failing: bool) -> bool {
        match self {
            Self::Require => any_failing,
            Self::Narrow => !any_passing,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct PreparedCheckerDirective {
    pub(super) operation: CheckerOperation,
    pub(super) condition: ByteSpan,
    pub(super) predicate: Option<CheckerPredicate>,
    pub(super) initializer_predicate: Option<InitializerPredicate>,
    pub(super) call_signature: Option<CheckedCallSignature>,
    pub(super) output: CheckedLiteralOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InitializerPredicate {
    #[cfg(feature = "internal-checker")]
    Opaque,
    #[cfg(feature = "internal-checker")]
    And(Box<[Self; 2]>),
    #[cfg(feature = "internal-checker")]
    Or(Box<[Self; 2]>),
    #[cfg(feature = "internal-checker")]
    Not(Box<Self>),
    Boolean(bool),
    BooleanStorage(ByteSpan),
    IntegerComparison {
        relation: IntegerRelation,
        operands: Box<[InitializerIntegerOperand; 2]>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InitializerIntegerOperand {
    Storage(ByteSpan),
    Literal(BigInt),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckedLiteralOutput {
    pub(super) stdout: ByteSpan,
    pub(super) stderr: ByteSpan,
    pub(super) code: ByteSpan,
    pub(super) files: ByteSpan,
    pub(super) stdout_bytes: Box<[u8]>,
    pub(super) stderr_bytes: Box<[u8]>,
    pub(super) code_value: CheckerCode,
    pub(super) text_files: Box<[ScalarCheckerTextFile]>,
    pub(super) state_files: Box<[CheckerStateFile]>,
    #[cfg(feature = "internal-checker")]
    pub(super) has_module_values: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum CheckerScalarTransform {
    Identity,
    #[cfg(feature = "internal-checker")]
    LogicalNot,
    #[cfg(feature = "internal-checker")]
    Negate,
}

impl CheckerScalarTransform {
    fn select(expression: &ScalarExpression) -> (&ScalarExpression, Self) {
        #[cfg(feature = "internal-checker")]
        if let ScalarExpression::Unary { operator, operand, .. } = expression {
            if matches!(operand.as_ref(), ScalarExpression::Name { .. } | ScalarExpression::Contextual { .. } | ScalarExpression::Member { .. } | ScalarExpression::IndexedRead { .. }) {
                let transform = match operator {
                    super::UnaryOperator::LogicalNot => Self::LogicalNot,
                    super::UnaryOperator::Negate => Self::Negate,
                    _ => return (expression, Self::Identity),
                };
                return (operand, transform);
            }
        }
        (expression, Self::Identity)
    }

    fn accepts(self, #[cfg_attr(not(feature = "internal-checker"), allow(unused_variables))] ty: &CheckerStateType) -> bool {
        match self {
            Self::Identity => true,
            #[cfg(feature = "internal-checker")]
            Self::LogicalNot => matches!(ty, CheckerStateType::Scalar(ScalarType::Bool)),
            #[cfg(feature = "internal-checker")]
            Self::Negate => matches!(ty, CheckerStateType::Scalar(
                ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerCode {
    Literal(BigInt),
    Lexical(std::rc::Rc<str>, ScalarType, CheckerScalarTransform),
    #[cfg(feature = "internal-checker")]
    Binding(std::rc::Rc<ReferenceBindingId>, ScalarType, CheckerScalarTransform),
    #[cfg(feature = "internal-checker")]
    Module(ByteSpan, ScalarType, CheckerScalarTransform),
    #[cfg(feature = "internal-checker")]
    Projection(CheckerStateSelection, ScalarType, CheckerScalarTransform),
}

impl CheckerCode {
    #[cfg(any(test, feature = "internal-checker"))]
    pub(super) fn literal(&self) -> Option<&BigInt> {
        match self {
            Self::Literal(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "internal-checker"), allow(dead_code))]
pub(super) enum CheckerStateSelection {
    Lexical(std::rc::Rc<str>),
    Binding(ReferenceBindingId),
    #[cfg(feature = "internal-checker")]
    Aggregate(std::rc::Rc<CheckerSnapshotShape>),
    #[cfg(feature = "internal-checker")]
    LexicalProjection { receiver: std::rc::Rc<str>, component: CheckerAggregateProjection },
    #[cfg(feature = "internal-checker")]
    AggregateLeaf { shape: std::rc::Rc<CheckerSnapshotShape>, leaf: usize },
    #[cfg(feature = "internal-checker")]
    Module(ByteSpan),
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerAggregateProjection {
    Nominal { owner: std::rc::Rc<super::ScalarStructId>, field: usize },
    FixedArray { length: u64, index: usize },
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckerSnapshotShape {
    pub(super) has_affine_leaf: bool,
    pub(super) root: CheckerSnapshotNode,
    pub(super) leaves: Box<[CheckerSnapshotLeaf]>,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerSnapshotNode {
    Nominal {
        name: Box<str>,
        fields: Box<[(Box<str>, CheckerSnapshotNode)]>,
    },
    Array(Box<[CheckerSnapshotNode]>),
    Leaf(usize),
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckerSnapshotLeaf {
    pub(super) span: super::ByteSpan,
    pub(super) ty: ScalarType,
    pub(super) capture: ReferenceBindingId,
    pub(super) operand: CheckerSnapshotOperand,
}

#[cfg(feature = "internal-checker")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerSnapshotOperand {
    Binding(ReferenceBindingId),
    Literal(BigInt),
    Affine {
        source: ReferenceBindingId,
        operator: super::BinaryOperator,
        literal: BigInt,
        source_on_left: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CheckerStateType {
    Scalar(ScalarType),
    #[cfg(feature = "internal-checker")]
    Nominal(std::rc::Rc<super::ScalarStructId>),
    #[cfg(feature = "internal-checker")]
    FixedArray,
}

#[cfg(feature = "internal-checker")]
pub(super) fn direct_array_index(expression: &ScalarExpression) -> Option<(&str, &ScalarExpression)> {
    use super::ScalarPlace;
    let ScalarExpression::IndexedRead { place: ScalarPlace::Index { base, index, .. }, .. } = expression
        else { return None; };
    let ScalarPlace::Name { name, .. } = base.as_ref() else { return None; };
    Some((name, index))
}

#[cfg(feature = "internal-checker")]
pub(super) fn array_index_chain(expression: &ScalarExpression) -> Option<(&str, Vec<&ScalarExpression>)> {
    use super::ScalarPlace;
    let ScalarExpression::IndexedRead { place, .. } = expression else { return None; };
    let mut place = place;
    // The syntax tree peels the last bracket first.
    let mut indices = Vec::new();
    while let ScalarPlace::Index { base, index, .. } = place {
        if indices.len() == 8 || !matches!(index.as_ref(), ScalarExpression::Integer { .. } | ScalarExpression::Name { .. }) {
            return None;
        }
        indices.push(index.as_ref());
        place = base;
    }
    let ScalarPlace::Name { name, .. } = place else { return None; };
    (!indices.is_empty()).then_some((name, indices))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckerStateFile {
    pub(super) position: usize,
    pub(super) path: Box<str>,
    pub(super) path_span: ByteSpan,
    pub(super) state_span: ByteSpan,
    pub(super) selection: CheckerStateSelection,
    pub(super) transform: CheckerScalarTransform,
    pub(super) ty: CheckerStateType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScalarCheckerTextFile {
    pub path: Box<str>,
    pub path_span: ByteSpan,
    pub text_span: ByteSpan,
    pub text: Box<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[cfg_attr(not(test), allow(dead_code))]
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

#[derive(Default)]
#[doc(hidden)]
pub struct CheckedOutputTargets<'a> {
    children: HashMap<(usize, String), (usize, &'a str, bool)>,
}

impl<'a> CheckedOutputTargets<'a> {
    pub fn insert(&mut self, path: &'a str) -> bool {
        let mut parent = 0;
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            let is_file = components.peek().is_none();
            let next_id = self.children.len() + 1;
            match self.children.entry((parent, component.to_ascii_uppercase())) {
                Entry::Vacant(entry) => {
                    entry.insert((next_id, component, is_file));
                    parent = next_id;
                }
                Entry::Occupied(entry) if !is_file => {
                    let &(id, spelling, existing_is_file) = entry.get();
                    if existing_is_file || spelling != component {
                        return false;
                    }
                    parent = id;
                }
                Entry::Occupied(_) => return false,
            }
        }
        true
    }
}

impl CheckerOutput<'_> {
    pub(super) fn explicit_literal(&self) -> Option<CheckedLiteralOutput> {
        self.explicit_with_state(|_| None)
    }

    pub(super) fn explicit_with_state(
        &self, mut select: impl FnMut(&ScalarExpression) -> Option<(CheckerStateSelection, CheckerStateType)>,
    ) -> Option<CheckedLiteralOutput> {
        let ScalarExpression::Utf8 { span: stdout, value: stdout_bytes } = self.stdout? else {
            return None;
        };
        let ScalarExpression::Utf8 { span: stderr, value: stderr_bytes } = self.stderr? else {
            return None;
        };
        let code = self.code?;
        let code_value = match code {
            ScalarExpression::Integer { value, .. } => CheckerCode::Literal(value.clone()),
            _ => {
                let (operand, transform) = CheckerScalarTransform::select(code);
                let (selection, state_ty) = select(operand)?;
                if !transform.accepts(&state_ty) { return None; }
                let ty = match state_ty {
                    CheckerStateType::Scalar(ty) if super::typecheck::is_integer_type(&ty) => ty,
                    _ => return None,
                };
                match selection {
                    CheckerStateSelection::Lexical(name) => CheckerCode::Lexical(name, ty, transform),
                    #[cfg(feature = "internal-checker")]
                    selection @ CheckerStateSelection::LexicalProjection { .. } =>
                        CheckerCode::Projection(selection, ty, transform),
                    _ => return None,
                }
            }
        };
        let mut targets = CheckedOutputTargets::default();
        let mut text_files = Vec::new();
        let mut state_files = Vec::new();
        for (position, file) in self.files.as_ref()?.iter().enumerate() {
            let ScalarExpression::Utf8 { span: path_span, value: path_text } = file.path else { return None; };
            let path = std::str::from_utf8(path_text).ok()?;
            if !portable_relative_output_path(path) || !targets.insert(path) { return None; }
            match &file.content {
                CheckerFileContent::Text(ScalarExpression::Utf8 { span: text_span, value: text }) => {
                    std::str::from_utf8(text).ok()?;
                    text_files.push(ScalarCheckerTextFile {
                        path: path.into(), path_span: *path_span, text_span: *text_span, text: text.as_slice().into(),
                    });
                }
                CheckerFileContent::State(expression) => {
                    let (operand, transform) = CheckerScalarTransform::select(expression);
                    let (selection, ty) = select(operand)?;
                    if !transform.accepts(&ty) { return None; }
                    state_files.push(CheckerStateFile {
                        position, path: path.into(), path_span: *path_span,
                        state_span: expression_span(expression), selection, transform, ty,
                    });
                }
                _ => return None,
            }
        }
        Some(CheckedLiteralOutput {
            stdout: *stdout,
            stderr: *stderr,
            code: expression_span(code),
            files: self.files_literal?,
            stdout_bytes: stdout_bytes.as_slice().into(),
            stderr_bytes: stderr_bytes.as_slice().into(),
            code_value,
            text_files: text_files.into_boxed_slice(),
            state_files: state_files.into_boxed_slice(),
            #[cfg(feature = "internal-checker")]
            has_module_values: false,
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

const MAX_PORTABLE_OUTPUT_PATH_BYTES: usize = 4096;
const MAX_PORTABLE_OUTPUT_COMPONENT_BYTES: usize = 255;
const MAX_PORTABLE_OUTPUT_COMPONENTS: usize = 64;

fn portable_relative_output_path(path: &str) -> bool {
    if path.len() > MAX_PORTABLE_OUTPUT_PATH_BYTES || !path.is_ascii() {
        return false;
    }
    path.split('/').enumerate().all(|(depth, component)| {
        if depth >= MAX_PORTABLE_OUTPUT_COMPONENTS
            || component.is_empty() || component.len() > MAX_PORTABLE_OUTPUT_COMPONENT_BYTES
            || component.ends_with(['.', ' '])
            || component.bytes().any(|b| b.is_ascii_control() || matches!(b, b'\\' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*'))
        {
            return false;
        }
        let stem = component.split('.').next().expect("component has a first segment").trim_end_matches(' ');
        let upper = stem.to_ascii_uppercase();
        let device = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$")
            || upper.strip_prefix("COM").or_else(|| upper.strip_prefix("LPT")).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
        let short_alias = stem.rsplit_once('~').is_some_and(|(_, suffix)| {
            !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
        });
        !device && !short_alias
    })
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
    use crate::scalar::{derive_scalar_declarations_from_cst, ScalarBlockItem, ScalarItem, ScalarType};
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
        #[cfg(feature = "internal-checker")]
        {
            let receipts = &body.structure.checker_ordinary_directive_calls;
            assert_eq!(receipts.len(), 1);
            let (&ordinal, receipt) = receipts.iter().next().unwrap();
            assert!(receipt.operands.is_empty());
            assert!(matches!(directives[&ordinal].operation, CheckerOperation::Narrow));
            assert!(body.resolved_call_site(ordinal).is_none());
        }
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
        let CheckerPredicate::Integer(branch) = &body.structure.checker_branches[condition_ordinal] else {
            panic!("expected typed integer branch");
        };
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
        let Some(CheckerPredicate::Integer(directive_predicate)) = &branch_directive.predicate else {
            panic!("expected typed integer directive");
        };
        assert_eq!(branch.binding, directive_predicate.binding);
        assert_eq!(branch_directive.condition, expression_span(syntax.condition));
        assert_eq!(branch_directive.output, syntax.output.explicit_literal().unwrap());

        let after_join = directive_for(body.structure.root, 1);
        let failure = check_integer_requirements(body).unwrap().unwrap_err();
        assert_eq!(&body.structure.checker_directives[&failure.directive], after_join);
        for value in 0..=255 {
            assert_eq!(failure.state.includes(&[value]), value <= 10);
        }
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
                for value in 0..=255 {
                    assert_eq!(failure.state.includes(&[value]), value >= 11);
                }
            },
        );
    }

    #[test]
    fn literal_assignment_overwrites_the_previous_range() {
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
            |body| {
                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                for value in 0..=255 {
                    assert_eq!(failure.state.includes(&[value]), value == 5);
                }
            },
        );
    }

    #[test]
    fn joins_preserve_exact_relations_between_distinct_integer_parameters() {
        use crate::scalar::checker_flow::check_integer_requirements;
        let prefix = r#"
            u8(u8, u8) check_pair = fn(x, y) {
                if (x > 0) {
                    core.narrow(y > 0, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                }
        "#;
        let guarded = r#"
                if (x > 0) {
                    core.require(y > 0, { .stdout = ""; .stderr = "guarded"; .code = 1; .files = []; });
                }
        "#;
        let unconditional = r#"
                core.require(y > 0, { .stdout = ""; .stderr = "unconditional"; .code = 1; .files = []; });
        "#;
        let suffix = "x };";
        check_source(&[prefix, guarded, suffix].concat(), |body| {
            let bindings = body.boundary.checker_parameter_bindings.as_deref().unwrap();
            assert_ne!(bindings[0], bindings[1]);
            assert_eq!(check_integer_requirements(body), Some(Ok(())));
        });
        check_source(&[prefix, guarded, unconditional, suffix].concat(), |body| {
            let failure = check_integer_requirements(body).unwrap().unwrap_err();
            assert!(failure.state.includes(&[0, 0]));
            assert!(!failure.state.includes(&[1, 0]));
            assert!(!failure.state.includes(&[0, 1]));
        });
    }

    #[test]
    fn independent_no_op_branches_rejoin_as_one_exact_row() {
        use crate::scalar::checker_flow::check_integer_requirements;

        let width = 12;
        let parameters = (0..width).map(|index| format!("p{index}")).collect::<Vec<_>>();
        let types = vec!["u8"; width].join(", ");
        let signature = parameters.join(", ");
        let mut source = format!("u8({types}) check_many = fn({signature}) {{\n");
        let output = r#"{ .stdout = ""; .stderr = "failed"; .code = 1; .files = []; }"#;
        for name in &parameters {
            source.push_str(&format!("if ({name} > 0) {{ core.require({name} > 0, {output}); }}\n"));
        }
        source.push_str(&format!("core.require(p0 > 0, {output});\np0 }};"));
        check_source(&source, |body| {
            let failure = check_integer_requirements(body).unwrap().unwrap_err();
            assert_eq!(failure.state.row_count(), 1);
            let mut input = vec![255; width];
            assert!(!failure.state.includes(&input));
            input[0] = 0;
            assert!(failure.state.includes(&input));
            assert!(failure.state.includes(&vec![0; width]));
        });
    }

    #[test]
    fn coalescing_an_unrelated_axis_keeps_existing_correlations() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8, u8, u8) check_three = fn(x, y, z) {
                if (x > 0) {
                    core.narrow(y > 0, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                }
                if (z > 0) {
                    core.require(z > 0, { .stdout = ""; .stderr = "guarded"; .code = 1; .files = []; });
                }
                core.require(z > 0, { .stdout = ""; .stderr = "unconditional"; .code = 1; .files = []; });
                x
            };
            "#,
            |body| {
                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                assert_eq!(failure.state.row_count(), 2);
                assert!(failure.state.includes(&[0, 0, 0]));
                assert!(failure.state.includes(&[1, 1, 0]));
                assert!(!failure.state.includes(&[1, 0, 0]));
                assert!(!failure.state.includes(&[1, 1, 1]));
            },
        );
    }

    #[test]
    fn assignment_keeps_correlations_between_other_fixed_width_bindings() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8, u8, u8) check_three = fn(x, y, z) {
                if (x > 0) {
                    core.narrow(y > 0, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                }
                z = 5;
                if (x > 0) {
                    core.require(y > 0, { .stdout = ""; .stderr = "guarded"; .code = 1; .files = []; });
                }
                core.require(z == 5, { .stdout = ""; .stderr = "assigned"; .code = 1; .files = []; });
                x
            };
            "#,
            |body| assert_eq!(check_integer_requirements(body), Some(Ok(()))),
        );
    }

    #[test]
    fn nonliteral_assignment_is_not_proved_using_stale_facts() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8, u8) check_pair = fn(value, another) {
                core.narrow(value >= 11, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                value = another;
                core.require(value >= 11, { .stdout = ""; .stderr = "changed"; .code = 1; .files = []; });
                value
            };
            "#,
            |body| assert_eq!(check_integer_requirements(body), None),
        );
    }

    #[test]
    fn sibling_integer_locals_have_distinct_ids_and_leave_no_axis_at_the_join() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8, u8) check_pair = fn(x, y) {
                if (x > 0) {
                    u8 local = 1;
                    core.require(local == 1, { .stdout = ""; .stderr = "first local"; .code = 1; .files = []; });
                    core.narrow(y > 0, { .stdout = ""; .stderr = "narrow"; .code = 1; .files = []; });
                }
                if (x == 0) {
                    u8 local = 2;
                    core.require(local == 2, { .stdout = ""; .stderr = "second local"; .code = 1; .files = []; });
                }
                if (x > 0) {
                    core.require(y > 0, { .stdout = ""; .stderr = "outer correlation"; .code = 1; .files = []; });
                }
                core.require(y > 0, { .stdout = ""; .stderr = "after join"; .code = 1; .files = []; });
                x
            };
            "#,
            |body| {
                let locals = body.structure.checker_integer_locals.values().collect::<Vec<_>>();
                assert_eq!(locals.len(), 2);
                assert_ne!(locals[0], locals[1]);

                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                assert_eq!(failure.axes, body.boundary.checker_parameter_bindings.as_deref().unwrap().iter().map(|binding| (binding, &ScalarType::U8)).collect::<Vec<_>>());
                assert!(failure.state.includes(&[0, 0]));
                assert!(!failure.state.includes(&[1, 0]));
            },
        );
    }

    #[test]
    fn failure_inside_branch_retains_current_local_for_author_output() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8) check_value = fn(value) {
                if (value > 0) {
                    u8 local = 7;
                    core.require(local < 7, { .stdout = ""; .stderr = "local failure"; .code = 1; .files = []; });
                }
                value
            };
            "#,
            |body| {
                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                let local = body.structure.checker_integer_locals.values().next().unwrap();
                assert_eq!(failure.axes, [(&body.boundary.checker_parameter_bindings.as_deref().unwrap()[0], &ScalarType::U8), (local, &ScalarType::U8)]);
                assert!(failure.state.includes(&[1, 7]));
                assert!(!failure.state.includes(&[0, 7]));
                assert!(!failure.state.includes(&[1, 6]));
            },
        );
    }

    #[test]
    fn literal_local_overwrite_reports_the_current_value() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            u8(u8) check_value = fn(value) {
                u8 local = 1;
                local = 2;
                core.require(local == 1, { .stdout = ""; .stderr = "mutated local"; .code = 1; .files = []; });
                value
            };
            "#,
            |body| {
                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                let local = body.structure.checker_integer_locals.values().next().unwrap();
                assert_eq!(failure.axes[1], (local, &ScalarType::U8));
                assert!(failure.state.includes(&[0, 2]));
                assert!(failure.state.includes(&[255, 2]));
                assert!(!failure.state.includes(&[0, 1]));
                assert!(!failure.state.includes(&[255, 1]));
            },
        );
    }

    #[test]
    fn checker_mode_does_not_allow_shadowing_a_visible_ancestor() {
        let project = checker_project(
            r#"
            u8(u8) check_value = fn(value) {
                if (value > 0) {
                    u8 value = 7;
                    core.require(value == 7, { .stdout = ""; .stderr = "shadow"; .code = 1; .files = []; });
                }
                value
            };
            "#,
        );
        let (rejected, _) = crate::scalar::typecheck_scalar_project_in_mode(project, true);
        assert!(
            rejected.diagnostics.iter().any(|diagnostic| diagnostic.code == "B0002"
                && diagnostic.message == "duplicate declaration"),
            "{:?}",
            rejected.diagnostics,
        );
    }

    #[test]
    fn closed_checker_root_has_one_initial_state_and_local_facts() {
        use crate::scalar::checker_flow::check_integer_requirements;
        for (condition, succeeds) in [("local == 7", true), ("local != 7", false)] {
            let output = r#"{ .stdout = ""; .stderr = "closed failure"; .code = 1; .files = []; }"#;
            let source =
                format!("unit() check_value = fn {{ u8 local = 7; core.require({condition}, {output}); }};");
            check_source(&source, |body| {
                assert_eq!(body.boundary.checker_parameter_bindings.as_deref(), Some([].as_slice()));
                let result = check_integer_requirements(body).unwrap();
                if succeeds {
                    assert_eq!(result, Ok(()));
                } else {
                    let failure = result.unwrap_err();
                    let local = body.structure.checker_integer_locals.values().next().unwrap();
                    assert_eq!(failure.axes, [(local, &ScalarType::U8)]);
                    assert_eq!(failure.state.row_count(), 1);
                    assert!(failure.state.includes(&[7]));
                    assert!(!failure.state.includes(&[0]));
                }
            });
        }
    }

    #[test]
    fn closed_checker_failure_retains_only_live_local_axes() {
        use crate::scalar::checker_flow::check_integer_requirements;
        check_source(
            r#"
            unit() check_value = fn {
                u8 outer = 7;
                if (outer == 7) {
                    u8 expired = 2;
                    core.require(expired == 2, { .stdout = ""; .stderr = "expired"; .code = 1; .files = []; });
                }
                if (outer == 7) {
                    u8 active = 3;
                    core.require(active == 4, { .stdout = ""; .stderr = "active"; .code = 1; .files = []; });
                }
                core.require(outer == 7, { .stdout = ""; .stderr = "outer"; .code = 1; .files = []; });
            };
            "#,
            |body| {
                let failure = check_integer_requirements(body).unwrap().unwrap_err();
                assert_eq!(body.structure.checker_integer_locals.len(), 3);
                assert_eq!(failure.axes.len(), 2);
                assert!(failure.state.includes(&[7, 3]));
                assert!(!failure.state.includes(&[7, 2]));
            },
        );
    }

    #[test]
    fn closed_root_unsupported_expressions_and_ordinary_preparation_are_not_success() {
        use crate::scalar::{checker_flow::check_integer_requirements, reference, typecheck_scalar_project_in_mode};
        for source in [
            r#"unit() check_value = fn {
                core.require(!false, { .stdout = ""; .stderr = "unsupported"; .code = 1; .files = []; });
            };"#,
            r#"
            unit() check_value = fn { effect(); };
            unit() effect = fn {};
            "#,
        ] {
            check_source(source, |body| assert_eq!(check_integer_requirements(body), None));
        }

        let project = checker_project("unit() ordinary = fn {};");
        let (checked, typing) = typecheck_scalar_project_in_mode(project, false);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let calls = reference::resolve_project_call_sites_with_occurrences(&checked.project.modules, typing);
        assert_eq!(calls.lookup_errors().count(), 0);
        let body = calls.function(0, 0);
        assert!(body.boundary.checker_parameter_bindings.is_none());
        assert_eq!(check_integer_requirements(body), None);
    }

    #[test]
    fn boolean_literals_partition_the_reachable_state_exactly() {
        use crate::scalar::checker_flow::check_integer_requirements;
        let output = r#"{ .stdout = ""; .stderr = "literal failure"; .code = 1; .files = []; }"#;
        for (operation, syntax) in [
            (CheckerOperation::Require, "require"),
            (CheckerOperation::Narrow, "narrow"),
        ] {
            for value in [true, false] {
                let source = format!(
                    "u8() check_value = fn {{ u8 local = 7; core.{syntax}({value}, {output}); local }};"
                );
                check_source(&source, |body| {
                    let result = check_integer_requirements(body).unwrap();
                    let directive = body.structure.checker_directives.values().next().unwrap();
                    assert_eq!(directive.predicate, Some(CheckerPredicate::Boolean(value)));
                    if value {
                        assert_eq!(result, Ok(()));
                    } else {
                        let failure = result.unwrap_err();
                        assert_eq!(body.structure.checker_directives[&failure.directive].operation, operation);
                        assert_eq!(failure.axes.len(), 1);
                        assert_eq!(failure.state.row_count(), 1);
                        assert!(failure.state.includes(&[7]));
                        assert!(!failure.state.includes(&[6]));
                    }
                });
            }
        }
    }

    #[test]
    fn constant_unit_branch_only_checks_its_reachable_arm() {
        use crate::scalar::checker_flow::check_integer_requirements;
        let output = r#"{ .stdout = ""; .stderr = "reached"; .code = 1; .files = []; }"#;
        for guard in [false, true] {
            let source = format!(
                r#"
                unit() check_value = fn {{
                    if ({guard}) {{
                        effect();
                        core.require(false, {output});
                    }}
                    core.require(true, {output});
                }};
                unit() effect = fn {{}};
                "#
            );
            check_source(&source, |body| {
                assert!(body.structure.checker_branches.values().any(|predicate| {
                    *predicate == CheckerPredicate::Boolean(guard)
                }));
                assert_eq!(check_integer_requirements(body), if guard { None } else { Some(Ok(())) });
            });
        }
    }

    #[test]
    fn checked_failure_retains_exact_literal_payload_and_source_spans() {
        use crate::scalar::checker_flow::check_integer_requirements;

        let source = r#"
            unit() check_value = fn {
                core.require(false, {
                    .stdout = "café\n☕";
                    .stderr = "erro\t\"";
                    .code = 340282366920938463463374607431768211456;
                    .files = [];
                });
            };
        "#;
        check_source(source, |body| {
            let failure = check_integer_requirements(body).unwrap().unwrap_err();
            let payload = &body.structure.checker_directives[&failure.directive].output;
            assert_eq!(payload.stdout_bytes.as_ref(), "café\n☕".as_bytes());
            assert_eq!(payload.stderr_bytes.as_ref(), b"erro\t\"");
            assert_eq!(payload.code_value.literal(), Some(&(BigInt::from(u128::MAX) + 1)));
            let offset = "%%start\n".len();
            let spelling = |span: ByteSpan| {
                &source[span.start as usize - offset..span.end as usize - offset]
            };
            assert_eq!(spelling(payload.stdout), r#""café\n☕""#);
            assert_eq!(spelling(payload.stderr), r#""erro\t\"""#);
            assert_eq!(spelling(payload.code), "340282366920938463463374607431768211456");
            assert_eq!(spelling(payload.files), "[]");
        });
    }

    #[test]
    fn output_payload_rejects_nonconcrete_or_uncontained_text_files_and_typed_state() {
        for source in [
            r#"core.require(false, { .stdout = write(); .stderr = ""; .code = 1; .files = []; })"#,
            r#"core.require(false, { .stdout = ""; .stderr = ""; .code = 1; })"#,
            r#"core.require(false, {
                .stdout = ""; .stderr = ""; .code = 1;
                .files = [{ .path = "out.txt"; .text = message; }];
            })"#,
            r#"core.require(false, {
                .stdout = ""; .stderr = ""; .code = 1;
                .files = [{ .path = "out.txt"; .state = 42; }];
            })"#,
            r#"core.require(false, {
                .stdout = ""; .stderr = ""; .code = 1;
                .files = [{ .path = "../out.txt"; .text = "test"; }];
            })"#,
            r#"core.require(false, {
                .stdout = ""; .stderr = ""; .code = 1;
                .files = [{ .path = "/out.txt"; .text = "test"; }];
            })"#,
            r#"core.require(false, {
                .stdout = ""; .stderr = ""; .code = 1;
                .files = [{ .path = "reports/day.txt"; .text = "a"; },
                          { .path = "./reports//day.txt"; .text = "b"; }];
            })"#,
        ] {
            with_expression(source, |expression| {
                let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
                assert!(directive.output.explicit_literal().is_none());
            });
        }
    }

    #[test]
    fn checker_file_paths_reject_portable_aliases_and_file_directory_conflicts() {
        use super::portable_relative_output_path;
        for path in [
            "", "/out.txt", "./out.txt", "../out.txt", "reports/../out.txt", "reports//day.txt",
            "reports/./day.txt", "reports/day.txt/", ".", "..", "out.txt.", "out.txt ",
            r"C:\temp\out.txt", "C:/temp/out.txt", r"\\server\share\out.txt",
            "//server/share/out.txt", "report.txt:stream", "nul.txt", "COM¹.txt", "LPT9.out",
            "a/ConIn$.txt", "aux .out", "REPORT~1.TXT", "a\0b.txt", "a?b.txt",
            "réports/day.txt", "re\u{301}ports/day.txt",
        ] {
            assert!(!portable_relative_output_path(path), "path {path:?}");
        }
        for path in ["reports/day.txt", "README", ".hidden/report.txt", "February/report.txt"] {
            assert!(portable_relative_output_path(path), "path {path:?}");
        }
        assert!(portable_relative_output_path(&"a".repeat(255)));
        assert!(!portable_relative_output_path(&"a".repeat(256)));
        assert!(portable_relative_output_path(&["a"; 64].join("/")));
        assert!(!portable_relative_output_path(&["a"; 65].join("/")));
        assert!(portable_relative_output_path(&vec!["a".repeat(255); 16].join("/")));
        assert!(!portable_relative_output_path(&vec!["a".repeat(255); 17].join("/")));
        for (first, second) in [
            ("report.txt", "REPORT.TXT"), ("Reports/one.txt", "reports/two.txt"),
            ("a", "a/b.txt"), ("a/b.txt", "a"), ("A", "a/b.txt"),
        ] {
            let source = format!(r#"core.require(false, {{
                .stdout = ""; .stderr = ""; .code = 1;
                .files = [{{ .path = "{first}"; .text = ""; }},
                          {{ .path = "{second}"; .text = ""; }}];
            }})"#);
            with_expression(&source, |expression| {
                let directive = CheckerDirective::from_expression(expression).unwrap().unwrap();
                assert!(directive.output.explicit_literal().is_none(), "{first:?} vs {second:?}");
            });
        }
    }

    #[test]
    fn observes_typed_return_alias_after_narrowing() {
        use crate::scalar::checker_flow::{observe_integer_returns, IntegerReturn};

        check_source(
            r#"
            u8(u8) bounded_identity = fn(value) {
                core.narrow(value <= 10, { .stdout = ""; .stderr = "range"; .code = 1; .files = []; });
                value
            };
            "#,
            |body| {
                let mut returns = observe_integer_returns(body).unwrap().unwrap();
                assert_eq!(returns.len(), 1);
                let returned = returns.pop().unwrap();
                let parameter = &body.boundary.checker_parameter_bindings.as_deref().unwrap()[0];
                assert_eq!(returned.value, IntegerReturn::Alias(parameter));
                assert_eq!(returned.ty, &crate::ScalarType::U8);
                assert_eq!(returned.axes, [parameter]);
                for value in 0..=255 {
                    assert_eq!(returned.state.includes(&[value]), value <= 10);
                }
            },
        );
    }

    #[test]
    fn observes_typed_constant_and_refuses_effectful_return_values() {
        use crate::scalar::checker_flow::{observe_integer_returns, IntegerReturn};
        check_source("u8() ten = fn { 10 };", |body| {
            let mut returns = observe_integer_returns(body).unwrap().unwrap();
            assert_eq!(returns.len(), 1);
            let returned = returns.pop().unwrap();
            assert_eq!(returned.value, IntegerReturn::Const(&BigInt::from(10)));
            assert_eq!(returned.ty, &crate::ScalarType::U8);
            assert!(returned.axes.is_empty());
            assert!(returned.state.includes(&[]));
        });
        check_source(
            "u8() ignored = fn { effect() }; u8() effect = fn { 10 };",
            |body| assert_eq!(observe_integer_returns(body), None),
        );
    }

    #[test]
    fn tail_value_conditional_preserves_disjoint_constant_and_alias_returns() {
        use crate::scalar::{
            checker_flow::{check_integer_return_bound, observe_integer_returns, IntegerReturn},
            symbolic_integer::IntegerRelation,
        };
        use crate::BinaryOperator;

        check_source(
            "u8(u8) clamp_bounded = fn(value) { if (value > 10) { 10 } else { value } };",
            |body| {
                let paths = observe_integer_returns(body).unwrap().unwrap();
                let [high, low] = paths.as_slice() else {
                    panic!("expected two disjoint return paths");
                };
                let parameter = &body.boundary.checker_parameter_bindings.as_deref().unwrap()[0];
                assert_eq!(high.value, IntegerReturn::Const(&BigInt::from(10)));
                assert_eq!(low.value, IntegerReturn::Alias(parameter));
                let upper = BinaryOperator::LessEqual;
                let relation = IntegerRelation::from_operator(&upper).unwrap();
                let ten = BigInt::from(10);
                assert_eq!(low.binding_satisfies(parameter, relation, &ten), Some(true));
                assert_eq!(high.binding_satisfies(parameter, relation, &ten), Some(false));
                assert_eq!(check_integer_return_bound(body, &upper, &ten), Some(Ok(true)));
                assert_eq!(check_integer_return_bound(body, &upper, &9.into()), Some(Ok(false)));
                for value in 0..=255 {
                    assert_eq!(high.state.includes(&[value]), value > 10);
                    assert_eq!(low.state.includes(&[value]), value <= 10);
                }
            },
        );
    }

    #[test]
    fn tail_value_conditional_skips_only_unreachable_unsupported_returns() {
        use crate::scalar::checker_flow::{check_integer_return_bound, observe_integer_returns, IntegerReturn};
        for guard in [false, true] {
            let source = format!(
                "u8(u8) select = fn(value) {{ if ({guard}) {{ effect() }} else {{ value }} }}; u8() effect = fn {{ 10 }};",
            );
            check_source(&source, |body| {
                let result = observe_integer_returns(body);
                if guard {
                    assert_eq!(result, None);
                    assert_eq!(
                        check_integer_return_bound(body, &crate::BinaryOperator::LessEqual, &10.into()),
                        None,
                    );
                } else {
                    let paths = result.unwrap().unwrap();
                    let [path] = paths.as_slice() else {
                        panic!("expected one reachable return path");
                    };
                    let parameter = &body.boundary.checker_parameter_bindings.as_deref().unwrap()[0];
                    assert_eq!(path.value, IntegerReturn::Alias(parameter));
                    assert!(path.state.includes(&[0]));
                    assert!(path.state.includes(&[255]));
                }
            });
        }
    }

    #[test]
    fn reachable_tail_arm_failure_is_not_a_partial_return_success() {
        use crate::scalar::checker_flow::check_integer_return_bound;
        check_source(
            r#"
            u8(u8) checked_clamp = fn(value) {
                if (value <= 10) {
                    value
                } else {
                    core.require(value >= 12, { .stdout = ""; .stderr = "eleven"; .code = 1; .files = []; });
                    10
                }
            };
            "#,
            |body| {
                let failure = check_integer_return_bound(body, &crate::BinaryOperator::LessEqual, &10.into())
                    .unwrap()
                    .unwrap_err();
                for value in 0..=255 {
                    assert_eq!(failure.state.includes(&[value]), value == 11);
                }
            },
        );
    }

    #[test]
    fn signed_callable_branch_and_requirement_share_the_same_exact_bound() {
        use crate::scalar::checker_flow::check_integer_requirements;
        for bound in [-5, -6] {
            let source = format!(r#"
                i8(i8) checked = fn(value) {{
                    if (value >= {bound}) {{
                        core.require(value >= -5, {{ .stdout = ""; .stderr = "below -5"; .code = 1; .files = []; }});
                    }};
                    value
                }};
            "#);
            check_source(&source, |body| {
                let checked = check_integer_requirements(body).unwrap();
                if bound == -5 {
                    assert_eq!(checked, Ok(()));
                } else {
                    let failure = checked.unwrap_err();
                    for value in -128..=127 {
                        assert_eq!(failure.state.includes_signed(&[value]), value == -6);
                    }
                }
            });
        }
    }

    #[test]
    fn where_bound_filters_only_the_selected_proof() {
        use crate::scalar::{
            checker_flow::{check_integer_return_bound, check_integer_return_bound_where, ProofCoverage},
            BinaryOperator,
        };
        check_source(
            "u8(u8) identity = fn(value) { if (value <= 9) {}; value };",
            |body| {
                let filter = body.structure.checker_branches.values().next().unwrap();
                assert_eq!(
                    check_integer_return_bound(body, &BinaryOperator::LessEqual, &9.into()),
                    Some(Ok(false)),
                );
                assert_eq!(
                    check_integer_return_bound_where(body, &BinaryOperator::LessEqual, &9.into(), std::slice::from_ref(filter)),
                    Some(Ok(ProofCoverage::Applicable(true))),
                );
                assert_eq!(
                    check_integer_return_bound_where(body, &BinaryOperator::LessEqual, &8.into(), std::slice::from_ref(filter)),
                    Some(Ok(ProofCoverage::Applicable(false))),
                );
                assert_eq!(
                    check_integer_return_bound(body, &BinaryOperator::LessEqual, &9.into()),
                    Some(Ok(false)),
                );
            },
        );
    }

    #[test]
    fn where_filter_uses_call_entry_before_a_parameter_overwrite() {
        use crate::scalar::{
            checker_flow::{check_integer_return_bound_where, ProofCoverage},
            BinaryOperator,
        };
        check_source(
            "u8(u8) overwrite = fn(value) { if (value > 10) {}; value = 0; value };",
            |body| {
                let filter = body.structure.checker_branches.values().next().unwrap();
                assert_eq!(
                    check_integer_return_bound_where(body, &BinaryOperator::Equal, &0.into(), std::slice::from_ref(filter)),
                    Some(Ok(ProofCoverage::Applicable(true))),
                );
            },
        );
    }

    #[test]
    fn empty_where_supplies_no_fact_and_local_is_not_an_input() {
        use crate::scalar::{
            checker_flow::{check_integer_return_bound, check_integer_return_bound_where, ProofCoverage},
            BinaryOperator,
        };
        check_source(
            "u8(u8) opaque = fn(value) { if (value > 255) {}; effect() }; u8() effect = fn { 1 };",
            |body| {
                let filter = body.structure.checker_branches.values().next().unwrap();
                assert_eq!(check_integer_return_bound(body, &BinaryOperator::Equal, &0.into()), None);
                assert_eq!(
                    check_integer_return_bound_where(body, &BinaryOperator::Equal, &0.into(), std::slice::from_ref(filter)),
                    Some(Ok(ProofCoverage::NoApplicableInputs)),
                );
            },
        );
        check_source(
            "u8(u8) local_guard = fn(value) { u8 local = 7; if (local == 7) {}; value };",
            |body| {
                let filter = body.structure.checker_branches.values().next().unwrap();
                assert_eq!(
                    check_integer_return_bound_where(body, &BinaryOperator::LessEqual, &255.into(), std::slice::from_ref(filter)),
                    None,
                );
            },
        );
    }

}
