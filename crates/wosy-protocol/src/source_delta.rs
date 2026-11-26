//! Client-owned, selected-predecessor base CST delta. A proposal does not
//! confer a daemon lease, prepare compile-time IR, or publish a snapshot.
use super::{BuildSourceSelection, FileEntry, FileKey, Kind, Node, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const BASE_CST_DELTA_VERSION: u16 = 2;

/// The trusted local adapter's scope for one admission, captured from the
/// same SQLite transaction as the proposal. Never deserialized from a peer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseCstProposalBinding {
    pub selected: BuildSourceSelection,
    pub proposed: NodeId,
    pub attempt: i64,
}

/// Receipt from a trusted local source-cache adapter. A committed immutable
/// base-source root is visible and recomputable, not a semantic SnapshotRoot,
/// compile-time completion, or a Build/Check verdict.
#[derive(Debug, Eq, PartialEq)]
pub struct SourceDeltaCommit {
    pub binding: BaseCstProposalBinding,
    pub validated_files: usize,
    pub written_records: usize,
    pub already_present: bool,
    pub conflicts: usize,
}

pub enum SourceDeltaPublication {
    PredecessorMissing,
    Committed(SourceDeltaCommit),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaseCstProposal {
    pub version: u16,
    pub base: NodeId,
    pub root: NodeId,
    pub attempt: i64,
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

/// A canonical change partition. More than one file may use a single new
/// CST node, but only its first reachable changed file carries the record.
pub struct CstFilePartition {
    pub file: FileKey,
    pub old_file_root: Option<NodeId>,
    pub new_file_root: Option<NodeId>,
    pub added: Vec<NodeId>,
}

pub fn partition_cst_files<'a>(
    previous: &BTreeMap<FileKey, FileEntry>, proposed: &BTreeMap<FileKey, FileEntry>,
    mut novel: impl FnMut(NodeId) -> Option<&'a [u8]>,
) -> Result<Vec<CstFilePartition>, &'static str> {
    let mut files = Vec::new();
    let mut assigned = BTreeSet::new();
    for key in previous.keys().chain(proposed.keys()).collect::<BTreeSet<_>>() {
        let old_file_root = previous.get(key).map(|file| file.file);
        let current = proposed.get(key);
        let new_file_root = current.map(|file| file.file);
        if old_file_root == new_file_root { continue; }
        let mut added = Vec::new();
        if let Some(file) = current {
            let mut pending = vec![file.executable, file.authored];
            while let Some(id) = pending.pop() {
                let Some(bytes) = novel(id) else { continue; };
                if !assigned.insert(id) { continue; }
                pending.extend(Node::child_ids(bytes).ok_or("invalid packed CST node")?);
                added.push(id);
            }
        }
        files.push(CstFilePartition { file: key.clone(), old_file_root, new_file_root, added });
    }
    Ok(files)
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
