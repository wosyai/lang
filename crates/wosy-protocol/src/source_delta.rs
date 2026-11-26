//! Client-owned, selected-predecessor base CST delta. A proposal does not
//! confer a daemon lease, prepare compile-time IR, or publish a snapshot.
use super::{FileKey, Kind, Node, NodeId};
use serde::{Deserialize, Serialize};

pub const BASE_CST_DELTA_VERSION: u16 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaseCstProposal {
    pub version: u16,
    pub base: NodeId,
    pub root: NodeId,
    pub files: Vec<CstDelta>,
    pub source_index: Vec<SourceIndexAddition>,
}

/// A change in file provenance: each root identifies the authored CST,
/// executable CST, and typed annotation projection. Missing prior/new roots
/// represent additions/removals. Structural nodes shared by multiple files
/// are included once, at the first reachable changed file in key order.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CstDelta {
    pub file: FileKey,
    pub old_file_root: Option<NodeId>,
    pub new_file_root: Option<NodeId>,
    pub added: Vec<CstAddition>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackedSourceNode {
    id: NodeId,
    bytes: Vec<u8>,
}

/// The underlying canonical record encoding is shared with the source graph.
/// The DTO's kind check does not verify digest, grammar, provenance, or closure
/// at a receiving peer.
#[derive(Debug, Deserialize, Serialize)]
#[serde(try_from = "PackedSourceNode")]
pub struct CstAddition(PackedSourceNode);

impl CstAddition {
    pub fn from_canonical(id: NodeId, bytes: Vec<u8>) -> Result<Self, &'static str> {
        PackedSourceNode { id, bytes }.try_into()
    }

    pub fn id(&self) -> NodeId { self.0.id }

    pub fn encoded(&self) -> &[u8] { &self.0.bytes }
}

impl TryFrom<PackedSourceNode> for CstAddition {
    type Error = &'static str;

    fn try_from(node: PackedSourceNode) -> Result<Self, Self::Error> {
        match Node::parts(&node.bytes).map(|parts| parts.0) {
            Some(Kind::Syntax | Kind::Token) => Ok(Self(node)),
            _ => Err("expected a canonical CST node or token"),
        }
    }
}

/// Configured source graph and provenance records, never compiler-owned AST,
/// diagnostics, namespace facts, artifacts, or IR.
#[derive(Debug, Deserialize, Serialize)]
#[serde(try_from = "PackedSourceNode")]
pub struct SourceIndexAddition(PackedSourceNode);

impl SourceIndexAddition {
    pub fn from_canonical(id: NodeId, bytes: Vec<u8>) -> Result<Self, &'static str> {
        PackedSourceNode { id, bytes }.try_into()
    }

    pub fn id(&self) -> NodeId { self.0.id }

    pub fn encoded(&self) -> &[u8] { &self.0.bytes }
}

impl TryFrom<PackedSourceNode> for SourceIndexAddition {
    type Error = &'static str;

    fn try_from(node: PackedSourceNode) -> Result<Self, Self::Error> {
        use Kind::*;
        match Node::parts(&node.bytes).map(|parts| parts.0) {
            Some(Annotations | File | ExecutableFile | AnnotationFile | Executables
                | AnnotationTree | Dependencies | Module | Config | SourcePreparation) => Ok(Self(node)),
            _ => Err("expected a canonical source-index record"),
        }
    }
}
