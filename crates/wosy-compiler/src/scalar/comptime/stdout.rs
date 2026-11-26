//! Typed stdout preparation and readback. No runner, re-entry, or verdict.
use super::{
    analyze_scalar_build, audited_closed_eager_binary, audited_closed_unary, collect_scalar_diagnostics,
    derive_block, prepare_scalar_project, root_receiver_and_initializer, single_binding_type, span_of,
    ClosedComptimeFragment, ClosedComptimeRejection, RootInitializer, ScalarBlockItem, ScalarExpression,
    ScalarItem, ScalarPreparationInputs, ScalarTargetLayout, ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
    FUNCTION,
};
use super::super::{
    derive_scalar_declarations_from_cst, derive_scalar_declarations_with_embedding,
    EmbeddedComptimeExpression, ScalarNamespaceBinding, ScalarProject, ScalarStructId, ScalarStructLiteralField,
};
use crate::{
    emit_scalar_build_project_artifact_llvm, emit_scalar_build_single_module_artifact_llvm, LlvmPartition,
    ScalarBlock, ScalarFunction, ScalarOutput, ScalarOutputSequence, UnaryOperator,
};
use std::{collections::HashMap, fmt, io::{self, Write}, sync::Arc};
use wosy_syntax::{ByteSpan, CanonicalCstRoot, ComptimeCst, FreshParseResult, SourceSpan, SyntaxKind};

// Bound recursive AST representation as well as the parser's stack.
const MAX_VALUE_DEPTH: usize = 64;

mod literal;
use literal::LiteralStdout;

#[derive(Debug)]
pub struct PreparedComptimeStdout {
    site: SourceSpan,
    result_type: ScalarType,
    shape: Arc<Shape>,
    literal: Option<LiteralStdout>,
}

#[derive(Debug)]
enum Shape {
    Bool,
    I32,
    U32,
    Utf8,
    Array { count: usize, element: Arc<Shape> },
    Struct(Vec<(String, Arc<Shape>)>),
}

impl Shape {
    fn scalar(ty: &ScalarType) -> Option<Self> {
        match ty {
            ScalarType::Bool => Some(Self::Bool),
            ScalarType::I32 => Some(Self::I32),
            ScalarType::U32 => Some(Self::U32),
            _ => None,
        }
    }
}

/// Prepare the expected output of one authored root comptime expression
/// using the ordinary project's resolved namespace and types. The entry
/// is the first item of the supplied canonical source graph, and all rows
/// are fresh compiler parser output, not caller-supplied result schemas.
/// The caller establishes the selected target/source authority. This is a
/// non-emitting preparation; it does not authorize a runner or substitution.
pub fn prepare_comptime_stdout(
    sources: Vec<(&FreshParseResult, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<wosy_syntax::SourceIdentity>,
    id: &str,
    layout: ScalarTargetLayout,
) -> Result<PreparedComptimeStdout, ClosedComptimeRejection> {
    prepare(sources, initialization_order, id, layout, false).map(|(output, _)| output)
}

/// The compiler-issued standalone evaluator and the stdout type from that
/// same ordinary preparation. It neither serializes an evaluated value
/// inside the module yet nor launches a builder, module, or runner.
pub struct PreparedComptimeStdoutEvaluator {
    output: PreparedComptimeStdout,
    llvm: LlvmPartition,
    layout: ScalarTargetLayout,
}

impl PreparedComptimeStdoutEvaluator {
    pub fn output(&self) -> &PreparedComptimeStdout { &self.output }
    pub fn llvm(&self) -> &LlvmPartition { &self.llvm }
    pub fn layout(&self) -> ScalarTargetLayout { self.layout }
}

/// Emit the actual selected body using ordinary compiler return lowering.
/// The detached evaluator supports a closed audited subset; unrelated
/// initializers/functions never enter its artifact.
pub fn prepare_comptime_stdout_evaluator(
    sources: Vec<(&FreshParseResult, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<wosy_syntax::SourceIdentity>,
    id: &str,
    layout: ScalarTargetLayout,
) -> Result<PreparedComptimeStdoutEvaluator, ClosedComptimeRejection> {
    let (output, llvm) = prepare(sources, initialization_order, id, layout, true)?;
    let llvm = llvm.ok_or_else(|| ClosedComptimeRejection::Emitter("prepared evaluator is missing".into()))?;
    Ok(PreparedComptimeStdoutEvaluator { output, llvm, layout })
}

fn prepare(
    sources: Vec<(&FreshParseResult, Vec<ScalarNamespaceBinding>)>,
    initialization_order: Vec<wosy_syntax::SourceIdentity>,
    id: &str,
    layout: ScalarTargetLayout,
    emit: bool,
) -> Result<(PreparedComptimeStdout, Option<LlvmPartition>), ClosedComptimeRejection> {
    use ClosedComptimeRejection as Error;
    if !matches!(layout, ScalarTargetLayout::NATIVE64 | ScalarTargetLayout::WASM32) {
        return Err(Error::UnsupportedLayout);
    }
    let Some((entry, _)) = sources.first() else { return Err(Error::InvalidParse); };
    let mut bytes = 0usize;
    for (fresh, _) in &sources {
        if !fresh.result().is_valid() { return Err(Error::InvalidParse); }
        bytes = bytes.saturating_add(fresh.result().text.len());
        if bytes > CLOSED_COMPTIME_MAX_SOURCE_BYTES { return Err(Error::SourceLimit); }
    }
    let parsed = (*entry).result();
    let qualify = |span| SourceSpan::new(parsed.source.clone(), span);
    let mut occurrences = parsed.root.descendants().filter_map(|node| {
        let site = ComptimeCst::from_node(&node)?;
        (site.id.text() == id).then_some((node, site))
    });
    let (node, site) = occurrences.next().ok_or(Error::SiteNotFound)?;
    if let Some((_, repeated)) = occurrences.next() { return Err(Error::RepeatedSite(qualify(repeated.span))); }
    let scope = qualify(site.span);
    let unsupported = |feature| Error::Unsupported { site: scope.clone(), feature };
    let RootInitializer { initializer, .. } = root_receiver_and_initializer(&node)
        .ok_or_else(|| unsupported("expected one explicit root binding"))?;
    for child in site.block.descendants() {
        match child.kind() {
            SyntaxKind::ComptimeExpr | SyntaxKind::Assignment =>
                return Err(unsupported("nested comptime or assignments are not admitted in this preparation")),
            SyntaxKind::LocalBinding if single_binding_type(&child).is_none() =>
                return Err(unsupported("a comptime local binding must have one typed value")),
            _ => {}
        }
    }
    let (diagnostics, _) = collect_scalar_diagnostics(
        &CanonicalCstRoot::new(parsed.source.clone(), site.block.clone()), None, |_| {},
    );
    if !diagnostics.is_empty() { return Err(Error::OrdinaryDiagnostics(diagnostics)); }
    let block = derive_block(&site.block);
    if block.final_expressions().count() != 1 || !matches!(block.items.last(), Some(ScalarBlockItem::Expression(_))) {
        return Err(unsupported("expected one final comptime value"));
    }
    if emit && !closed_evaluator_body(&block) {
        return Err(unsupported("not a closed authored scalar/aggregate evaluator body"));
    }
    let block_span = block.span;
    let value = ScalarExpression::Block(block);
    let modules = sources.into_iter().enumerate().map(|(index, (fresh, namespaces))| {
        let source = fresh.result().canonical_cst();
        let mut declarations = if index == 0 {
            derive_scalar_declarations_with_embedding(source, Some(EmbeddedComptimeExpression {
                site: &node, expression: &initializer, value: &value,
            }))
        } else {
            derive_scalar_declarations_from_cst(source)
        };
        if emit {
            // Retain the ordinary declaration context, moving the selected
            // body out of the exact binding rather than retaining another
            // project/AST copy or compiling arbitrary ambient runtime code.
            let mut root = None;
            for item in std::mem::take(&mut declarations.items) {
                match item {
                    ScalarItem::Namespace(_) => declarations.items.push(item),
                    ScalarItem::Binding(binding) if index == 0 && span_of(&binding.value) == block_span => {
                        if root.replace(binding).is_some() {
                            return Err(unsupported("ordinary comptime result binding is not unique"));
                        }
                    }
                    _ => {}
                }
            }
            if index == 0 {
                let binding = root.ok_or_else(|| unsupported("ordinary comptime result binding was not preserved"))?;
                let ScalarExpression::Block(body) = binding.value else {
                    return Err(unsupported("ordinary comptime evaluator body was not preserved"));
                };
                let (result, out) = (binding.declared_type, body.final_output_values[0].span);
                declarations.items.push(ScalarItem::Function(ScalarFunction {
                    name: FUNCTION.into(), name_span: super::super::token_span(&site.id), span: scope.range,
                    signature: ScalarType::Callable {
                        parameters: Vec::new(),
                        outputs: ScalarOutputSequence {
                            outputs: vec![ScalarOutput { ty: result, span: out }], span: out,
                        },
                    },
                    parameters: Vec::new(), parameter_spans: Vec::new(), generic_parameters: Vec::new(),
                    overload_arms: Vec::new(), body, reference_cfg: Default::default(),
                }));
            }
        }
        Ok((declarations, namespaces))
    }).collect::<Result<Vec<_>, _>>()?;
    drop(value);
    let prepared = prepare_scalar_project(modules, initialization_order, ScalarPreparationInputs { target_layout: layout })
        .map_err(|error| Error::OrdinaryDiagnostics(error.into_ordered_diagnostics()))?;
    let build = analyze_scalar_build(&prepared);
    if !build.diagnostics().is_empty() { return Err(Error::OrdinaryDiagnostics(build.diagnostics().to_vec())); }
    let project = prepared.project();
    let module = project.modules.first().filter(|module| module.source == parsed.source)
        .ok_or_else(|| unsupported("expected the selected ordinary module"))?;
    let (result_type, literal_body) = if emit {
        let function = module.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == FUNCTION && function.span == scope.range => Some(function),
            _ => None,
        }).ok_or_else(|| unsupported("ordinary comptime evaluator signature was not preserved"))?;
        let ScalarType::Callable { outputs, .. } = &function.signature else {
            return Err(unsupported("ordinary comptime evaluator signature was not preserved"));
        };
        let [result] = outputs.outputs.as_slice() else {
            return Err(unsupported("expected one typed comptime evaluator result"));
        };
        (&result.ty, None)
    } else {
        let mut targets = module.items.iter().filter_map(|item| match item {
            ScalarItem::Binding(binding) if span_of(&binding.value) == block_span => Some(binding),
            _ => None,
        });
        let binding = targets.next().ok_or_else(|| unsupported("ordinary comptime result binding was not preserved"))?;
        if targets.next().is_some() {
            return Err(unsupported("ordinary comptime result binding is not unique"));
        }
        let literal_body = match &binding.value {
            ScalarExpression::Block(block) => match block.items.as_slice() {
                [ScalarBlockItem::Expression(value)] => Some(value),
                _ => None,
            },
            _ => None,
        };
        (&binding.declared_type, literal_body)
    };
    let (shape, _) = shape_for(project, result_type, &mut HashMap::new(), 0)
        .ok_or_else(|| unsupported("unsupported resolved comptime stdout type"))?;
    let literal = literal_body.and_then(|value| LiteralStdout::from_checked(&shape, value));
    let llvm = if emit {
        let llvm = if project.modules.len() == 1 {
            emit_scalar_build_single_module_artifact_llvm(&build)
        } else {
            emit_scalar_build_project_artifact_llvm(&build)
        }.map_err(Error::Emitter)?;
        Some(llvm)
    } else { None };
    Ok((PreparedComptimeStdout { site: scope, result_type: result_type.clone(), shape, literal }, llvm))
}

fn closed_evaluator_body(body: &ScalarBlock) -> bool {
    if body.unsafe_context { return false; }
    let Some((ScalarBlockItem::Expression(tail), locals)) = body.items.split_last() else { return false; };
    let mut values = vec![tail];
    for item in locals {
        let ScalarBlockItem::LocalBinding(binding) = item else { return false; };
        values.push(&binding.value);
    }
    while let Some(value) = values.pop() {
        match value {
            ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. } |
            ScalarExpression::Name { .. } | ScalarExpression::Utf8 { .. } => {},
            ScalarExpression::Unary { operator, operand, .. }
                if audited_closed_unary(operator) ||
                   (*operator == UnaryOperator::Negate && matches!(operand.as_ref(), ScalarExpression::Integer { .. })) =>
                values.push(operand),
            ScalarExpression::Binary { operator, left, right, .. } if audited_closed_eager_binary(operator) =>
                values.extend([left.as_ref(), right.as_ref()]),
            ScalarExpression::ArrayLiteral { elements, .. } => values.extend(elements.iter()),
            ScalarExpression::StructLiteral { fields, .. } => values.extend(fields.iter().map(|field| &field.value)),
            _ => return false,
        }
    }
    true
}

fn shape_for(
    project: &ScalarProject, ty: &ScalarType,
    memo: &mut HashMap<ScalarStructId, (Arc<Shape>, usize)>, depth: usize,
) -> Option<(Arc<Shape>, usize)> {
    if depth > MAX_VALUE_DEPTH { return None; }
    if let Some(shape) = Shape::scalar(ty) { return Some((Arc::new(shape), 0)); }
    let (kind, height) = match ty {
        ScalarType::Array { element, length, .. } => {
            let (element, child_height) = shape_for(project, element, memo, depth + 1)?;
            (Shape::Array { count: usize::try_from(*length).ok()?, element }, child_height + 1)
        }
        ScalarType::Struct(id) => {
            if let Some((shape, height)) = memo.get(id) {
                return (depth + height <= MAX_VALUE_DEPTH).then(|| (shape.clone(), *height));
            }
            let structure = project.modules.iter().find(|module| module.source == id.source)?
                .structs.iter().find(|structure| structure.id == *id)?;
            let (shape, height) = if id.source.package == "std" && structure.name == "utf8" {
                // The resolved standard text declaration, not a same-spelled
                // user struct or bytes of its native memory layout.
                let [data, length] = structure.fields.as_slice() else { return None; };
                if data.name != "data" || length.name != "length"
                    || !matches!(&data.ty, ScalarType::RawPointer(element) if **element == ScalarType::U8)
                    || length.ty != ScalarType::U64
                { return None; }
                (Shape::Utf8, 0)
            } else {
                let mut height = 0;
                let mut fields = Vec::with_capacity(structure.fields.len());
                for field in &structure.fields {
                    let (shape, child_height) = shape_for(project, &field.ty, memo, depth + 1)?;
                    height = height.max(child_height + 1);
                    fields.push((field.name.clone(), shape));
                }
                (Shape::Struct(fields), height)
            };
            let shape = Arc::new(shape);
            memo.insert(id.clone(), (shape.clone(), height));
            return Some((shape, height));
        }
        _ => return None,
    };
    Some((Arc::new(kind), height))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeStdoutError {
    ByteLimit,
    NonCanonical,
    OutOfRange,
    UnsupportedResultType,
}

impl fmt::Display for ComptimeStdoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ByteLimit => "comptime stdout exceeds its byte budget",
            Self::NonCanonical => "comptime stdout must be one canonical value followed by LF",
            Self::OutOfRange => "comptime stdout integer is outside the expected type",
            Self::UnsupportedResultType => "unsupported comptime stdout result type",
        })
    }
}

impl std::error::Error for ComptimeStdoutError {}

impl PreparedComptimeStdout {
    pub fn site(&self) -> &SourceSpan { &self.site }
    pub fn result_type(&self) -> &ScalarType { &self.result_type }

    /// Write the exact compiler-observed ordinary literal when the whole
    /// selected body is that value. A computed body still needs the evaluator.
    /// Failure returns no byte count; the caller owns any partial scratch output.
    pub fn write_literal_stdout(&self, out: &mut impl Write, max_bytes: usize) -> io::Result<usize> {
        self.literal.as_ref().ok_or_else(|| io::Error::new(
            io::ErrorKind::Unsupported, "comptime stdout needs an evaluator for this computed body",
        ))?.write_line(out, max_bytes)
    }

    /// Materialize one parsed compiler value, not authority to apply it.
    pub fn parse_stdout(&self, stdout: &[u8], max_bytes: usize) -> Result<ScalarExpression, ComptimeStdoutError> {
        parse(&self.shape, self.site.range, stdout, max_bytes)
    }
}

impl ClosedComptimeFragment {
    /// Same grammar for the existing compiler-issued scalar fragment.
    pub fn parse_stdout(&self, stdout: &[u8], max_bytes: usize) -> Result<ScalarExpression, ComptimeStdoutError> {
        let shape = Shape::scalar(self.result_type()).ok_or(ComptimeStdoutError::UnsupportedResultType)?;
        parse(&shape, self.site().range, stdout, max_bytes)
    }
}

fn parse(shape: &Shape, span: ByteSpan, stdout: &[u8], budget: usize) -> Result<ScalarExpression, ComptimeStdoutError> {
    use ComptimeStdoutError as Error;
    if stdout.len() > budget { return Err(Error::ByteLimit); }
    let line = stdout.strip_suffix(b"\n").ok_or(Error::NonCanonical)?;
    let mut parser = Parser { input: line, span, field_bytes: budget };
    let value = parser.value(shape)?;
    if !parser.input.is_empty() { return Err(Error::NonCanonical); }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    span: ByteSpan,
    // Materializing ordinary named AST fields copies these names. Do not
    // allow repeated short records to amplify them beyond the same budget.
    field_bytes: usize,
}

impl Parser<'_> {
    fn eat(&mut self, text: &[u8]) -> Result<(), ComptimeStdoutError> {
        self.input = self.input.strip_prefix(text).ok_or(ComptimeStdoutError::NonCanonical)?;
        Ok(())
    }

    fn value(&mut self, shape: &Shape) -> Result<ScalarExpression, ComptimeStdoutError> {
        use ComptimeStdoutError as Error;
        let span = self.span;
        match shape {
            Shape::Bool => {
                let value = if self.input.starts_with(b"true") {
                    self.eat(b"true")?;
                    true
                } else {
                    self.eat(b"false")?;
                    false
                };
                Ok(ScalarExpression::Boolean { value, span })
            }
            Shape::I32 => {
                let value = i32::try_from(self.decimal(true)?).map_err(|_| Error::OutOfRange)?.into();
                Ok(ScalarExpression::Integer { value, span })
            }
            Shape::U32 => {
                let value = u32::try_from(self.decimal(false)?).map_err(|_| Error::OutOfRange)?.into();
                Ok(ScalarExpression::Integer { value, span })
            }
            Shape::Utf8 => Ok(ScalarExpression::Utf8 { value: self.text()?, span }),
            Shape::Array { count, element } => {
                self.eat(b"[")?;
                let mut elements = Vec::new();
                for index in 0..*count {
                    if index != 0 { self.eat(b",")?; }
                    elements.push(self.value(element)?);
                }
                self.eat(b"]")?;
                Ok(ScalarExpression::ArrayLiteral { elements, span })
            }
            Shape::Struct(expected) => {
                self.eat(b"[")?;
                let mut fields = Vec::new();
                for (index, (name, shape)) in expected.iter().enumerate() {
                    if index != 0 { self.eat(b",")?; }
                    let value = self.value(shape)?;
                    self.field_bytes = self.field_bytes.checked_sub(name.len()).ok_or(Error::ByteLimit)?;
                    fields.push(ScalarStructLiteralField { name: name.clone(), name_span: span, span, value });
                }
                self.eat(b"]")?;
                Ok(ScalarExpression::StructLiteral { fields, span })
            }
        }
    }

    fn decimal(&mut self, signed: bool) -> Result<i64, ComptimeStdoutError> {
        use ComptimeStdoutError as Error;
        let negative = signed && self.input.starts_with(b"-");
        if negative { self.eat(b"-")?; }
        let (&first, rest) = self.input.split_first().ok_or(Error::NonCanonical)?;
        self.input = rest;
        if first == b'0' && !negative { return Ok(0); }
        if !(b'1'..=b'9').contains(&first) { return Err(Error::NonCanonical); }
        let mut value = i64::from(first - b'0');
        while let Some((&byte, tail)) = self.input.split_first() {
            if !byte.is_ascii_digit() { break; }
            value = value.checked_mul(10).and_then(|v| v.checked_add(i64::from(byte - b'0')))
                .ok_or(Error::OutOfRange)?;
            self.input = tail;
        }
        Ok(if negative { -value } else { value })
    }

    fn text(&mut self) -> Result<Vec<u8>, ComptimeStdoutError> {
        use ComptimeStdoutError as Error;
        self.eat(b"\"")?;
        let mut text = Vec::new();
        let mut start = self.input;
        while let Some((&byte, rest)) = self.input.split_first() {
            self.input = rest;
            if byte == b'"' || byte == b'\\' {
                let length = start.len() - rest.len() - 1;
                let raw = &start[..length];
                // Validate each raw UTF-8 run once before appending it. Its
                // boundaries are ASCII quote/escape, never inside a character.
                std::str::from_utf8(raw).map_err(|_| Error::NonCanonical)?;
                text.extend_from_slice(raw);
                if byte == b'"' { return Ok(text); }
                let (&escaped, after) = self.input.split_first().ok_or(Error::NonCanonical)?;
                self.input = after;
                let value = match escaped {
                    b'"' | b'\\' => escaped,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'u' => {
                        let [b'0', b'0', high, low, tail @ ..] = self.input else { return Err(Error::NonCanonical); };
                        let value = hex(*high).and_then(|high| hex(*low).map(|low| high * 16 + low))
                            .filter(|value| *value < 0x20 && !matches!(*value, b'\n' | b'\r' | b'\t'))
                            .ok_or(Error::NonCanonical)?;
                        self.input = tail;
                        value
                    }
                    _ => return Err(Error::NonCanonical),
                };
                text.push(value);
                start = self.input;
            } else if byte < 0x20 {
                return Err(Error::NonCanonical);
            }
        }
        Err(Error::NonCanonical)
    }
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}
