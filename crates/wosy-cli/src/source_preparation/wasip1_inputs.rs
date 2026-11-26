//! One ordinary source and watch basis for an opted-in WASIp1 continuation.
use super::{prepare_from_observation, verify_live_source, ComptimeAttachment, Files, LiveSourceReader,
    ObservationError, PreparedComptimeWatches, PreparedSources};
use crate::comptime_fragment::write_comptime_partition_ir;
use std::{collections::BTreeMap, io, path::{Path, PathBuf}};
use tokio::time::Instant;
use wosy_compiler::{prepare_wasip1_comptime_continuation, PreparedWasip1ComptimeContinuation, ScalarNamespaceBinding};
use wosy_project::{packages::{observe_comptime_watches, ComptimeWatchLimits}, ParsedProjectManifest, ProjectConfiguration,
    SelectedComptimeTools, SourceGraph};
use wosy_protocol::{FileKey, NodeId};
use wosy_syntax::{ByteSpan, FreshParseResult};

#[derive(Clone, Copy)]
pub struct Wasip1ComptimeInputSelection<'a> {
    pub target: &'a str,
    pub profile: &'a str,
    pub id: &'a str,
    pub max_authored_source_bytes: usize,
    pub max_stdout_bytes: usize,
    pub watch_limits: ComptimeWatchLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeInputsFreshness {
    SourceOrConfigurationChanged,
    WatchesChanged,
    Unchanged,
}

/// Owner of the actual once-parsed authored namespace graph and selected
/// evaluator. Neither stdout supplied by a caller nor matching watch bytes
/// authorizes a result, an execution, persistent data, or a cache hit.
pub struct PreparedWasip1ComptimeInputs {
    basis: Wasip1InputsBasis,
    files: Files,
}

pub(super) struct Wasip1InputsBasis {
    project: PathBuf,
    configuration: ProjectConfiguration,
    manifest: ParsedProjectManifest,
    package: String,
    root: NodeId,
    authored_cst: NodeId,
    watches: PreparedComptimeWatches,
    tools: SelectedComptimeTools,
    continuation: PreparedWasip1ComptimeContinuation,
    max_authored_source_bytes: usize,
    watch_limits: ComptimeWatchLimits,
}

pub fn prepare_wasip1_comptime_inputs(
    root: &Path, selection: Wasip1ComptimeInputSelection<'_>,
) -> Result<(PreparedSources, PreparedWasip1ComptimeInputs), String> {
    let (prepared, files, _, attachment) =
        prepare_from_observation(root, Some(selection.target), None, None, None, None, Some(selection))
            .map_err(|error| match error {
                ObservationError::Source(message) => message,
                ObservationError::LiveRead(_) | ObservationError::Changed =>
                    unreachable!("a WASI continuation starts with fresh authored source"),
            })?;
    let Some(ComptimeAttachment::Wasip1(basis)) = attachment else { unreachable!("selected WASI basis") };
    Ok((prepared, PreparedWasip1ComptimeInputs { basis, files }))
}

impl Wasip1InputsBasis {
    pub(super) fn assemble(
        project: PathBuf, configuration: ProjectConfiguration, manifest: ParsedProjectManifest,
        package: String, root: NodeId, authored_cst: NodeId, graph: SourceGraph,
        mut fresh: BTreeMap<FileKey, FreshParseResult>, tools: SelectedComptimeTools,
        selection: Wasip1ComptimeInputSelection<'_>,
    ) -> Result<Self, String> {
        let mut ordered = Vec::with_capacity(graph.nodes.len());
        for node in &graph.nodes {
            ordered.push(fresh.remove(&FileKey::from(&node.source)).expect("ordinary source graph parse"));
        }
        let order = graph.initialization_order.iter().map(|id| ordered[id.0].result().source.clone()).collect();
        let namespaces: Vec<_> = graph.nodes.iter().map(|node| node.namespace_edges.iter().map(|edge| {
            ScalarNamespaceBinding { binding: edge.binding.clone(), target: ordered[edge.target.0].result().source.clone(),
                span: ByteSpan::new(edge.span.range.start, edge.span.range.end) }
        }).collect()).collect();
        let continuation = prepare_wasip1_comptime_continuation(ordered.into_iter().zip(namespaces).collect(), order,
            selection.id, selection.max_authored_source_bytes, selection.max_stdout_bytes)
            .map_err(|error| format!("WASI comptime continuation: {error:?}"))?;
        let files = observe_comptime_watches(&project, &package, &configuration, &manifest,
            &package, selection.id, selection.watch_limits)?;
        let watches = PreparedComptimeWatches { target: tools.target().to_owned(),
            profile: tools.profile().to_owned(), id: selection.id.to_owned(),
            site: continuation.evaluator().output().site().clone(), files };
        Ok(Self { project, configuration, manifest, package, root, authored_cst, watches, tools, continuation,
            max_authored_source_bytes: selection.max_authored_source_bytes, watch_limits: selection.watch_limits })
    }
}

impl PreparedWasip1ComptimeInputs {
    pub fn source_root(&self) -> NodeId { self.basis.root }
    pub fn authored_cst(&self) -> NodeId { self.basis.authored_cst }
    pub fn watches(&self) -> &PreparedComptimeWatches { &self.basis.watches }
    pub fn declared_tools(&self) -> &SelectedComptimeTools { &self.basis.tools }
    pub fn continuation(&self) -> &PreparedWasip1ComptimeContinuation { &self.basis.continuation }
    pub fn write_evaluator_ir(&self, out: &mut impl io::Write, cap: usize) -> io::Result<crate::comptime_fragment::StagedEvaluatorIr> {
        write_comptime_partition_ir(self.basis.continuation.evaluator().llvm(), out, cap)
    }

    /// Verify the same ordinary source/configuration without parsing it
    /// again, then compare the complete sorted watch observation. This is
    /// a live read cut, not an atomic filesystem snapshot or execution proof.
    pub fn observe_current(&self, deadline: Instant) -> Result<ComptimeInputsFreshness, Box<dyn std::error::Error + Send + Sync>> {
        let basis = &self.basis;
        let reader = LiveSourceReader::new(basis.max_authored_source_bytes, deadline);
        if verify_live_source(&basis.project, &basis.watches.target, &self.files, basis.root, reader, false)?.is_none() {
            return Ok(ComptimeInputsFreshness::SourceOrConfigurationChanged);
        }
        let now = observe_comptime_watches(&basis.project, &basis.package, &basis.configuration, &basis.manifest,
            &basis.package, &basis.watches.id, basis.watch_limits)?;
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "comptime live observation deadline elapsed").into());
        }
        Ok(if now == basis.watches.files { ComptimeInputsFreshness::Unchanged }
            else { ComptimeInputsFreshness::WatchesChanged })
    }
}
