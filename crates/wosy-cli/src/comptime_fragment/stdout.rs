//! Select an ordinary compiler result shape and parse already collected stdout.
//! Neither entry point launches a program or accepts a result into a build.
use super::{ClosedComptimeModuleObservation, ClosedComptimePlanError};
use crate::project_sources::{self, Error as SourceError};
use std::{collections::BTreeMap, fmt, path::Path, process::ExitStatus};
use wosy_compiler::{
    parse_source_with_snapshot, prepare_comptime_stdout, ComptimeStdoutError, PreparedComptimeStdout,
    ScalarExpression, ScalarNamespaceBinding, ScalarTargetLayout, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use wosy_project::{ObservedClosedComptimeProfile, ProjectConfiguration, ProjectContext};
use wosy_syntax::{FreshParseResult, SourceIdentity, SourceSpan};

pub struct ProjectComptimeStdout {
    output: PreparedComptimeStdout,
}

/// Inspect a root comptime's actual resolved result type in the ordinary
/// selected target. Source, namespaces, and declaration types come from
/// the existing loaders; no result type or wire schema is caller supplied.
pub fn prepare_project_comptime_stdout(
    root: &Path, target: &str, profile: &str, id: &str, layout: ScalarTargetLayout,
) -> Result<ProjectComptimeStdout, ClosedComptimePlanError> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let (configuration, manifest) = ProjectConfiguration::load_with_manifest(&root)?;
    configuration.artifact_profile(target, profile)?;
    ObservedClosedComptimeProfile::observe_input_declarations(&configuration, &manifest, target, profile)?;
    let context = ProjectContext::from_manifest(&root, target, manifest.as_table())?;
    let mut remaining = CLOSED_COMPTIME_MAX_SOURCE_BYTES;
    let mut authored = BTreeMap::new();
    let graph = project_sources::load_with_reader(&root, &context, &configuration, |path, opened| {
        let (revision, text) = project_sources::read_bounded_regular_source(path, opened, remaining)
            .map_err(ClosedComptimePlanError::Project)?;
        remaining -= text.len();
        Ok((revision, text))
    }, |source, revision, text| -> Result<_, ClosedComptimePlanError> {
        let path = source.path.as_path().to_str().ok_or("source path is not UTF-8").map_err(str::to_owned)?;
        let identity = SourceIdentity::new(source.project.clone(), source.package.clone(), path.into(), revision.0.clone());
        let (fresh, diagnostics) = parse_source_with_snapshot(identity, text, &context.comment_categories);
        if !diagnostics.is_empty() { return Err(ClosedComptimePlanError::Parse(diagnostics)); }
        let imports = wosy_protocol::imports_from_cst(&fresh.result().canonical_cst().root)?;
        authored.insert((source.package.clone(), source.path.clone()), fresh);
        Ok(imports)
    }).map_err(|error| match error {
        SourceError::Observe(error) => error,
        SourceError::Graph(error) => ClosedComptimePlanError::Project(format!("{error:?}")),
    })?;

    let sources: Vec<FreshParseResult> = graph.nodes.iter().map(|node| {
        authored.remove(&(node.source.package.clone(), node.source.path.clone()))
            .ok_or_else(|| ClosedComptimePlanError::Project("missing ordinary canonical source".into()))
    }).collect::<Result<_, _>>()?;
    let canonical = graph.nodes.iter().zip(&sources).map(|(node, fresh)| {
        let bindings = node.namespace_edges.iter().map(|edge| ScalarNamespaceBinding {
            binding: edge.binding.clone(),
            target: sources[edge.target.0].result().source.clone(),
            span: wosy_syntax::ByteSpan::new(edge.span.range.start, edge.span.range.end),
        }).collect();
        (fresh, bindings)
    }).collect();
    let order = graph.initialization_order.iter().map(|id| sources[id.0].result().source.clone()).collect();
    let output = prepare_comptime_stdout(canonical, order, id, layout).map_err(ClosedComptimePlanError::Fragment)?;
    Ok(ProjectComptimeStdout { output })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComptimeReadbackError {
    Exit(Option<i32>),
    Output(ComptimeStdoutError),
}

impl fmt::Display for ComptimeReadbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exit(Some(code)) => write!(f, "comptime runner exited with status {code}"),
            Self::Exit(None) => f.write_str("comptime runner did not exit normally"),
            Self::Output(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ComptimeReadbackError {}

impl ProjectComptimeStdout {
    pub fn site(&self) -> &SourceSpan { self.output.site() }

    /// Parse one value after exit 0 against the type already prepared from
    /// this authored site. The bytes and status themselves confer no runner
    /// provenance or permission to apply the value to a project.
    pub fn read_stdout(&self, status: ExitStatus, stdout: &[u8], max_bytes: usize) -> Result<ScalarExpression, ComptimeReadbackError> {
        after_success(status, || self.output.parse_stdout(stdout, max_bytes))
    }
}

impl ClosedComptimeModuleObservation<'_> {
    /// The already-provisioned scalar path uses the same parser and compiler
    /// result type without an extra selection or validation phase.
    pub fn read_stdout(&self, status: ExitStatus, stdout: &[u8], max_bytes: usize) -> Result<ScalarExpression, ComptimeReadbackError> {
        after_success(status, || self.provision().staging().fragment().parse_stdout(stdout, max_bytes))
    }
}

fn after_success(
    status: ExitStatus, parse: impl FnOnce() -> Result<ScalarExpression, ComptimeStdoutError>,
) -> Result<ScalarExpression, ComptimeReadbackError> {
    if !status.success() { return Err(ComptimeReadbackError::Exit(status.code())); }
    parse().map_err(ComptimeReadbackError::Output)
}
