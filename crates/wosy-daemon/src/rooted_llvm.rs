//! Opt-in projection of ordinary LLVM from a leased authored source root.
use super::{
    build_session::BuildRequestRejection, file_source, project_declarations::RootedOrder,
    read_index_using, read_verified_node_with, retention_plan::SourceLease,
    source_proposal::{authored_grammar_matches, take},
    validate_file_using, BaseCstAdmissionLimits, FileTree, Result, SourceStore,
};
use std::{io::{self, Write}, time::{Duration, Instant}};
use surrealkv::Mode;
use wosy_compiler::{
    analyze_scalar_build, derive_scalar_declarations_from_cst, prepare_scalar_project, Diagnostic,
    ScalarArtifactEmission, ScalarNamespaceBinding, ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_protocol::{source_preparation_parts, Kind, NodeId, SourceConfig};
use wosy_syntax::{ByteSpan, CanonicalCstRoot};

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

/// Exact ordinary partition bytes, bound to the selected source and actual
/// compiler image. No artifact manifest, reuse decision or semantic
/// SnapshotRoot is implied.
#[derive(Debug)]
pub struct RootedLlvmPartition {
    pub source_root: NodeId,
    pub profile: String,
    pub compiler_image: NodeId,
    pub bytes: Vec<u8>,
    pub digest: NodeId,
    pub grammar_verified_files: usize,
    pub grammar_verification: Duration,
}

impl SourceStore {
    pub(super) fn project_ll_for_lease(
        &self, lease: &SourceLease, profile: String, limits: BaseCstAdmissionLimits, output_bytes: usize,
    ) -> Result<RootedLlvmProjection> {
        let grant = lease.grant();
        let (tx, _read) = self.begin_leased_source(grant, Mode::ReadOnly)?
            .ok_or(BuildRequestRejection::Expired)?;
        let result = self.project_ll(&tx, grant.root, profile, limits, output_bytes)?;
        if lease.deadline().is_none_or(|deadline| Instant::now() >= deadline) {
            return Err(BuildRequestRejection::Expired.into());
        }
        Ok(result)
    }

    fn project_ll(
        &self, tx: &surrealkv::Transaction, root: NodeId, profile: String,
        limits: BaseCstAdmissionLimits, output_bytes: usize,
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
        let mut owners = Vec::with_capacity(order.modules.len());
        for module in &order.modules {
            if module.file.metadata.source.path.as_path().to_str().is_none_or(|path| path.contains('\\')) {
                // Legacy ordinary Build rewrites the display path. This is
                // not the same identity as the rooted native path.
                return Ok(Projection::Unsupported(Unsupported::LegacySourcePath));
            }
            owners.push(file_source(&module.file)?);
        }
        let root_owner = owners.first().ok_or("leased source has no entry owner")?.clone();
        let module_count = owners.len();
        let mut work = limits.cst_work();
        let mut input = Vec::with_capacity(order.modules.len());
        let mut extracted = Vec::new();
        let mut grammar_verification = Duration::ZERO;
        for (module, owner) in order.modules.into_iter().zip(&owners) {
            let checked = validate_file_using(&module.file, &config.parser.categories, FileTree::Authored,
                Some(&mut work), |id, work| {
                    read_verified_node_with(tx, id, |bytes| work.expect("bounded authored expansion").read(bytes))
                })?;
            // A published legacy root attests source structure, not the
            // ordinary grammar. Use the same oracle as sparse admission.
            let verification = Instant::now();
            if !authored_grammar_matches(&module.file, &config.parser.categories, &checked)? {
                return Ok(Projection::Unsupported(Unsupported::AuthoredGrammarNotVerified));
            }
            grammar_verification += verification.elapsed();
            let authored = checked.authored.ok_or("leased source has no authored CST")?;
            let tree = CanonicalCstRoot::new(owner.clone(), authored);
            let declarations = derive_scalar_declarations_from_cst(&tree);
            extracted.extend_from_slice(declarations.diagnostics());
            let bindings = module.file.metadata.imports.into_iter().zip(module.imports).map(|(site, target)| {
                ScalarNamespaceBinding {
                    binding: site.binding,
                    target: owners[target.0].clone(),
                    span: ByteSpan::new(site.span.start, site.span.end),
                }
            }).collect();
            input.push((declarations, bindings));
        }
        if !extracted.is_empty() { return Ok(Projection::Rejected(Rejection::Declarations(extracted))); }
        let prepared = match prepare_scalar_project(input, owners, ScalarPreparationInputs {
            target_layout: ScalarTargetLayout {
                pointer_size: layout.pointer_size, pointer_alignment: layout.pointer_alignment,
            },
        }) {
            Ok(prepared) => prepared,
            Err(rejected) => return Ok(Projection::Rejected(Rejection::Preparation(rejected.into_ordered_diagnostics()))),
        };
        // No diagnostic cache can authorize this route: the real emitter
        // consumes the fresh ordinary validation and its ownership facts.
        let validation = analyze_scalar_build(&prepared);
        if !validation.diagnostics().is_empty() {
            return Ok(Projection::Rejected(Rejection::Analysis(validation.diagnostics().to_vec())));
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
            source_root: root, profile, compiler_image: image, bytes: output.bytes, digest,
            grammar_verified_files: module_count, grammar_verification,
        }))
    }
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
