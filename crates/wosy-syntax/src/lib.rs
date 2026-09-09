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
    Expression,
    Call,
    IfExpr,
    Parenthesized,
    Boolean,
    Binary,
    Operator,
    Integer,
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
            22 => SyntaxKind::Expression,
            23 => SyntaxKind::Call,
            24 => SyntaxKind::IfExpr,
            25 => SyntaxKind::Parenthesized,
            26 => SyntaxKind::Boolean,
            27 => SyntaxKind::Binary,
            28 => SyntaxKind::Operator,
            29 => SyntaxKind::Integer,
            30 => SyntaxKind::String,
            31 => SyntaxKind::Punctuation,
            32 => SyntaxKind::NamespaceDecl,
            33 => SyntaxKind::QualifiedCall,
            34 => SyntaxKind::QualifiedMember,
            35 => SyntaxKind::AssignmentTarget,
            36 => SyntaxKind::LocalBinding,
            37 => SyntaxKind::While,
            38 => SyntaxKind::Assignment,
            39 => SyntaxKind::TopLevelItem,
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
        Rule::raw_pointer_type => SyntaxKind::RawPointerType,
        Rule::unsafe_marker => SyntaxKind::Unsafe,
        Rule::identifier => SyntaxKind::Identifier,
        Rule::parameters => SyntaxKind::Parameters,
        Rule::block => SyntaxKind::Block,
        Rule::block_item => SyntaxKind::BlockItem,
        Rule::expression => SyntaxKind::Expression,
        Rule::call => SyntaxKind::Call,
        Rule::qualified_call => SyntaxKind::QualifiedCall,
        Rule::qualified_member => SyntaxKind::QualifiedMember,
        Rule::assignment_target => SyntaxKind::AssignmentTarget,
        Rule::if_expr => SyntaxKind::IfExpr,
        Rule::unsafe_expr => SyntaxKind::Unsafe,
        Rule::while_expr => SyntaxKind::While,
        Rule::assignment => SyntaxKind::Assignment,
        Rule::top_level_item => SyntaxKind::TopLevelItem,
        Rule::error_item => SyntaxKind::Error,
        Rule::parenthesized => SyntaxKind::Parenthesized,
        Rule::boolean => SyntaxKind::Boolean,
        Rule::logical_or
        | Rule::logical_and
        | Rule::comparison
        | Rule::additive
        | Rule::multiplicative => SyntaxKind::Binary,
        Rule::or_operator
        | Rule::and_operator
        | Rule::comparison_operator
        | Rule::add_operator
        | Rule::multiply_operator
        | Rule::operator => SyntaxKind::Operator,
        Rule::integer => SyntaxKind::Integer,
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
            SyntaxKind::Call,
            SyntaxKind::QualifiedCall,
            SyntaxKind::Binary,
            SyntaxKind::Operator,
            SyntaxKind::Integer,
            SyntaxKind::String,
            SyntaxKind::Punctuation,
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
        let text = "%%start\nwasi = extern wasm \"wasi_snapshot_preview1\" { i32(i32, utf8) fd_write; };\ni32 out = wasi.fd_write(1, \"Olá\\n\");\ni32 err = wasi.fd_write(2, \"erro\\n\");\n%%end";
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
}
