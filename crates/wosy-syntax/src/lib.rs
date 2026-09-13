use pest::iterators::Pair;
use pest::Parser;
use rowan::{GreenNodeBuilder, Language, SyntaxNode, SyntaxToken};
use serde::{Deserialize, Serialize};

#[derive(pest_derive::Parser)]
#[grammar = "grammar.pest"]
struct WosyParser;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u16)]
pub enum SyntaxKind {
    Root,
    Error,
    Whitespace,
    LineEnding,
    Indentation,
    TypedComment,
    BoundaryStart,
    BoundaryEnd,
    ExternDecl,
    ExternFunction,
    FunctionDecl,
    BindingDecl,
    Item,
    CallableType,
    RawPointerType,
    Unsafe,
    TypeSpec,
    TypeName,
    Identifier,
    Parameters,
    Block,
    BlockItem,
    FinalOutputList,
    Expression,
    Call,
    IfExpr,
    Parenthesized,
    Boolean,
    Binary,
    Operator,
    Integer,
    Float,
    Char,
    String,
    Punctuation,
    NamespaceDecl,
    QualifiedCall,
    QualifiedMember,
    AssignmentTarget,
    LocalBinding,
    While,
    Assignment,
    TopLevelItem,
    StructDecl,
    StructField,
    CallableOutput,
    ReceiverList,
    Receiver,
    OutputList,
    AssignmentTargets,
    StructLiteral,
    StructLiteralField,
    FieldAccess,
    DereferencedField,
    Dereference,
    RawAddress,
    GenericTypeArguments,
    GenericTypeArgument,
    QualifiedType,
    UnitIfExpr,
    Unary,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WosyLanguage;

impl Language for WosyLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        match raw.0 {
            0 => SyntaxKind::Root,
            1 => SyntaxKind::Error,
            2 => SyntaxKind::Whitespace,
            3 => SyntaxKind::LineEnding,
            4 => SyntaxKind::Indentation,
            5 => SyntaxKind::TypedComment,
            6 => SyntaxKind::BoundaryStart,
            7 => SyntaxKind::BoundaryEnd,
            8 => SyntaxKind::ExternDecl,
            9 => SyntaxKind::ExternFunction,
            10 => SyntaxKind::FunctionDecl,
            11 => SyntaxKind::BindingDecl,
            12 => SyntaxKind::Item,
            13 => SyntaxKind::CallableType,
            14 => SyntaxKind::RawPointerType,
            15 => SyntaxKind::Unsafe,
            16 => SyntaxKind::TypeSpec,
            17 => SyntaxKind::TypeName,
            18 => SyntaxKind::Identifier,
            19 => SyntaxKind::Parameters,
            20 => SyntaxKind::Block,
            21 => SyntaxKind::BlockItem,
            22 => SyntaxKind::FinalOutputList,
            23 => SyntaxKind::Expression,
            24 => SyntaxKind::Call,
            25 => SyntaxKind::IfExpr,
            26 => SyntaxKind::Parenthesized,
            27 => SyntaxKind::Boolean,
            28 => SyntaxKind::Binary,
            29 => SyntaxKind::Operator,
            30 => SyntaxKind::Integer,
            31 => SyntaxKind::Float,
            32 => SyntaxKind::Char,
            33 => SyntaxKind::String,
            34 => SyntaxKind::Punctuation,
            35 => SyntaxKind::NamespaceDecl,
            36 => SyntaxKind::QualifiedCall,
            37 => SyntaxKind::QualifiedMember,
            38 => SyntaxKind::AssignmentTarget,
            39 => SyntaxKind::LocalBinding,
            40 => SyntaxKind::While,
            41 => SyntaxKind::Assignment,
            42 => SyntaxKind::TopLevelItem,
            43 => SyntaxKind::StructDecl,
            44 => SyntaxKind::StructField,
            45 => SyntaxKind::CallableOutput,
            46 => SyntaxKind::ReceiverList,
            47 => SyntaxKind::Receiver,
            48 => SyntaxKind::OutputList,
            49 => SyntaxKind::AssignmentTargets,
            50 => SyntaxKind::StructLiteral,
            51 => SyntaxKind::StructLiteralField,
            52 => SyntaxKind::FieldAccess,
            53 => SyntaxKind::DereferencedField,
            54 => SyntaxKind::Dereference,
            55 => SyntaxKind::RawAddress,
            56 => SyntaxKind::GenericTypeArguments,
            57 => SyntaxKind::GenericTypeArgument,
            58 => SyntaxKind::QualifiedType,
            59 => SyntaxKind::UnitIfExpr,
            60 => SyntaxKind::Unary,
            _ => panic!("invalid syntax kind: {}", raw.0),
        }
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type CstNode = SyntaxNode<WosyLanguage>;
pub type CstToken = SyntaxToken<WosyLanguage>;

#[derive(Clone, Debug)]
pub struct CanonicalCstRoot {
    pub source: SourceIdentity,
    pub root: CstNode,
}

impl CanonicalCstRoot {
    pub fn new(source: SourceIdentity, root: CstNode) -> Self {
        Self { source, root }
    }

    pub fn snapshot(&self) -> CanonicalCstSnapshot {
        CanonicalCstSnapshot {
            source: self.source.clone(),
            root: snapshot_node(&self.root),
        }
    }

    pub fn from_snapshot(snapshot: CanonicalCstSnapshot) -> Self {
        let mut builder = GreenNodeBuilder::new();
        build_snapshot_node(&mut builder, &snapshot.root);
        Self::new(snapshot.source, CstNode::new_root(builder.finish()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalCstSnapshot {
    pub source: SourceIdentity,
    pub root: CanonicalCstSnapshotNode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CanonicalCstSnapshotNode {
    pub kind: SyntaxKind,
    pub text: String,
    pub children: Vec<CanonicalCstSnapshotElement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CanonicalCstSnapshotElement {
    Node(CanonicalCstSnapshotNode),
    Token { kind: SyntaxKind, text: String },
}

fn snapshot_node(node: &CstNode) -> CanonicalCstSnapshotNode {
    let children = node
        .children_with_tokens()
        .map(|element| match element {
            rowan::NodeOrToken::Node(child) => {
                CanonicalCstSnapshotElement::Node(snapshot_node(&child))
            }
            rowan::NodeOrToken::Token(token) => CanonicalCstSnapshotElement::Token {
                kind: token.kind(),
                text: token.text().to_owned(),
            },
        })
        .collect();
    CanonicalCstSnapshotNode {
        kind: node.kind(),
        text: node.text().to_string(),
        children,
    }
}

fn build_snapshot_node(builder: &mut GreenNodeBuilder<'_>, node: &CanonicalCstSnapshotNode) {
    builder.start_node(WosyLanguage::kind_to_raw(node.kind));
    for child in &node.children {
        match child {
            CanonicalCstSnapshotElement::Node(child) => build_snapshot_node(builder, child),
            CanonicalCstSnapshotElement::Token { kind, text } => {
                builder.token(WosyLanguage::kind_to_raw(*kind), text);
            }
        }
    }
    builder.finish_node();
}

pub fn byte_span(node: &CstNode) -> ByteSpan {
    let range = node.text_range();
    ByteSpan::new(u32::from(range.start()), u32::from(range.end()))
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub project: String,
    pub package: String,
    pub path: String,
    pub revision: String,
}

impl SourceIdentity {
    pub fn new(project: String, package: String, path: String, revision: String) -> Self {
        Self {
            project,
            package,
            path,
            revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ByteSpan {
    pub start: u32,
    pub end: u32,
}

impl ByteSpan {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub source: SourceIdentity,
    pub range: ByteSpan,
}

impl SourceSpan {
    pub fn new(source: SourceIdentity, range: ByteSpan) -> Self {
        Self { source, range }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypedComment {
    pub span: ByteSpan,
    pub category: String,
    pub raw_payload: String,
    pub valid: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SyntaxError {
    pub span: ByteSpan,
    pub message: String,
}

pub struct ParseResult {
    pub source: SourceIdentity,
    pub text: String,
    pub root: CstNode,
    pub canonical_root: CanonicalCstRoot,
    pub comments: Vec<TypedComment>,
    pub errors: Vec<SyntaxError>,
}

impl ParseResult {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
    pub fn reconstruct(&self) -> String {
        self.root.text().to_string()
    }
    pub fn range(&self) -> ByteSpan {
        ByteSpan::new(0, self.text.len() as u32)
    }

    pub fn canonical_cst(&self) -> &CanonicalCstRoot {
        &self.canonical_root
    }
}

pub fn parse(source: SourceIdentity, text: String, configured_comments: &[String]) -> ParseResult {
    let mut errors = Vec::new();
    let comments = scan_comments(&text, configured_comments, &mut errors);
    let mut builder = GreenNodeBuilder::new();
    builder.start_node(WosyLanguage::kind_to_raw(SyntaxKind::Root));
    let parsed = WosyParser::parse(Rule::source, &text);
    if let Ok(mut pairs) = parsed {
        let pair = pairs.next().expect("source pair");
        build_pair(&mut builder, pair, &text, &comments, &mut errors);
    } else {
        builder.start_node(WosyLanguage::kind_to_raw(SyntaxKind::Error));
        add_gap(&mut builder, &text, 0, &comments);
        builder.finish_node();
        errors.push(SyntaxError {
            span: ByteSpan::new(0, text.len() as u32),
            message: "source does not match the base grammar".to_owned(),
        });
    }
    builder.finish_node();
    let root = CstNode::new_root(builder.finish());
    let canonical_root = CanonicalCstRoot::new(source.clone(), root.clone());
    ParseResult {
        source,
        text,
        root,
        canonical_root,
        comments,
        errors,
    }
}

fn build_pair(
    builder: &mut GreenNodeBuilder<'_>,
    pair: Pair<'_, Rule>,
    text: &str,
    comments: &[TypedComment],
    errors: &mut Vec<SyntaxError>,
) {
    let span = pair.as_span();
    if pair.as_rule() == Rule::error_item {
        let range = ByteSpan::new(span.start() as u32, span.end() as u32);
        builder.start_node(WosyLanguage::kind_to_raw(SyntaxKind::Error));
        add_gap(builder, span.as_str(), span.start(), comments);
        builder.finish_node();
        errors.push(SyntaxError {
            span: range,
            message: "source does not match the base grammar".to_owned(),
        });
        return;
    }
    if pair.as_rule() == Rule::comment {
        builder.token(
            WosyLanguage::kind_to_raw(SyntaxKind::TypedComment),
            span.as_str(),
        );
        return;
    }
    if is_empty_operator_tier(&pair) {
        let span = pair.as_span();
        let mut inner = pair.into_inner();
        let Some(first) = inner.next() else {
            builder.token(
                WosyLanguage::kind_to_raw(SyntaxKind::Punctuation),
                span.as_str(),
            );
            return;
        };
        let mut cursor = span.start();
        for child in std::iter::once(first).chain(inner) {
            let child_span = child.as_span();
            add_gap(builder, &text[cursor..child_span.start()], cursor, comments);
            build_pair(builder, child, text, comments, errors);
            cursor = child_span.end();
        }
        add_gap(builder, &text[cursor..span.end()], cursor, comments);
        return;
    }
    let kind = kind(pair.as_rule());
    let mut inner = pair.into_inner();
    let Some(first) = inner.next() else {
        builder.token(WosyLanguage::kind_to_raw(kind), span.as_str());
        return;
    };
    builder.start_node(WosyLanguage::kind_to_raw(kind));
    let mut cursor = span.start();
    for child in std::iter::once(first).chain(inner) {
        let child_span = child.as_span();
        add_gap(builder, &text[cursor..child_span.start()], cursor, comments);
        build_pair(builder, child, text, comments, errors);
        cursor = child_span.end();
    }
    add_gap(builder, &text[cursor..span.end()], cursor, comments);
    builder.finish_node();
}

fn is_empty_operator_tier(pair: &Pair<'_, Rule>) -> bool {
    matches!(
        pair.as_rule(),
        Rule::bit_or | Rule::bit_xor | Rule::bit_and | Rule::shift | Rule::unary
    ) && !pair.clone().into_inner().any(|child| {
        matches!(
            child.as_rule(),
            Rule::or_operator
                | Rule::and_operator
                | Rule::bit_or_operator
                | Rule::bit_xor_operator
                | Rule::bit_and_operator
                | Rule::comparison_operator
                | Rule::shift_operator
                | Rule::add_operator
                | Rule::multiply_operator
                | Rule::unary_operator
        )
    })
}

fn add_gap(
    builder: &mut GreenNodeBuilder<'_>,
    text: &str,
    offset: usize,
    comments: &[TypedComment],
) {
    let mut start = 0;
    let mut index = 0;
    while index < text.len() {
        let byte = text.as_bytes()[index];
        let (kind, end) = match byte {
            b' ' => (Some(SyntaxKind::Whitespace), index + 1),
            b'\t' => (Some(SyntaxKind::Indentation), index + 1),
            b'\r' if text.as_bytes().get(index + 1) == Some(&b'\n') => {
                (Some(SyntaxKind::LineEnding), index + 2)
            }
            b'\r' | b'\n' => (Some(SyntaxKind::LineEnding), index + 1),
            _ if comments
                .iter()
                .any(|comment| comment.span.start as usize == offset + index) =>
            {
                let end = comments
                    .iter()
                    .find(|comment| comment.span.start as usize == offset + index)
                    .expect("comment span")
                    .span
                    .end as usize
                    - offset;
                if index > start {
                    builder.token(
                        WosyLanguage::kind_to_raw(SyntaxKind::Punctuation),
                        &text[start..index],
                    );
                }
                builder.token(
                    WosyLanguage::kind_to_raw(SyntaxKind::TypedComment),
                    &text[index..end],
                );
                start = end;
                index = end;
                continue;
            }
            _ => (Some(SyntaxKind::Punctuation), next_char_end(text, index)),
        };
        if let Some(kind) = kind {
            if index > start {
                builder.token(
                    WosyLanguage::kind_to_raw(SyntaxKind::Punctuation),
                    &text[start..index],
                );
            }
            builder.token(WosyLanguage::kind_to_raw(kind), &text[index..end]);
            start = end;
        }
        index = end;
    }
    if start < text.len() {
        builder.token(
            WosyLanguage::kind_to_raw(SyntaxKind::Punctuation),
            &text[start..],
        );
    }
}

fn next_char_end(text: &str, index: usize) -> usize {
    text[index..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| index + offset)
}

fn scan_comments(
    text: &str,
    configured: &[String],
    errors: &mut Vec<SyntaxError>,
) -> Vec<TypedComment> {
    let mut comments = Vec::new();
    let Ok(mut pairs) = WosyParser::parse(Rule::comment_scan, text) else {
        return comments;
    };
    let pair = pairs.next().expect("comment scan pair");
    collect_comments(pair, configured, errors, &mut comments);
    comments
}

fn collect_comments(
    pair: Pair<'_, Rule>,
    configured: &[String],
    errors: &mut Vec<SyntaxError>,
    comments: &mut Vec<TypedComment>,
) {
    if pair.as_rule() == Rule::comment {
        let span = pair.as_span();
        let mut children = pair.into_inner();
        let category_pair = children
            .find(|child| matches!(child.as_rule(), Rule::typed_comment | Rule::untyped_comment))
            .expect("comment category pair");
        let typed = category_pair.as_rule() == Rule::typed_comment;
        let mut fields = category_pair.into_inner();
        let category = fields.next().expect("comment category");
        let payload = fields.next();
        let category_text = category.as_str();
        let raw_payload = payload.map_or_else(String::new, |pair| pair.as_str().to_owned());
        let valid = typed
            && !category_text.is_empty()
            && configured.iter().any(|item| item == category_text);
        if !valid {
            errors.push(SyntaxError {
                span: ByteSpan::new(span.start() as u32, span.end() as u32),
                message: "invalid or unconfigured typed comment".to_owned(),
            });
        }
        comments.push(TypedComment {
            span: ByteSpan::new(span.start() as u32, span.end() as u32),
            category: category_text.to_owned(),
            raw_payload,
            valid,
        });
        return;
    }
    for child in pair.into_inner() {
        collect_comments(child, configured, errors, comments);
    }
}

fn kind(rule: Rule) -> SyntaxKind {
    match rule {
        Rule::source => SyntaxKind::Root,
        Rule::boundary_start => SyntaxKind::BoundaryStart,
        Rule::boundary_end => SyntaxKind::BoundaryEnd,
        Rule::extern_decl => SyntaxKind::ExternDecl,
        Rule::extern_function => SyntaxKind::ExternFunction,
        Rule::function_decl => SyntaxKind::FunctionDecl,
        Rule::binding_decl => SyntaxKind::BindingDecl,
        Rule::local_binding => SyntaxKind::LocalBinding,
        Rule::namespace_decl => SyntaxKind::NamespaceDecl,
        Rule::item => SyntaxKind::Item,
        Rule::type_spec => SyntaxKind::TypeSpec,
        Rule::callable_type => SyntaxKind::CallableType,
        Rule::type_name => SyntaxKind::TypeName,
        Rule::qualified_type => SyntaxKind::QualifiedType,
        Rule::raw_pointer_type => SyntaxKind::RawPointerType,
        Rule::unsafe_marker => SyntaxKind::Unsafe,
        Rule::identifier => SyntaxKind::Identifier,
        Rule::parameters => SyntaxKind::Parameters,
        Rule::block => SyntaxKind::Block,
        Rule::block_item => SyntaxKind::BlockItem,
        Rule::if_block_item => SyntaxKind::Expression,
        Rule::final_output_list => SyntaxKind::FinalOutputList,
        Rule::expression => SyntaxKind::Expression,
        Rule::call => SyntaxKind::Call,
        Rule::qualified_call => SyntaxKind::QualifiedCall,
        Rule::generic_type_arguments => SyntaxKind::GenericTypeArguments,
        Rule::generic_type_argument => SyntaxKind::GenericTypeArgument,
        Rule::qualified_member => SyntaxKind::QualifiedMember,
        Rule::assignment_target => SyntaxKind::AssignmentTarget,
        Rule::if_expr => SyntaxKind::IfExpr,
        Rule::unit_if_expr => SyntaxKind::UnitIfExpr,
        Rule::unsafe_expr => SyntaxKind::Unsafe,
        Rule::while_expr => SyntaxKind::While,
        Rule::assignment => SyntaxKind::Assignment,
        Rule::top_level_item => SyntaxKind::TopLevelItem,
        Rule::struct_decl => SyntaxKind::StructDecl,
        Rule::struct_field => SyntaxKind::StructField,
        Rule::callable_output => SyntaxKind::CallableOutput,
        Rule::receiver_list => SyntaxKind::ReceiverList,
        Rule::receiver => SyntaxKind::Receiver,
        Rule::output_list => SyntaxKind::OutputList,
        Rule::assignment_targets => SyntaxKind::AssignmentTargets,
        Rule::struct_literal => SyntaxKind::StructLiteral,
        Rule::struct_literal_field => SyntaxKind::StructLiteralField,
        Rule::field_access => SyntaxKind::FieldAccess,
        Rule::dereferenced_field => SyntaxKind::DereferencedField,
        Rule::dereference => SyntaxKind::Dereference,
        Rule::raw_address => SyntaxKind::RawAddress,
        Rule::error_item => SyntaxKind::Error,
        Rule::parenthesized => SyntaxKind::Parenthesized,
        Rule::boolean => SyntaxKind::Boolean,
        Rule::logical_or
        | Rule::logical_and
        | Rule::comparison
        | Rule::additive
        | Rule::multiplicative
        | Rule::bit_or
        | Rule::bit_xor
        | Rule::bit_and
        | Rule::shift => SyntaxKind::Binary,
        Rule::unary => SyntaxKind::Unary,
        Rule::or_operator
        | Rule::and_operator
        | Rule::bit_or_operator
        | Rule::bit_xor_operator
        | Rule::bit_and_operator
        | Rule::shift_operator
        | Rule::unary_operator
        | Rule::comparison_operator
        | Rule::add_operator
        | Rule::multiply_operator
        | Rule::operator => SyntaxKind::Operator,
        Rule::integer => SyntaxKind::Integer,
        Rule::float => SyntaxKind::Float,
        Rule::char => SyntaxKind::Char,
        Rule::string => SyntaxKind::String,
        Rule::comment => SyntaxKind::TypedComment,
        _ => SyntaxKind::Punctuation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = "%%start\nenv = extern wasm \"env\" {\n\tunit(i32) print_i32;\n};\n\ni32(i32) add_one = fn(value) {\n\tvalue + 1\n};\n\n# INTENT: Exercise raw typed-comment preservation.\nunit() main = fn {\n\tenv.print_i32(add_one(41));\n};\n%%end";

    fn identity() -> SourceIdentity {
        SourceIdentity::new("p".into(), "pkg".into(), "src/main.w".into(), "r1".into())
    }
    #[test]
    fn baseline_and_lossless() {
        let result = parse(identity(), BASE.into(), &["INTENT".into()]);
        assert!(result.is_valid());
        assert_eq!(result.reconstruct(), BASE);
        assert_eq!(result.range(), ByteSpan::new(0, BASE.len() as u32));
    }
    #[test]
    fn comment_payload_and_tabs() {
        let result = parse(identity(), BASE.into(), &["INTENT".into()]);
        assert_eq!(result.comments[0].category, "INTENT");
        assert_eq!(
            result.comments[0].raw_payload,
            " Exercise raw typed-comment preservation."
        );
        assert!(result.reconstruct().contains('\t'));
    }

    #[test]
    fn accepts_space_indentation_and_preserves_source() {
        let text = "%%start\ni32() main = fn {\n  1 + 2\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid());
        assert_eq!(result.reconstruct(), text);
    }

    #[test]
    fn accepts_tab_indentation_and_expression_spaces() {
        let text = "%%start\ni32() main = fn {\n\t1 + 2\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid());
        assert_eq!(result.reconstruct(), text);
    }

    #[test]
    fn untyped_and_malformed_comments_keep_structured_fields() {
        let text =
            "# note\n# : malformed payload\n# INTENT:\n# INTENT:  exact  \n# \"quoted # text\"";
        let result = parse(identity(), text.into(), &["INTENT".into()]);
        assert_eq!(result.comments.len(), 5);
        assert_eq!(result.comments[0].category, "note");
        assert_eq!(result.comments[0].raw_payload, "");
        assert_eq!(result.comments[1].category, "");
        assert_eq!(result.comments[1].raw_payload, " malformed payload");
        assert_eq!(result.comments[2].raw_payload, "");
        assert_eq!(result.comments[3].raw_payload, "  exact  ");
        assert_eq!(result.comments[3].span, ByteSpan::new(39, 57));
        assert_eq!(result.comments[4].category, "\"quoted # text\"");
        assert_eq!(result.reconstruct(), text);
    }
    #[test]
    fn recovery_is_lossless() {
        let text = "%%start\n# nope\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(!result.is_valid());
        assert_eq!(result.reconstruct(), text);
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Error));
    }

    #[test]
    fn recovery_retains_adjacent_items_and_precise_structure() {
        let text = "%%start\ni32 before = 1;\n# INTENT: retained while recovering.\nbroken source;\ni32 after = 2;\n%%end";
        let result = parse(identity(), text.into(), &["INTENT".into()]);
        assert!(!result.is_valid());
        assert_eq!(result.reconstruct(), text);
        assert_eq!(result.comments.len(), 1);
        assert_eq!(result.comments[0].category, "INTENT");
        assert_eq!(
            result.comments[0].raw_payload,
            " retained while recovering."
        );
        let error = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::Error)
            .expect("recovered error node");
        let error_span = byte_span(&error);
        assert_eq!(
            error_span,
            ByteSpan::new(
                text.find("broken source;").expect("error text") as u32,
                (text.find("broken source;").expect("error text") + "broken source;".len()) as u32
            )
        );
        assert_eq!(result.errors[0].span, error_span);
        assert!(result.root.descendants_with_tokens().any(|element| {
            element.kind() == SyntaxKind::TypedComment
                && element.to_string() == "# INTENT: retained while recovering."
        }));
        let top_level: Vec<_> = result
            .root
            .descendants_with_tokens()
            .filter(|element| {
                matches!(
                    element.kind(),
                    SyntaxKind::BoundaryStart
                        | SyntaxKind::BindingDecl
                        | SyntaxKind::Error
                        | SyntaxKind::BoundaryEnd
                )
            })
            .collect();
        assert_eq!(
            top_level
                .iter()
                .map(|element| element.kind())
                .collect::<Vec<_>>(),
            vec![
                SyntaxKind::BoundaryStart,
                SyntaxKind::BindingDecl,
                SyntaxKind::Error,
                SyntaxKind::BindingDecl,
                SyntaxKind::BoundaryEnd,
            ]
        );
    }

    #[test]
    fn recovery_synchronizes_at_boundary_end() {
        let text = "%%start\nbroken source\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(!result.is_valid());
        let error = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::Error)
            .expect("recovered error node");
        assert_eq!(byte_span(&error), ByteSpan::new(8, 22));
        assert_eq!(result.errors[0].span, ByteSpan::new(8, 22));
        assert!(result.root.descendants_with_tokens().any(|element| {
            element.kind() == SyntaxKind::BoundaryEnd && element.to_string() == "%%end"
        }));
        assert_eq!(result.reconstruct(), text);
    }
    #[test]
    fn raw_kinds_cover_declared_values() {
        let kinds = [
            SyntaxKind::Root,
            SyntaxKind::Error,
            SyntaxKind::Whitespace,
            SyntaxKind::LineEnding,
            SyntaxKind::Indentation,
            SyntaxKind::TypedComment,
            SyntaxKind::BoundaryStart,
            SyntaxKind::BoundaryEnd,
            SyntaxKind::ExternDecl,
            SyntaxKind::ExternFunction,
            SyntaxKind::NamespaceDecl,
            SyntaxKind::FunctionDecl,
            SyntaxKind::CallableType,
            SyntaxKind::TypeName,
            SyntaxKind::Identifier,
            SyntaxKind::Parameters,
            SyntaxKind::Block,
            SyntaxKind::Expression,
            SyntaxKind::FinalOutputList,
            SyntaxKind::Call,
            SyntaxKind::QualifiedCall,
            SyntaxKind::Binary,
            SyntaxKind::Operator,
            SyntaxKind::Integer,
            SyntaxKind::Char,
            SyntaxKind::String,
            SyntaxKind::Punctuation,
            SyntaxKind::StructDecl,
            SyntaxKind::StructField,
            SyntaxKind::CallableOutput,
            SyntaxKind::ReceiverList,
            SyntaxKind::Receiver,
            SyntaxKind::OutputList,
            SyntaxKind::AssignmentTargets,
            SyntaxKind::StructLiteral,
            SyntaxKind::StructLiteralField,
            SyntaxKind::FieldAccess,
            SyntaxKind::DereferencedField,
            SyntaxKind::Dereference,
            SyntaxKind::RawAddress,
            SyntaxKind::GenericTypeArguments,
            SyntaxKind::GenericTypeArgument,
            SyntaxKind::QualifiedType,
            SyntaxKind::Unary,
        ];
        for kind in kinds {
            assert_eq!(
                WosyLanguage::kind_from_raw(WosyLanguage::kind_to_raw(kind)),
                kind
            );
        }
    }

    #[test]
    fn punctuation_is_preserved_as_punctuation() {
        let result = parse(identity(), BASE.into(), &["INTENT".into()]);
        assert!(result.root.descendants_with_tokens().any(|element| {
            element.kind() == SyntaxKind::Punctuation && element.to_string() == "="
        }));
        assert!(result.root.descendants_with_tokens().any(|element| {
            element.kind() == SyntaxKind::TypedComment && element.to_string().starts_with('#')
        }));
    }

    #[test]
    fn expression_operators_have_structural_precedence_and_lossless_tokens() {
        let text = "%%start\ni32 value = 1 | 2 ^ 3 & 4 << 1 + 2 * 3;\nbool flags = !!!true && false || true;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);

        let binary = result
            .root
            .descendants()
            .find(|node| {
                node.kind() == SyntaxKind::Binary && node.text().to_string().contains("1 |")
            })
            .expect("outer bitwise-or node");
        assert_eq!(binary.text(), "1 | 2 ^ 3 & 4 << 1 + 2 * 3");
        assert!(binary.children().any(|node| {
            node.kind() == SyntaxKind::Binary && node.text().to_string().contains("2 ^")
        }));
        assert!(result.root.descendants().any(|node| {
            node.kind() == SyntaxKind::Unary && node.text().to_string() == "!!!true"
        }));
        assert!(result.root.descendants().any(|node| {
            node.kind() == SyntaxKind::Unary && node.text().to_string() == "!!true"
        }));
        assert!(result.root.descendants().any(|node| {
            node.kind() == SyntaxKind::Unary && node.text().to_string() == "!true"
        }));
        for operator in ["|", "^", "&", "<<", "+", "*", "&&", "||", "!"] {
            assert!(
                result.root.descendants_with_tokens().any(|element| {
                    matches!(
                        element,
                        rowan::NodeOrToken::Token(token)
                            if token.kind() == SyntaxKind::Operator && token.text() == operator
                    )
                }),
                "missing operator token {operator}"
            );
        }
    }

    #[test]
    fn unary_negation_is_structural_lossless_and_span_exact() {
        let text = "%%start\ni32 negative = -7;\ni32 grouped = -(7);\ni32 nested = -~-!7;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);

        let unary_nodes = result
            .root
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::Unary)
            .collect::<Vec<_>>();
        assert_eq!(
            unary_nodes
                .iter()
                .map(|node| node.text().to_string())
                .collect::<Vec<_>>(),
            vec!["-7", "-(7)", "-~-!7", "~-!7", "-!7", "!7"]
        );
        assert_eq!(byte_span(&unary_nodes[0]), ByteSpan::new(23, 25));
        assert_eq!(byte_span(&unary_nodes[1]), ByteSpan::new(41, 45));
        assert_eq!(byte_span(&unary_nodes[2]), ByteSpan::new(60, 65));

        let nested = &unary_nodes[2];
        assert_eq!(
            nested
                .descendants_with_tokens()
                .filter_map(|element| match element {
                    rowan::NodeOrToken::Token(token) if token.kind() == SyntaxKind::Operator => {
                        Some((token.text().to_owned(), token.text_range()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .iter()
                .map(|(text, range)| {
                    (
                        text.clone(),
                        ByteSpan::new(u32::from(range.start()), u32::from(range.end())),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("-".to_owned(), ByteSpan::new(60, 61)),
                ("~".to_owned(), ByteSpan::new(61, 62)),
                ("-".to_owned(), ByteSpan::new(62, 63)),
                ("!".to_owned(), ByteSpan::new(63, 64)),
            ]
        );

        let integer = result
            .root
            .descendants_with_tokens()
            .find_map(|element| match element {
                rowan::NodeOrToken::Token(token)
                    if token.kind() == SyntaxKind::Integer && token.text() == "7" =>
                {
                    Some(token)
                }
                _ => None,
            })
            .expect("negative integer token");
        assert_eq!(u32::from(integer.text_range().start()), 24);
        assert_eq!(u32::from(integer.text_range().end()), 25);
    }

    #[test]
    fn binary_tiers_are_left_associative_and_parentheses_override_them() {
        let text = "%%start\ni32 value = 8 >> 1 >> 1;\ni32 grouped = 8 >> (1 >> 1);\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        let outer_shift = result
            .root
            .descendants()
            .find(|node| {
                node.kind() == SyntaxKind::Binary && node.text().to_string() == "8 >> 1 >> 1"
            })
            .expect("left-associative shift node");
        assert_eq!(
            outer_shift
                .descendants_with_tokens()
                .filter(|element| {
                    matches!(
                        element,
                        rowan::NodeOrToken::Token(token)
                            if token.kind() == SyntaxKind::Operator && token.text() == ">>"
                    )
                })
                .count(),
            2
        );
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Parenthesized && node.text() == "(1 >> 1)"));
    }

    #[test]
    fn namespace_and_qualified_call_are_structural_and_lossless() {
        let text =
            "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.add(20, 22);\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid());
        assert_eq!(result.reconstruct(), text);
        let namespace = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::NamespaceDecl)
            .expect("namespace declaration");
        assert_eq!(byte_span(&namespace), ByteSpan::new(8, 42));
        let call = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::Call)
            .expect("qualified call");
        assert_eq!(byte_span(&call), ByteSpan::new(56, 72));
        assert!(call
            .children()
            .any(|node| node.kind() == SyntaxKind::QualifiedCall));
    }

    #[test]
    fn generic_call_arguments_are_structural_lossless_and_ordered() {
        let text = "%%start\nu32 result = core.cast<u32>(value, \"exact\");\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let call = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::Call)
            .expect("generic call");
        assert_eq!(call.text(), "core.cast<u32>(value, \"exact\")");
        let generic = call
            .children()
            .find(|node| node.kind() == SyntaxKind::GenericTypeArguments)
            .expect("generic type arguments");
        assert_eq!(generic.text(), "<u32>");
        assert_eq!(byte_span(&generic), ByteSpan::new(30, 35));
        let argument = generic
            .children()
            .find(|node| node.kind() == SyntaxKind::GenericTypeArgument)
            .expect("generic type argument");
        assert_eq!(argument.text(), "u32");
        assert_eq!(byte_span(&argument), ByteSpan::new(31, 34));
        let children: Vec<_> = call
            .children_with_tokens()
            .filter_map(|element| match element {
                rowan::NodeOrToken::Node(node) => Some(node.kind()),
                rowan::NodeOrToken::Token(_) => None,
            })
            .collect();
        assert_eq!(
            children,
            vec![
                SyntaxKind::QualifiedCall,
                SyntaxKind::GenericTypeArguments,
                SyntaxKind::Expression,
                SyntaxKind::Expression,
            ]
        );
    }

    #[test]
    fn qualified_types_preserve_tokens_and_spans_in_every_type_position() {
        let text = "%%start\nchild = namespace app \"src/child.w\";\n*?child.Pair(child.Pair) convert = fn(value) { value };\nchild.Pair item = { .value = 1; };\ncore.cast<child.Pair>(item, \"exact\");\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let qualified = result
            .root
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::QualifiedType)
            .collect::<Vec<_>>();
        assert_eq!(qualified.len(), 4);
        assert!(qualified.iter().all(|node| node.text() == "child.Pair"));
        let first = &qualified[0];
        assert_eq!(byte_span(first), ByteSpan::new(47, 57));
        let tokens = first
            .children_with_tokens()
            .filter_map(|element| match element {
                rowan::NodeOrToken::Token(token) => Some((token.kind(), token.text().to_string())),
                rowan::NodeOrToken::Node(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tokens,
            vec![
                (SyntaxKind::Identifier, "child".to_owned()),
                (SyntaxKind::Punctuation, ".".to_owned()),
                (SyntaxKind::Identifier, "Pair".to_owned()),
            ]
        );
    }

    #[test]
    fn malformed_generic_call_recovers_losslessly() {
        let text = "%%start\nu32 result = core.cast<$>(value, \"exact\");\nu32 after = 1;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(!result.is_valid());
        assert_eq!(result.reconstruct(), text);
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Error));
        assert!(result
            .errors
            .iter()
            .any(|error| { error.span.start <= text.find("core.cast").expect("call") as u32 }));
    }

    #[test]
    fn unsafe_block_is_structured_and_lossless() {
        let text = "%%start\nunit() main = fn {\n\tunsafe {\n\t\tenv.read();\n\t}\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let unsafe_node = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::Unsafe)
            .expect("unsafe expression");
        assert!(unsafe_node
            .children()
            .any(|node| node.kind() == SyntaxKind::Block));
        assert_eq!(byte_span(&unsafe_node), ByteSpan::new(28, 53));
    }

    #[test]
    fn unsafe_helper_preserves_unterminated_final_expression() {
        let text = "%%start\nunsafe i32(i32, *?Iovec, *?i32) fd_write_once = fn(\n\tdescriptor,\n\tiovec_address,\n\tbyte_count_address\n) {\n\twasi.fd_write(descriptor, iovec_address, 1, byte_count_address)\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Unsafe));
        let final_list = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::FinalOutputList)
            .expect("unsafe helper final output list");
        let output_list = final_list
            .children()
            .find(|node| node.kind() == SyntaxKind::OutputList)
            .expect("unsafe helper output list");
        assert_eq!(
            output_list
                .children()
                .filter(|node| node.kind() == SyntaxKind::Expression)
                .map(|node| node.text().to_string().trim().to_owned())
                .collect::<Vec<_>>(),
            vec!["wasi.fd_write(descriptor, iovec_address, 1, byte_count_address)"]
        );
    }

    #[test]
    fn unit_if_is_structured_lossless_and_span_exact() {
        let text = "%%start\nunit() main = fn {\n\tif (true) {\n\t\tprint();\n\t}\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let conditional = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::UnitIfExpr)
            .expect("unit if expression");
        assert_eq!(conditional.text(), "if (true) {\n\t\tprint();\n\t}");
        assert_eq!(byte_span(&conditional), ByteSpan::new(28, 53));
    }

    #[test]
    fn qualified_member_read_and_assignment_are_structural_and_lossless() {
        let text = "%%start\nmath = namespace app \"src/math.w\";\ni32 result = math.value;\nmath.value = result;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid());
        assert_eq!(result.reconstruct(), text);
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::QualifiedMember));
        let assignment = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::Assignment)
            .expect("qualified assignment");
        assert_eq!(byte_span(&assignment), ByteSpan::new(68, 87));
        assert!(assignment.descendants().any(|node| {
            node.kind() == SyntaxKind::AssignmentTarget
                && node
                    .children()
                    .any(|child| child.kind() == SyntaxKind::QualifiedMember)
        }));
    }

    #[test]
    fn accepts_lossless_wasi_fd_write_declaration_and_utf8_calls() {
        let text = "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, i32) fd_write; };\ni32 out = wasi.fd_write(1, \"Olá\\n\");\ni32 err = wasi.fd_write(2, \"erro\\n\");\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        assert!(!result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::RawPointerType));
        assert!(!result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Unsafe));
        assert!(result
            .root
            .descendants_with_tokens()
            .any(|element| element.kind() == SyntaxKind::String));
    }

    #[test]
    fn preserves_raw_pointer_type_structure_and_utf8_source() {
        let text = "%%start\nraw = extern wasm \"raw\" { unsafe *?u8(i32, *?u8) read; };\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        assert_eq!(
            result
                .root
                .descendants()
                .filter(|node| node.kind() == SyntaxKind::RawPointerType)
                .count(),
            2
        );
    }

    #[test]
    fn local_binding_assignment_and_direct_block_while_are_structural() {
        let text = "%%start\ni32(i32) loop = fn(start) {\n\ti32 value = start;\n\twhile (value < 3) {\n\t\tvalue = value + 1;\n\t}\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid());
        assert_eq!(result.reconstruct(), text);
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::LocalBinding));
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Assignment));
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::While));
        let while_node = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::While)
            .expect("while node");
        assert_eq!(byte_span(&while_node), ByteSpan::new(57, 100));
        let trailing_semicolon =
            "%%start\ni32(i32) loop = fn(start) {\n\ti32 value = start;\n\twhile (value < 3) {\n\t\tvalue = value + 1;\n\t};\n};\n%%end";
        let rejected = parse(identity(), trailing_semicolon.into(), &[]);
        assert!(!rejected.is_valid());

        let expression_statement = "%%start\nunit() main = fn {\n\tprint();\n};\n%%end";
        assert!(parse(identity(), expression_statement.into(), &[]).is_valid());
    }

    #[test]
    fn final_output_list_preserves_order_and_spans() {
        let text = "%%start\n(i32, i32)(i32) pair = fn(value) {\n\tprint(value);\n\tvalue, value + 1\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);

        let final_list = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::FinalOutputList)
            .expect("final output list");
        let start = text.find("value, value + 1").expect("final list") as u32;
        let end = text.find("\n};").expect("function close") as u32 + 1;
        assert_eq!(byte_span(&final_list), ByteSpan::new(start, end));
        let output_list = final_list
            .children()
            .find(|node| node.kind() == SyntaxKind::OutputList)
            .expect("output list child");
        assert_eq!(
            output_list
                .children()
                .filter(|node| node.kind() == SyntaxKind::Expression)
                .map(|node| node.text().to_string().trim().to_owned())
                .collect::<Vec<_>>(),
            vec!["value", "value + 1"]
        );
        assert_eq!(
            output_list
                .children()
                .filter(|node| node.kind() == SyntaxKind::Expression)
                .map(|node| byte_span(&node))
                .collect::<Vec<_>>(),
            vec![
                ByteSpan::new(start, start + 5),
                ByteSpan::new(start + 7, end)
            ]
        );
        assert_eq!(byte_span(&output_list), byte_span(&final_list));
    }

    #[test]
    fn final_single_expression_and_terminated_items_keep_distinct_forms() {
        let single = "%%start\ni32() one = fn {\n\t1\n};\n%%end";
        let single_result = parse(identity(), single.into(), &[]);
        assert!(single_result.is_valid(), "{:?}", single_result.errors);
        assert_eq!(
            single_result
                .root
                .descendants()
                .filter(|node| node.kind() == SyntaxKind::FinalOutputList)
                .count(),
            1
        );

        let terminated = "%%start\nunit() many = fn {\n\tprint();\n\tlog();\n};\n%%end";
        let terminated_result = parse(identity(), terminated.into(), &[]);
        assert!(
            terminated_result.is_valid(),
            "{:?}",
            terminated_result.errors
        );
        assert_eq!(
            terminated_result
                .root
                .descendants()
                .filter(|node| node.kind() == SyntaxKind::FinalOutputList)
                .count(),
            0
        );
        assert_eq!(
            terminated_result
                .root
                .descendants()
                .filter(|node| node.kind() == SyntaxKind::BlockItem)
                .count(),
            2
        );
    }

    #[test]
    fn unterminated_control_flow_precedes_final_output_list() {
        let text = "%%start\ni32(i32, i32) f = fn(input) {\n\twhile (input < 2) {\n\t\tinput = input + 1;\n\t}\n\tif (input == 2) {\n\t\tinput;\n\t} else {\n\t\tinput;\n\t}\n\tinput, input + 1\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);

        let function = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::FunctionDecl)
            .expect("function declaration");
        let block = function
            .children()
            .find(|node| node.kind() == SyntaxKind::Block)
            .expect("function block");
        assert_eq!(
            block.children().map(|node| node.kind()).collect::<Vec<_>>(),
            vec![
                SyntaxKind::BlockItem,
                SyntaxKind::BlockItem,
                SyntaxKind::FinalOutputList,
            ]
        );

        let final_list = block
            .children()
            .find(|node| node.kind() == SyntaxKind::FinalOutputList)
            .expect("final output list");
        let start = text.find("input, input + 1").expect("final list") as u32;
        let end = text.find("\n};").expect("function close") as u32 + 1;
        assert_eq!(byte_span(&final_list), ByteSpan::new(start, end));
        assert_eq!(
            final_list
                .descendants()
                .filter(|node| node.kind() == SyntaxKind::Expression)
                .map(|node| node.text().to_string().trim().to_owned())
                .collect::<Vec<_>>(),
            vec!["input", "input + 1"]
        );
    }

    #[test]
    fn top_level_executable_items_are_structural_and_ordered() {
        let text = "%%start\ni32 value = 0;\nvalue = 1;\nvalue;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        let items: Vec<_> = result
            .root
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::TopLevelItem)
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text(), "value = 1;");
        assert_eq!(items[1].text(), "value;");
        assert!(byte_span(&items[0]).start < byte_span(&items[1]).start);
    }

    #[test]
    fn structured_values_and_raw_addresses_are_lossless() {
        let text = "%%start\nstruct WasiIovec {\n\t*?u8 buf;\n\tu32 len;\n}\n\nWasiIovec item = {\n\t.buf = null;\n\t.len = 0;\n};\n\nunsafe {\n\t*?WasiIovec address = &?item;\n\t*?u8 bytes = (*address).buf;\n\titem.len = 4;\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);

        let struct_decl = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::StructDecl)
            .expect("struct declaration");
        assert_eq!(byte_span(&struct_decl), ByteSpan::new(8, 49));
        let fields: Vec<_> = struct_decl
            .children()
            .filter(|node| node.kind() == SyntaxKind::StructField)
            .map(|node| node.text().to_string())
            .collect();
        assert_eq!(fields, vec!["*?u8 buf;", "u32 len;"]);
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::StructLiteral));
        assert_eq!(
            result
                .root
                .descendants()
                .filter(|node| node.kind() == SyntaxKind::StructLiteralField)
                .count(),
            2
        );
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::DereferencedField));
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::RawAddress));
        assert!(result
            .root
            .descendants()
            .any(|node| node.kind() == SyntaxKind::AssignmentTarget));
    }

    #[test]
    fn typed_multiple_output_receivers_are_structural_and_lossless() {
        let text = "%%start\nunsafe {\n\ti32 first, i32 second = pair();\n};\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let receiver_list = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::ReceiverList)
            .expect("receiver list");
        assert_eq!(
            receiver_list
                .children()
                .filter(|node| node.kind() == SyntaxKind::Receiver)
                .count(),
            2
        );
        let output_list = result
            .root
            .descendants()
            .find(|node| node.kind() == SyntaxKind::OutputList)
            .expect("output list");
        assert_eq!(byte_span(&output_list), ByteSpan::new(42, 48));
    }

    #[test]
    fn integer_radix_spellings_are_lossless_and_span_exact() {
        let text = "%%start\ni128 value = 0xAB_CD + 0b1010_0011 + 0o7_1;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let integers = result
            .root
            .descendants_with_tokens()
            .filter_map(|element| match element {
                rowan::NodeOrToken::Token(token) if token.kind() == SyntaxKind::Integer => Some((
                    token.text().to_owned(),
                    byte_span(&token.parent().expect("integer parent")),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(integers.len(), 3);
        assert_eq!(integers[0].0, "0xAB_CD");
        assert_eq!(integers[1].0, "0b1010_0011");
        assert_eq!(integers[2].0, "0o7_1");
    }

    #[test]
    fn character_literals_are_structural_lossless_and_span_exact() {
        let text = "%%start\nchar value = '\\u{1F600}';\nchar newline = '\\n';\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let literals = result
            .root
            .descendants_with_tokens()
            .filter_map(|element| match element {
                rowan::NodeOrToken::Token(token) if token.kind() == SyntaxKind::Char => Some((
                    token.text().to_owned(),
                    byte_span(&token.parent().expect("parent")),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(literals[0].0, "'\\u{1F600}'");
        assert_eq!(literals[0].1, ByteSpan::new(21, 32));
        assert_eq!(literals[1].0, "'\\n'");
    }

    #[test]
    fn floating_literals_are_lossless_and_span_exact() {
        let text = "%%start\nf32 a = 1_2.3_4e-1_0;\nf64 b = 2E+3;\n%%end";
        let result = parse(identity(), text.into(), &[]);
        assert!(result.is_valid(), "{:?}", result.errors);
        assert_eq!(result.reconstruct(), text);
        let literals = result
            .root
            .descendants_with_tokens()
            .filter_map(|element| match element {
                rowan::NodeOrToken::Token(token) if token.kind() == SyntaxKind::Float => Some((
                    token.text().to_owned(),
                    byte_span(&token.parent().expect("parent")),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(literals[0].0, "1_2.3_4e-1_0");
        assert_eq!(literals[1].0, "2E+3");
        assert_eq!(literals[0].1, ByteSpan::new(16, 28));
        assert_eq!(literals[1].1, ByteSpan::new(38, 42));
    }
}
