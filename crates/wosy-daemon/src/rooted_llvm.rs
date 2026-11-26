//! Opt-in LLVM projections from one grammar-verified leased source root.
use super::{
    build_session::BuildRequestRejection, file_source, project_declarations::RootedOrder,
    grammar_receipts::VerifiedGrammarBatch,
    read_index_using, read_verified_node_with, retention_plan::SourceLease,
    source_proposal::{authored_grammar_matches, take},
    validate_file_using, BaseCstAdmissionLimits, FileTree, Result, SourceStore,
};
use std::{collections::HashMap, io::{self, Write}, time::{Duration, Instant}};
use surrealkv::Mode;
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, prepare_scalar_project, Diagnostic,
    ScalarArtifactEmission, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_protocol::{source_preparation_parts, Kind, NodeId, SourceConfig};
use wosy_syntax::{AuthoredSpanMap, ByteSpan, CanonicalCstRoot, SourceIdentity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootedLlvmIdentity {
    /// Exact symbol identity of the existing ordinary Build.
    Authored,
    /// Use the verified executable CST's identity and omit typed-comment tokens.
    /// This changes internal legacy symbols; authored diagnostics are preserved.
    Executable,
}

impl RootedLlvmIdentity {
    fn tree(self) -> FileTree {
        match self {
            Self::Authored => FileTree::Authored,
            Self::Executable => FileTree::Both,
        }
    }

    fn owner(self, authored: SourceIdentity, executable: NodeId) -> (SourceIdentity, Option<SourceIdentity>) {
        match self {
            Self::Authored => (authored, None),
            Self::Executable => (
                SourceIdentity { revision: executable.hex(), ..authored.clone() }, Some(authored),
            ),
        }
    }
}

#[derive(Debug)]
pub enum RootedLlvmProjection {
    Unsupported(RootedLlvmUnsupported),
    Rejected(RootedLlvmRejection),
    Emitted(RootedLlvmPartition),
}

#[derive(Debug, Eq, PartialEq)]
pub enum RootedLlvmUnsupported {
    ProfileNotSelected,
    ProfileNotSupported(String),
    LegacySourcePath,
    CompilerImageUnavailable,
    AuthoredGrammarNotVerified,
}

#[derive(Debug)]
pub enum RootedLlvmRejection {
    Declarations(Vec<Diagnostic>),
    Preparation(Vec<Diagnostic>),
    Analysis(Vec<Diagnostic>),
    Emitter(String),
}

/// Exact partition bytes in the selected codegen identity, bound to the
/// leased source and actual compiler image. No artifact manifest, reuse
/// decision or semantic SnapshotRoot is implied.
#[derive(Debug)]
pub struct RootedLlvmPartition {
    pub source_root: NodeId,
    pub profile: String,
    pub compiler_image: NodeId,
    pub identity: RootedLlvmIdentity,
    pub bytes: Vec<u8>,
    pub digest: NodeId,
    /// All files with verified grammar, counting a reused exact receipt.
    pub grammar_verified_files: usize,
    pub grammar_reparsed_files: usize,
    pub grammar_verification: Duration,
}

impl SourceStore {
    pub(super) fn project_ll_for_lease(
        &self, lease: &SourceLease, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize,
        identity: RootedLlvmIdentity,
    ) -> Result<RootedLlvmProjection> {
        let grant = lease.grant();
        let (tx, _read) = self.begin_leased_source(grant, Mode::ReadOnly)?
            .ok_or(BuildRequestRejection::Expired)?;
        let mut grammar = VerifiedGrammarBatch::default();
        let result = self.project_ll(&tx, grant.root, profile, limits, output_bytes, identity, &mut grammar)?;
        if lease.deadline().is_none_or(|deadline| Instant::now() >= deadline) {
            return Err(BuildRequestRejection::Expired.into());
        }
        if matches!(result, RootedLlvmProjection::Emitted(_)) {
            self.grammar_receipts.accept(grammar);
        }
        Ok(result)
    }

    fn project_ll(
        &self, tx: &surrealkv::Transaction, root: NodeId, profile: String,
        limits: BaseCstAdmissionLimits, output_bytes: usize, identity: RootedLlvmIdentity,
        grammar: &mut VerifiedGrammarBatch,
    ) -> Result<RootedLlvmProjection> {
        use RootedLlvmProjection as Projection;
        use RootedLlvmRejection as Rejection;
        use RootedLlvmUnsupported as Unsupported;

        let (mut records, mut encoded, mut imports) = (limits.records, limits.record_bytes, limits.graph_edges);
        let mut read_index_node = |id| read_verified_node_with(tx, id, |bytes| {
            take(&mut records, 1, "LLVM source index exceeds the record budget")?;
            take(&mut encoded, bytes, "LLVM source index exceeds the byte budget")
        });
        let top = read_index_node(root)?;
        let parts = source_preparation_parts(&top).ok_or("not a supported rooted base source")?;
        let config = read_index_node(parts.config)?;
        if config.kind != Kind::Config { return Err("invalid LLVM source configuration kind".into()); }
        let config: SourceConfig = serde_json::from_slice(&config.data)?;
        if top.data != config.schema() || config.parser.schema.0 != *wosy_syntax::base_cst_schema() {
            return Err("invalid LLVM source configuration schema".into());
        }
        let Some(artifact) = config.profiles.get(&profile) else {
            return Ok(Projection::Unsupported(Unsupported::ProfileNotSelected));
        };
        let layout = match artifact.target_layout() {
            Ok(layout) => layout,
            Err(reason) => return Ok(Projection::Unsupported(Unsupported::ProfileNotSupported(reason))),
        };
        let Some(image) = self.derivation_image() else {
            return Ok(Projection::Unsupported(Unsupported::CompilerImageUnavailable));
        };
        let index = read_index_using(top, read_index_node, |count| {
            take(&mut imports, count, "LLVM source index exceeds the import budget")
        })?;
        // The admitted index fixes the same first-discovery order and
        // ordered destinations as the ordinary project source loader.
        let order = RootedOrder::from_parts(&config.entry, index.modules, index.files, index.module_edges);
        let parser = config.parser.id();
        let mut owners = Vec::with_capacity(order.modules.len());
        let mut original_owners = Vec::new();
        if identity == RootedLlvmIdentity::Executable { original_owners.reserve(order.modules.len()); }
        for module in &order.modules {
            if module.file.metadata.policy != parser {
                return Err("LLVM file does not match the selected parser policy".into());
            }
            if module.file.metadata.source.path.as_path().to_str().is_none_or(|path| path.contains('\\')) {
                // Legacy ordinary Build rewrites the display path. This is
                // not the same identity as the rooted native path.
                return Ok(Projection::Unsupported(Unsupported::LegacySourcePath));
            }
            let (owner, authored) = identity.owner(file_source(&module.file)?, module.file.executable);
            owners.push(owner);
            original_owners.extend(authored);
        }
        let root_owner = owners.first().ok_or("leased source has no entry owner")?.clone();
        let module_count = owners.len();
        let mut work = limits.cst_work();
        let mut input = Vec::with_capacity(order.modules.len());
        let mut extracted = Vec::new();
        let mut authored_sources = HashMap::new();
        let mut grammar_verification = Duration::ZERO;
        let mut grammar_reparsed_files = 0;
        let mut original_owners = original_owners.into_iter();
        for (module, owner) in order.modules.into_iter().zip(&owners) {
            let checked = validate_file_using(&module.file, &config.parser.categories, identity.tree(),
                Some(&mut work), |id, work| {
                    read_verified_node_with(tx, id, |bytes| work.expect("bounded authored expansion").read(bytes))
                })?;
            // Hash, authored closure and budgets must pass even on receipt
            // hits. A generic published marker carries no grammar witness.
            if !self.grammar_receipts.contains(&module.file) {
                let verification = Instant::now();
                if !authored_grammar_matches(&module.file, &config.parser.categories, &checked)? {
                    return Ok(Projection::Unsupported(Unsupported::AuthoredGrammarNotVerified));
                }
                grammar_verification += verification.elapsed();
                grammar_reparsed_files += 1;
                grammar.verified(&module.file);
            }
            let selected = if let Some(original) = original_owners.next() {
                authored_sources.insert(owner.clone(), (original, AuthoredSpanMap::from_comments(&checked.comments)));
                checked.executable.ok_or("leased source has no executable CST")?
            } else {
                checked.authored.ok_or("leased source has no authored CST")?
            };
            let tree = CanonicalCstRoot::new(owner.clone(), selected);
            let declarations = derive_scalar_declarations_from_cst(&tree);
            extracted.extend_from_slice(declarations.diagnostics());
            let bindings = match identity {
                RootedLlvmIdentity::Authored => module.file.metadata.imports.into_iter().zip(module.imports)
                    .map(|(site, target)| ScalarNamespaceBinding {
                        binding: site.binding, target: owners[target.0].clone(),
                        span: ByteSpan::new(site.span.start, site.span.end),
                    }).collect(),
                RootedLlvmIdentity::Executable if declarations.diagnostics().is_empty() => {
                    // Publication already paired canonical namespace order
                    // to resolved destinations. Take coordinates from the
                    // actual executable compiler declarations, not the
                    // separately authored offset hints.
                    let mut resolved = module.imports.into_iter();
                    let mut bindings = Vec::with_capacity(resolved.len());
                    for site in declarations.namespaces() {
                        let target = resolved.next().ok_or("executable namespace has no resolved destination")?;
                        bindings.push(ScalarNamespaceBinding {
                            binding: site.binding.clone(), target: owners[target.0].clone(), span: site.span,
                        });
                    }
                    if resolved.next().is_some() { return Err("resolved destination has no executable namespace".into()); }
                    bindings
                }
                RootedLlvmIdentity::Executable => Vec::new(),
            };
            input.push((declarations, bindings));
        }
        let authored = |diagnostics| project_authored(identity, &authored_sources, diagnostics);
        if !extracted.is_empty() { return Ok(Projection::Rejected(Rejection::Declarations(authored(extracted)?))); }
        let prepared = match prepare_scalar_project(input, owners, ScalarPreparationInputs {
            target_layout: ScalarTargetLayout {
                pointer_size: layout.pointer_size, pointer_alignment: layout.pointer_alignment,
            },
        }) {
            Ok(prepared) => prepared,
            Err(rejected) => return Ok(Projection::Rejected(Rejection::Preparation(authored(rejected.into_ordered_diagnostics())?))),
        };
        // No diagnostic cache can authorize this route: the real emitter
        // consumes the fresh ordinary validation and its ownership facts.
        let validation = analyze_scalar_build(&prepared);
        if !validation.diagnostics().is_empty() {
            return Ok(Projection::Rejected(Rejection::Analysis(authored(validation.diagnostics().to_vec())?)));
        }
        let emission = ScalarArtifactEmission::select(
            module_count, artifact.exports.as_ref().map(|exports| (&root_owner, exports)),
        );
        let partition = match emission.emit(&validation) {
            Ok(partition) => partition,
            Err(rejected) => return Ok(Projection::Rejected(Rejection::Emitter(rejected))),
        };
        let mut output = PartitionBytes { bytes: Vec::new(), limit: output_bytes };
        partition.write_text(&mut output)?;
        let digest = NodeId(*blake3::hash(&output.bytes).as_bytes());
        Ok(Projection::Emitted(RootedLlvmPartition {
            source_root: root, profile, compiler_image: image, identity, bytes: output.bytes, digest,
            grammar_verified_files: module_count, grammar_reparsed_files, grammar_verification,
        }))
    }
}

fn project_authored(
    identity: RootedLlvmIdentity,
    sources: &HashMap<SourceIdentity, (SourceIdentity, AuthoredSpanMap)>,
    mut diagnostics: Vec<Diagnostic>,
) -> Result<Vec<Diagnostic>> {
    if identity == RootedLlvmIdentity::Executable {
        for label in diagnostics.iter_mut().flat_map(|diagnostic| &mut diagnostic.labels) {
            let (source, spans) = sources.get(&label.span.source)
                .ok_or("LLVM diagnostic lies outside the leased authored source")?;
            label.span.source = source.clone();
            label.span.range = spans.project(label.span.range);
        }
    }
    Ok(diagnostics)
}

// Limit retained egress, not temporary LLVM allocations during emission.
struct PartitionBytes {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for PartitionBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other("LLVM partition exceeds the output byte budget"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
