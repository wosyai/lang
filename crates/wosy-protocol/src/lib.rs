//! Content-addressed records for the base CST preparation protocol.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wosy_project::packages::{LockedInputId, PatchStackId};
use wosy_project::{ArtifactProfile, ByteSpan, PackageRelativePath, SourceIdentity};

mod source_delta;
pub use source_delta::{
    partition_cst_files, BaseCstProposal, BaseCstProposalBinding, CstAddition, CstDelta,
    SourceDeltaCommit, SourceDeltaPublication, SourceIndexAddition, BASE_CST_DELTA_VERSION,
};

mod build_diagnostics;
pub use build_diagnostics::{
    BaseDeclarationConnectionId, BaseDeclarationProjection, BuildAnalysisOrigin, BuildAnalysisPhase, BuildDiagnosticRequest,
    BuildDiagnosticResponse, BuildSourceSelection, BuildSessionCommand, BuildSessionCommandId,
    BuildSessionRefusal, BuildSessionReply, BuildSessionRequest, BuildSessionResponse, OrdinaryLlvmBuilderWitness, OrdinaryLlvmProjection,
    BASE_DECLARATION_PROJECTION_VERSION, BUILD_DIAGNOSTIC_VERSION, BUILD_RECONCILIATION_VERSION,
    MAX_ORDINARY_LLVM_PROFILE_BYTES, ORDINARY_EXECUTABLE_BUILDER_INPUT_VERSION, ORDINARY_LLVM_BUILDER_INPUT_VERSION, ORDINARY_LLVM_PROJECTION_VERSION,
};
#[cfg(all(target_os = "linux", feature = "native-diagnostics"))]
pub mod native_diagnostics;

/// Digest exactly the independently authorized local compiler image bytes.
pub fn compiler_image_digest(reader: impl std::io::Read) -> std::io::Result<NodeId> {
    let mut hasher = blake3::Hasher::new_derive_key("wosy local derivation executable image v1");
    hasher.update_reader(reader)?;
    Ok(NodeId(*hasher.finalize().as_bytes()))
}

mod session_frame;
pub use session_frame::{encode_session_frame, write_encoded_session_frame, SessionFrameError, SessionFrameReader, MAX_SESSION_FRAME_BYTES};
mod cst_walk;
pub use cst_walk::{syntax_kind, token_text, walk_cst, CstEvent, CstExpansionBudget};

/// Trusted local adapter data, not a wire command or an authorization token.
/// This describes named base source, not a final semantic SnapshotRoot pin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePinIntent {
    pub source: BuildSourceSelection,
    pub name: String,
    pub generation: i64,
    pub retention: SourcePinRetention,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcePinRetention {
    Retained,
    Released,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SourcePinReflection {
    ReplayRequired,
    Superseded,
    NameHeldByDirectPin,
    Durable(SourcePinIntent),
}

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
    SavedBuildDiagnostic,
}

impl Kind {
    fn from_byte(tag: u8) -> Option<Self> {
        use Kind::*;
        let tags = [
            Token, Syntax, Annotations, File, ExecutableFile, AnnotationFile, Executables,
            AnnotationTree, Dependencies, Module, Config, SourcePreparation, NamespaceFacts,
            BaseDeclarations, ModuleDeclarations, BuildDiagnosticReceipt, SavedBuildDiagnostic,
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
    pub envelope: SourcePreparationEnvelope,
    pub config: NodeId,
    pub executable: NodeId,
    pub annotation: NodeId,
    pub dependencies: NodeId,
    pub files: &'a [NodeId],
    grammar: SourceEnvelopeGrammar,
}

impl SourcePreparationParts<'_> {
    pub fn matches_config(&self, config: &SourceConfig) -> bool {
        self.envelope_identity() == config.envelope_identity()
    }

    pub fn kind_encoding(&self) -> CstKindEncoding { self.grammar.kind_encoding }

    pub fn envelope_identity(&self) -> SourceEnvelopeIdentity {
        SourceEnvelopeIdentity { family: self.envelope, parser: NodeId(*self.grammar.parser) }
    }
}

/// Structural layout and parser stamp, not workspace or compile authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceEnvelopeIdentity {
    family: SourcePreparationEnvelope,
    parser: NodeId,
}

/// A published source envelope family. A decoded envelope identifies a
/// layout, not a validated source scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourcePreparationEnvelope {
    NamespaceAuthority,
    InstalledGit,
}

impl SourcePreparationEnvelope {
    fn digest(self) -> &'static [u8; 32] {
        match self {
            Self::NamespaceAuthority => source_preparation_schema(),
            Self::InstalledGit => installed_git_source_schema(),
        }
    }

    /// Match this exact root layout to the already decoded configuration.
    /// Owner, target and per-file provenance still require rooted checks.
    pub fn matches_config(self, config: &SourceConfig) -> bool {
        self == config.envelope() && config.parser.schema.0 == *wosy_syntax::base_cst_schema()
    }
}

/// Allowed Rowan kind IDs for the decoded authored view, not parser or
/// execution authority. The shared typed-comment scanner must preserve
/// the V3 comment contract for this frozen reader.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CstKindEncoding {
    Current,
    FrozenBaseV3,
}

impl CstKindEncoding {
    pub fn allows(self, kind: wosy_syntax::SyntaxKind) -> bool { self.allows_raw(kind as u16) }

    /// Keys for per-file namespace facts have always used the ordinary
    /// envelope's digest, even for the installed-Git root family.
    pub fn namespace_authority_schema(self) -> &'static [u8; 32] {
        match self {
            Self::Current => source_preparation_schema(),
            Self::FrozenBaseV3 => SourceEnvelopeGrammar::FROZEN_BASE_V3.namespace,
        }
    }

    fn allows_raw(self, raw: u16) -> bool {
        match self {
            Self::Current => true,
            Self::FrozenBaseV3 => raw <= 84, // Last published tag: InvalidTemporaryPlace.
        }
    }
}

#[derive(Clone, Copy)]
struct SourceEnvelopeGrammar {
    parser: &'static [u8; 32],
    namespace: &'static [u8; 32],
    git: &'static [u8; 32],
    kind_encoding: CstKindEncoding,
}

impl SourceEnvelopeGrammar {
    fn current() -> Self {
        static CURRENT: std::sync::OnceLock<SourceEnvelopeGrammar> = std::sync::OnceLock::new();
        *CURRENT.get_or_init(|| Self {
            parser: wosy_syntax::base_cst_schema(), namespace: source_preparation_schema(),
            git: installed_git_source_schema(), kind_encoding: CstKindEncoding::Current,
        })
    }

    // Actual V3 grammar blob e708b3395f545b877f42fea8782cc769727f0ea7,
    // released with its current scanner and Rowan encoding. Do not derive
    // these read-only fingerprints from the grammar of a future compiler.
    const FROZEN_BASE_V3: Self = Self {
        parser: &[
            0x69, 0x5f, 0xe4, 0xab, 0x34, 0x7a, 0x9e, 0xda, 0x10, 0xb4, 0x8e, 0x64, 0xae, 0x4a, 0xab, 0x1d,
            0xfc, 0xd5, 0x8a, 0xb8, 0xbf, 0x55, 0x4a, 0x1b, 0xcb, 0xb9, 0xfa, 0x33, 0x7c, 0xfb, 0xbb, 0xd6,
        ],
        namespace: &[
            0x05, 0xac, 0xbf, 0x05, 0x06, 0x45, 0x63, 0xce, 0x20, 0x6e, 0x10, 0x74, 0xf3, 0xac, 0x47, 0x93,
            0x48, 0xb8, 0xc6, 0x35, 0xbf, 0xa1, 0x21, 0x23, 0x4f, 0xb7, 0x11, 0x1a, 0xb5, 0x89, 0xc3, 0x24,
        ],
        git: &[
            0x4d, 0x5c, 0x27, 0xd1, 0x40, 0x3f, 0x5c, 0xd5, 0x4c, 0xda, 0xb5, 0x3f, 0x18, 0x08, 0xe6, 0x01,
            0x9b, 0x9d, 0x2b, 0x06, 0x8b, 0xb9, 0x89, 0x23, 0x0b, 0x7c, 0x94, 0x48, 0x34, 0xe3, 0x4f, 0xf8,
        ],
        kind_encoding: CstKindEncoding::FrozenBaseV3,
    };

    fn classify(self, data: &[u8]) -> Option<SourcePreparationEnvelope> {
        if data == self.namespace { Some(SourcePreparationEnvelope::NamespaceAuthority) }
        else if data == self.git { Some(SourcePreparationEnvelope::InstalledGit) }
        else { None }
    }
}

pub fn source_preparation_parts(node: &Node) -> Option<SourcePreparationParts<'_>> {
    source_preparation_parts_using(node, SourceEnvelopeGrammar::current(), false)
}

/// Read-only metadata for archived source or deciding to observe current
/// source from scratch. Callers still authenticate the rooted owner and
/// scope. This does not admit a frozen grammar as active, deserialized
/// compiler input, or source replay.
pub fn retained_source_preparation_metadata(node: &Node) -> Option<SourcePreparationParts<'_>> {
    source_preparation_parts_using(node, SourceEnvelopeGrammar::current(), true)
}

fn source_preparation_parts_using(
    node: &Node, current: SourceEnvelopeGrammar, allow_frozen_read: bool,
) -> Option<SourcePreparationParts<'_>> {
    if node.kind != Kind::SourcePreparation { return None; }
    let frozen = SourceEnvelopeGrammar::FROZEN_BASE_V3;
    let (envelope, grammar) = current.classify(&node.data).map(|envelope| (envelope, current))
        .or_else(|| allow_frozen_read.then(|| frozen.classify(&node.data)).flatten()
            .map(|envelope| (envelope, frozen)))?;
    let [config, executable, annotation, dependencies, files @ ..] = node.children.as_slice() else {
        return None;
    };
    if files.is_empty() { return None; }
    Some(SourcePreparationParts {
        envelope,
        config: *config, executable: *executable, annotation: *annotation,
        dependencies: *dependencies, files, grammar,
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
    fn envelope(&self) -> SourcePreparationEnvelope {
        if self.namespaces.values().any(|ns| ns.git.is_some()) {
            SourcePreparationEnvelope::InstalledGit
        } else {
            SourcePreparationEnvelope::NamespaceAuthority
        }
    }

    pub fn schema(&self) -> &'static [u8; 32] { self.envelope().digest() }

    pub fn envelope_identity(&self) -> SourceEnvelopeIdentity {
        SourceEnvelopeIdentity { family: self.envelope(), parser: self.parser.schema }
    }
}

#[cfg(test)]
mod source_envelope_tests {
    use super::*;

    fn config(parser: &[u8; 32]) -> SourceConfig {
        SourceConfig {
            entry: FileKey { package: "app".into(), path: PackageRelativePath::new("src/main.w".into()).unwrap() },
            namespaces: BTreeMap::from([("app".into(), Namespace {
                project: "/workspace/app".into(), source_root: None, git: None,
            })]),
            target: "app".into(),
            inputs: NodeId([2; 32]),
            parser: ParserPolicy { schema: NodeId(*parser), categories: vec!["WHY".into()] },
            profiles: BTreeMap::new(),
        }
    }

    fn top(data: &[u8; 32]) -> Node {
        Node {
            kind: Kind::SourcePreparation, data: data.to_vec(), children: (1..=5).map(|byte| NodeId([byte; 32])).collect(),
        }
    }

    fn add_git_namespace(config: &mut SourceConfig) {
        config.namespaces.insert("fixture".into(), Namespace {
            project: "/workspace/fixture".into(),
            source_root: None,
            git: Some(serde_json::from_value(serde_json::json!({
                "locked": vec![3; 32], "patches": vec![4; 32],
            })).unwrap()),
        });
    }

    #[test]
    fn exact_current_envelope_matches_its_parser_and_namespace_family() {
        let mut config = config(wosy_syntax::base_cst_schema());
        let ordinary_root = top(source_preparation_schema());
        let git_root = top(installed_git_source_schema());
        let ordinary = source_preparation_parts(&ordinary_root).unwrap();
        let git = source_preparation_parts(&git_root).unwrap();
        assert_eq!(ordinary.envelope, SourcePreparationEnvelope::NamespaceAuthority);
        assert_eq!(git.envelope, SourcePreparationEnvelope::InstalledGit);
        assert_eq!(ordinary.files, &[NodeId([5; 32])]);
        assert_eq!(config.schema(), source_preparation_schema());
        assert!(ordinary.envelope.matches_config(&config));
        assert!(!git.envelope.matches_config(&config));

        add_git_namespace(&mut config);
        assert_eq!(config.schema(), installed_git_source_schema());
        assert!(git.envelope.matches_config(&config));
        assert!(!ordinary.envelope.matches_config(&config));
        config.parser.schema = NodeId([0; 32]);
        assert!(!git.envelope.matches_config(&config));
        assert!(source_preparation_parts(&top(&[0; 32])).is_none());
    }

    #[test]
    fn frozen_local_metadata_is_distinct_from_the_actual_current_epoch() {
        let current = SourceEnvelopeGrammar::current();
        let frozen = SourceEnvelopeGrammar::FROZEN_BASE_V3;
        assert_ne!(current.parser, frozen.parser);
        assert_ne!(current.namespace, frozen.namespace);
        assert_ne!(current.git, frozen.git);
        let frozen_local = top(frozen.namespace);
        let frozen_git = top(frozen.git);
        let current_local = top(current.namespace);
        let current_git = top(current.git);
        for (is_git, retained, active) in [(false, &frozen_local, &current_local), (true, &frozen_git, &current_git)] {
            let mut old_config = config(frozen.parser);
            let mut active_config = config(current.parser);
            if is_git {
                add_git_namespace(&mut old_config);
                add_git_namespace(&mut active_config);
            }
            assert!(source_preparation_parts(retained).is_none());
            let archive = retained_source_preparation_metadata(retained).unwrap();
            assert_eq!(archive.kind_encoding(), CstKindEncoding::FrozenBaseV3);
            assert!(archive.matches_config(&old_config));
            assert!(!archive.matches_config(&active_config));
            assert!(archive.kind_encoding().allows_raw(84));
            assert!(!archive.kind_encoding().allows_raw(85));
            let current = source_preparation_parts(active).unwrap();
            let current_pin = retained_source_preparation_metadata(active).unwrap();
            assert_eq!(current_pin.kind_encoding(), CstKindEncoding::Current);
            assert!(current_pin.kind_encoding().allows(wosy_syntax::SyntaxKind::ComptimeExpr));
            assert!(current.matches_config(&active_config) && current_pin.matches_config(&active_config));
            assert!(!current.matches_config(&old_config) && !current_pin.matches_config(&old_config));
            if !is_git { add_git_namespace(&mut old_config); }
            else { old_config.namespaces.remove("fixture"); }
            assert!(!archive.matches_config(&old_config));
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
