//! Content-addressed records for the base CST preparation protocol.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wosy_project::{ByteSpan, PackageRelativePath, SourceIdentity};

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
}

impl Kind {
    fn from_byte(tag: u8) -> Option<Self> {
        use Kind::*;
        let tags = [
            Token, Syntax, Annotations, File, ExecutableFile, AnnotationFile, Executables,
            AnnotationTree, Dependencies, Module, Config, SourcePreparation,
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
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(9 + self.data.len() + 32 * self.children.len());
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&(self.data.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&self.data);
        for child in &self.children {
            bytes.extend_from_slice(&child.0);
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let kind = Kind::from_byte(*bytes.first()?)?;
        let len = usize::try_from(u64::from_be_bytes(bytes.get(1..9)?.try_into().ok()?)).ok()?;
        let (data, links) = bytes.get(9..)?.split_at_checked(len)?;
        let (chunks, tail) = links.as_chunks::<32>();
        tail.is_empty().then(|| Self {
            kind,
            data: data.to_vec(),
            children: chunks.iter().copied().map(NodeId).collect(),
        })
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub origin: String,
    pub path: PackageRelativePath,
    pub binding: String,
    pub span: ByteSpan,
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
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub entry: FileKey,
    pub namespaces: BTreeMap<String, Namespace>,
    pub target: String,
    pub inputs: NodeId,
    pub parser: ParserPolicy,
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
