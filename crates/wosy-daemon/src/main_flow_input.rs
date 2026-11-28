//! A read-only source handoff for the common MainFlowOnly grammar.
//! Dev6 does not own the canonical proof checker or any Check verdict.
use super::{source_proposal::same_cst, ConfiguredSourceView, Result, SourceStore};
use std::sync::Arc;
use wosy_project::TargetLayout;
use wosy_protocol::{BuildSourceSelection, NodeId};
use wosy_syntax::{CstNode, SourceIdentity, SyntaxKind as Kind};

#[derive(Debug, Eq, PartialEq)]
pub enum MainFlowInputRefusal {
    SourceUnavailable,
    ProfileNotSelected,
    OutsideCommonScope,
    UnverifiedGrammar,
}

pub enum MainFlowInputPreparation {
    Ready(Arc<MainFlowSourceInput>),
    Refused(MainFlowInputRefusal),
}

/// Authored input only. The owner of a different parser must verify its own
/// canonical grammar and semantic shape before deriving typed declarations.
/// This type never contains foreign CST IDs, a proof, or a reusable verdict.
pub struct MainFlowSourceInput {
    selection: BuildSourceSelection,
    profile: String,
    layout: TargetLayout,
    source: SourceIdentity,
    text: String,
    categories: Vec<String>,
    shape: NodeId,
    identity: NodeId,
}

impl MainFlowSourceInput {
    pub fn selection(&self) -> &BuildSourceSelection { &self.selection }
    pub fn profile(&self) -> &str { &self.profile }
    pub fn layout(&self) -> TargetLayout { self.layout }
    pub fn source(&self) -> &SourceIdentity { &self.source }
    pub fn text(&self) -> &str { &self.text }
    pub fn categories(&self) -> &[String] { &self.categories }
    pub fn shape(&self) -> NodeId { self.shape }
    pub fn identity(&self) -> NodeId { self.identity }
}

impl SourceStore {
    /// Prepare a narrow, exact root-only authored input. This deliberately
    /// does not use Build-analysis admission, its diagnostics, or its keys.
    /// A previously owned packet is reusable only as source preparation.
    #[doc(hidden)]
    pub fn prepare_common_main_flow_source(
        &self, selected: &BuildSourceSelection, profile: &str, prior: Option<&Arc<MainFlowSourceInput>>,
    ) -> Result<MainFlowInputPreparation> {
        use MainFlowInputPreparation::{Ready, Refused};
        use MainFlowInputRefusal::*;
        let Some(view) = self.snapshot(selected.root)? else { return Ok(Refused(SourceUnavailable)); };
        let ConfiguredSourceView { source: view, config } = view.configured()?;
        let entry = config.namespaces.get(&config.entry.package).ok_or("missing selected entry namespace")?;
        if entry.git.is_some() || entry.project != selected.project || config.target != selected.target {
            return Ok(Refused(SourceUnavailable));
        }
        if config.parser.schema.0 != *wosy_syntax::base_cst_schema() { return Ok(Refused(UnverifiedGrammar)); }
        let Some(artifact) = config.profiles.get(profile) else { return Ok(Refused(ProfileNotSelected)); };
        if artifact.exports.is_some() || view.files.len() != 1
            || !view.module_edges.get(&config.entry).is_some_and(Vec::is_empty)
        {
            return Ok(Refused(OutsideCommonScope));
        }
        let layout = artifact.target_layout()?;
        if let Some(prior) = prior.filter(|prior| &prior.selection == selected && prior.profile == profile) {
            return Ok(Ready(Arc::clone(prior)));
        }
        let authored = view.authored(&config.entry)?;
        let text = authored.cst.root.text().to_string();
        let fresh = wosy_syntax::parse_with_snapshot(
            authored.cst.source.clone(), text, &config.parser.categories,
        ).into_result();
        if !fresh.is_valid() || fresh.comments != authored.comments
            || !matches!(same_cst(&authored.cst.root, &fresh.canonical_root.root), Some(_))
        {
            return Ok(Refused(UnverifiedGrammar));
        }
        let Some(shape) = common_shape(&fresh.canonical_root.root, &fresh.text) else { return Ok(Refused(OutsideCommonScope)); };
        let mut identity = blake3::Hasher::new_derive_key("Wosy common MainFlowOnly authored input v1");
        identity.update(&selected.root.0).update(&config.parser.id().0).update(&shape.0)
            .update(&layout.pointer_size.to_be_bytes()).update(&layout.pointer_alignment.to_be_bytes())
            .update(profile.as_bytes());
        Ok(Ready(Arc::new(MainFlowSourceInput {
            selection: selected.clone(), profile: profile.to_owned(), layout, source: authored.cst.source,
            text: fresh.text, categories: config.parser.categories, shape,
            identity: NodeId(*identity.finalize().as_bytes()),
        })))
    }
}

// The tag alphabet is semantic, not SyntaxKind's discriminants or Debug
// spellings. Each receiver must explicitly bind its parser's node AND token
// roles; a raw number, unknown role, or missing occurrence cannot qualify.
fn common_shape(root: &CstNode, authored: &str) -> Option<NodeId> {
    let mut hash = blake3::Hasher::new_derive_key("Wosy common MainFlowOnly semantic occurrences v1");
    hash.update(blake3::hash(authored.as_bytes()).as_bytes());
    for event in root.preorder_with_tokens() {
        use rowan::WalkEvent::*;
        let (edge, occurrence) = match event {
            Enter(node) => (0, node),
            Leave(node) if node.as_node().is_some() => (1, node),
            Leave(_) => continue,
        };
        let kind = match (occurrence.as_node().is_some(), occurrence.kind()) {
            (true, Kind::Root) => 0,
            (true, Kind::Item) => 1,
            (true, Kind::TopLevelItem) => 2,
            (true, Kind::Call) => 3,
            (true, Kind::QualifiedCall) => 4,
            (true, Kind::Expression) => 5,
            (true, Kind::Primary) => 6,
            (true, Kind::StructLiteral) => 7,
            (true, Kind::StructLiteralField) => 8,
            (true, Kind::ArrayLiteral) => 9,
            (false, Kind::BoundaryStart) => 10,
            (false, Kind::BoundaryEnd) => 11,
            (false, Kind::TypedComment) => 12,
            (false, Kind::LineEnding) => 13,
            (false, Kind::Whitespace) => 14,
            (false, Kind::Punctuation) => 15,
            (false, Kind::Identifier) => 16,
            (false, Kind::String) => 17,
            (false, Kind::Integer) => 18,
            (false, Kind::Boolean) => 19,
            (false, Kind::ArrayLiteral) => 20,
            _ => return None,
        };
        let range = occurrence.text_range();
        hash.update(&[edge, kind]).update(&u32::from(range.start()).to_be_bytes())
            .update(&u32::from(range.end()).to_be_bytes());
    }
    Some(NodeId(*hash.finalize().as_bytes()))
}
