//! Content-addressed records for the base CST preparation protocol.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wosy_project::packages::{LockedInputId, PatchStackId};
use wosy_project::{ArtifactProfile, ByteSpan, PackageRelativePath, SourceIdentity};

mod build_diagnostics;
pub use build_diagnostics::{
    BuildAnalysisOrigin, BuildAnalysisPhase, BuildDiagnosticRequest, BuildDiagnosticResponse, BuildSourceSelection, BuildSessionCommand, BuildSessionCommandId,
    BuildSessionRefusal, BuildSessionReply, BuildSessionRequest, BuildSessionResponse, BUILD_DIAGNOSTIC_VERSION,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    pub fn hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }

    pub fn parse_hex(text: &str) -> Result<Self, String> {
        blake3::Hash::from_hex(text)
            .map(|hash| Self(*hash.as_bytes()))
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[repr(u8)]
pub enum Kind {
    Token = 1,
    Syntax,
    Annotations,
    File,
    ExecutableFile,
    AnnotationFile,
    Executables,
    AnnotationTree,
    Dependencies,
    Module,
    Config,
    SourcePreparation,
    NamespaceFacts,
    BaseDeclarations,
    ModuleDeclarations,
    BuildDiagnosticReceipt,
}

impl Kind {
    fn from_byte(tag: u8) -> Option<Self> {
        use Kind::*;
        let tags = [
            Token, Syntax, Annotations, File, ExecutableFile, AnnotationFile, Executables,
            AnnotationTree, Dependencies, Module, Config, SourcePreparation, NamespaceFacts,
            BaseDeclarations, ModuleDeclarations, BuildDiagnosticReceipt,
        ];
        tags.get(tag.checked_sub(1)? as usize).copied()
    }
}

/// Children are ordered and may repeat, including two identical adjacent tokens.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Node {
    pub kind: Kind,
    pub data: Vec<u8>,
    pub children: Vec<NodeId>,
}

impl Node {
    pub fn encoded_len(&self) -> usize {
        9 + self.data.len() + 32 * self.children.len()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.encoded_len());
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&(self.data.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&self.data);
        for child in &self.children {
            bytes.extend_from_slice(&child.0);
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (kind, data, children) = Self::parts(bytes)?;
        Some(Self {
            kind,
            data: data.to_vec(),
            children: children.iter().copied().map(NodeId).collect(),
        })
    }

    /// Walk the same binary record's links without copying its payload.
    pub fn child_ids(bytes: &[u8]) -> Option<impl Iterator<Item = NodeId> + '_> {
        let (_, _, children) = Self::parts(bytes)?;
        Some(children.iter().copied().map(NodeId))
    }

    fn parts(bytes: &[u8]) -> Option<(Kind, &[u8], &[[u8; 32]])> {
        let kind = Kind::from_byte(*bytes.first()?)?;
        let len = usize::try_from(u64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?)).ok()?;
        let (data, links) = bytes.get(9..)?.split_at_checked(len)?;
        let (children, tail) = links.as_chunks::<32>();
        tail.is_empty().then_some((kind, data, children))
    }

    pub fn id(bytes: &[u8]) -> NodeId {
        NodeId(blake3::derive_key("wosy base source preparation node v2", bytes))
    }
}

#[derive(Default)]
pub struct Delta {
    pub nodes: BTreeMap<NodeId, Vec<u8>>,
}

impl Delta {
    pub fn add(&mut self, kind: Kind, data: Vec<u8>, children: Vec<NodeId>) -> NodeId {
        let bytes = Node { kind, data, children }.encode();
        let id = Node::id(&bytes);
        self.nodes.entry(id).or_insert(bytes);
        id
    }
}

/// In-process handoff for an admitted client candidate. The predecessor
/// payload can satisfy recovery if the store no longer knows its root.
/// This is not a wire format or a transport capability.
#[derive(Clone, Copy)]
pub struct SourceUpdate<'a> {
    pub base: NodeId,
    pub root: NodeId,
    pub added: Option<&'a Delta>,
    pub base_payload: &'a Delta,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileKey {
    pub package: String,
    pub path: PackageRelativePath,
}

impl From<&SourceIdentity> for FileKey {
    fn from(source: &SourceIdentity) -> Self {
        Self { package: source.package.clone(), path: source.path.clone() }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub origin: String,
    pub path: PackageRelativePath,
    pub binding: String,
    pub span: ByteSpan,
}

impl Import {
    pub fn from_tokens(
        origin: &str, path: &str, binding: &str, start: u32, end: u32,
    ) -> Result<Self, String> {
        let path: String = serde_json::from_str(path)
            .map_err(|error| format!("invalid namespace path: {error}"))?;
        Ok(Self {
            origin: origin.to_owned(),
            path: PackageRelativePath::new(path.into())?,
            binding: binding.to_owned(),
            span: ByteSpan::new(start, end),
        })
    }
}

/// Fetch hint projection and canonical namespace facts use the same typed CST
/// access as ordinary compiler namespace declaration derivation.
pub fn imports_from_cst(root: &wosy_syntax::CstNode) -> Result<Vec<Import>, String> {
    wosy_syntax::namespace_nodes(root)
        .map(|node| {
            let ns = wosy_syntax::NamespaceCst::from_node(&node)
                .ok_or("namespace declaration tokens are missing")?;
            Import::from_tokens(ns.package.text(), ns.path.text(), ns.binding.text(),
                ns.span.start, ns.span.end)
        })
        .collect()
}

pub fn source_preparation_schema() -> &'static [u8; 32] {
    static ID: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    ID.get_or_init(|| blake3::derive_key(
        "wosy source preparation with canonical namespace authority v1",
        wosy_syntax::base_cst_schema(),
    ))
}

pub fn installed_git_source_schema() -> &'static [u8; 32] {
    static ID: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    ID.get_or_init(|| blake3::derive_key(
        "wosy source preparation with installed Git selection v1",
        source_preparation_schema(),
    ))
}

/// The root layout shared by the local CST store and the daemon. File
/// provenance is indexed by these exact children, not by directory contents.
pub struct SourcePreparationParts<'a> {
    pub config: NodeId,
    pub executable: NodeId,
    pub annotation: NodeId,
    pub dependencies: NodeId,
    pub files: &'a [NodeId],
}

pub fn source_preparation_parts(node: &Node) -> Option<SourcePreparationParts<'_>> {
    if node.kind != Kind::SourcePreparation
        || (node.data != source_preparation_schema() && node.data != installed_git_source_schema())
    {
        return None;
    }
    let [config, executable, annotation, dependencies, files @ ..] = node.children.as_slice() else {
        return None;
    };
    if files.is_empty() { return None; }
    Some(SourcePreparationParts {
        config: *config, executable: *executable, annotation: *annotation,
        dependencies: *dependencies, files,
    })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParserPolicy {
    pub schema: NodeId,
    pub categories: Vec<String>,
}

impl ParserPolicy {
    pub fn id(&self) -> NodeId {
        let bytes = serde_json::to_vec(self).expect("serializable parser policy");
        NodeId(blake3::derive_key("wosy directive and parser policy v2", &bytes))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Namespace {
    pub project: String,
    pub source_root: Option<PackageRelativePath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<InstalledGitSelection>,
}

/// The resolved lock and installed patch stack. It does not attest that the
/// current editable checkout's observed bytes equal either pristine or replayed bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledGitSelection {
    pub locked: LockedInputId,
    pub patches: PatchStackId,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub entry: FileKey,
    pub namespaces: BTreeMap<String, Namespace>,
    pub target: String,
    pub inputs: NodeId,
    pub parser: ParserPolicy,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, ArtifactProfile>,
}

impl SourceConfig {
    pub fn schema(&self) -> &'static [u8; 32] {
        if self.namespaces.values().any(|ns| ns.git.is_some()) {
            installed_git_source_schema()
        } else {
            source_preparation_schema()
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileMetadata {
    pub source: SourceIdentity,
    pub revision: NodeId,
    pub policy: NodeId,
    pub imports: Vec<Import>,
    pub bytes: u32,
}

/// A per-file provenance record; the canonical node IDs stay context-free.
#[derive(Clone)]
pub struct FileEntry {
    pub metadata: FileMetadata,
    pub file: NodeId,
    pub authored: NodeId,
    pub executable: NodeId,
    pub annotation: NodeId,
}

impl FileEntry {
    pub fn from_node(file: NodeId, node: Node) -> Result<Self, String> {
        if node.kind != Kind::File {
            return Err("expected file provenance".to_owned());
        }
        let metadata = serde_json::from_slice(&node.data).map_err(|error| error.to_string())?;
        let [authored, executable, annotation] =
            node.children.try_into().map_err(|_| "invalid file roots")?;
        Ok(Self { metadata, file, authored, executable, annotation })
    }
}
