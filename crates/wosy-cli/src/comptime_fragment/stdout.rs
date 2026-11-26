//! Select an ordinary compiler result shape and parse already collected stdout.
//! Neither entry point launches a program or accepts a result into a build.
use super::{
    selected_builder_invocation, selected_runner_invocation, write_comptime_partition_ir, ClosedComptimeInvocation, ClosedComptimeModuleObservation,
    ClosedComptimePaths, ClosedComptimePlanError, StagedEvaluatorIr,
};
use crate::project_sources::{self, Error as SourceError};
use std::{collections::BTreeMap, fmt, io::{self, Write}, path::{Path, PathBuf}, process::ExitStatus};
use wosy_compiler::{
    parse_source_with_snapshot, prepare_comptime_stdout, prepare_comptime_stdout_evaluator,
    prepare_comptime_wasip1_stdout_evaluator,
    ClosedComptimeRejection, ComptimeStdoutError, PreparedComptimeStdout, PreparedComptimeStdoutEvaluator,
    ScalarExpression, ScalarNamespaceBinding, ScalarTargetLayout, ScalarType, CLOSED_COMPTIME_MAX_SOURCE_BYTES,
};
use wosy_project::{ObservedClosedComptimeProfile, ProjectConfiguration, ProjectContext, SelectedComptimeTools};
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
    let (output, _, _) = observe_project(root, target, profile, None, |sources, order| {
        prepare_comptime_stdout(sources, order, id, layout)
    })?;
    Ok(ProjectComptimeStdout { output })
}

/// The compiler-emitted aggregate evaluator and the exact result shape.
/// The builder/runner descriptions are inert. Only the explicitly selected
/// preview1 variant includes a stdout entry; detached IR does not.
pub struct ProjectComptimeStdoutEvaluator {
    evaluator: PreparedComptimeStdoutEvaluator,
    project: PathBuf,
    tools: SelectedComptimeTools,
}

pub fn prepare_project_comptime_stdout_evaluator(
    root: &Path, target: &str, profile: &str, id: &str, layout: ScalarTargetLayout,
) -> Result<ProjectComptimeStdoutEvaluator, ClosedComptimePlanError> {
    let (evaluator, project, tools) = observe_project(root, target, profile, None, |sources, order| {
        prepare_comptime_stdout_evaluator(sources, order, id, layout)
    })?;
    Ok(ProjectComptimeStdoutEvaluator { evaluator, project, tools })
}

/// Prepare an explicitly requested WASI preview1 stdout stage. The
/// selected ordinary artifact profile must declare llvm/wasm-wasip1;
/// this does not infer evaluator tool identity or support wasip2. The
/// caller's authored byte budget covers the selected root and imports.
pub fn prepare_project_comptime_wasip1_stdout_evaluator(
    root: &Path, target: &str, profile: &str, id: &str,
    max_authored_source_bytes: usize, max_stdout: usize,
) -> Result<ProjectComptimeStdoutEvaluator, ClosedComptimePlanError> {
    let (evaluator, project, tools) = observe_project(root, target, profile, Some(max_authored_source_bytes), |sources, order| {
        prepare_comptime_wasip1_stdout_evaluator(sources, order, id, max_authored_source_bytes, max_stdout)
    })?;
    Ok(ProjectComptimeStdoutEvaluator { evaluator, project, tools })
}

impl ProjectComptimeStdoutEvaluator {
    pub fn site(&self) -> &SourceSpan { self.evaluator.output().site() }
    pub fn result_type(&self) -> &ScalarType { self.evaluator.output().result_type() }
    pub fn layout(&self) -> ScalarTargetLayout { self.evaluator.layout() }

    pub fn write_evaluator_ir(&self, out: &mut impl Write, max_bytes: usize) -> io::Result<StagedEvaluatorIr> {
        write_comptime_partition_ir(self.evaluator.llvm(), out, max_bytes)
    }

    pub fn builder<'a>(&'a self, paths: ClosedComptimePaths<'a>) -> ClosedComptimeInvocation<'a> {
        selected_builder_invocation(&self.project, &self.tools, paths)
    }

    pub fn runner<'a>(&'a self, manifest: &'a Path) -> ClosedComptimeInvocation<'a> {
        selected_runner_invocation(&self.project, &self.tools, manifest)
    }

    pub fn read_stdout(&self, status: ExitStatus, stdout: &[u8], max_bytes: usize) -> Result<ScalarExpression, ComptimeReadbackError> {
        after_success(status, || self.evaluator.output().parse_stdout(stdout, max_bytes))
    }
}

fn observe_project<T>(
    root: &Path, target: &str, profile: &str, wasip1_source_limit: Option<usize>,
    prepare: impl FnOnce(
        Vec<(&FreshParseResult, Vec<ScalarNamespaceBinding>)>, Vec<SourceIdentity>,
    ) -> Result<T, ClosedComptimeRejection>,
) -> Result<(T, PathBuf, SelectedComptimeTools), ClosedComptimePlanError> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let (configuration, manifest) = ProjectConfiguration::load_with_manifest(&root)?;
    let (_, _, artifact, _) = configuration.artifact_profile(target, profile)?;
    if wasip1_source_limit.is_some() && (artifact.backend != "llvm" || artifact.output != "wasm-wasip1") {
        return Err(ClosedComptimePlanError::Project("computed stdout stage requires an explicit llvm/wasm-wasip1 profile".into()));
    }
    let selection = ObservedClosedComptimeProfile::observe_input_declarations(&configuration, &manifest, target, profile)?;
    let context = ProjectContext::from_manifest(&root, target, manifest.as_table())?;
    let mut remaining = wasip1_source_limit.unwrap_or(CLOSED_COMPTIME_MAX_SOURCE_BYTES);
    let mut is_entry = true;
    let mut authored = BTreeMap::new();
    let graph = project_sources::load_with_reader(&root, &context, &configuration, |path, opened| {
        // The selected closed evaluator retains its per-entry ceiling.
        // Every ordinary reachable source consumes the same graph budget.
        let limit = if std::mem::take(&mut is_entry) {
            remaining.min(CLOSED_COMPTIME_MAX_SOURCE_BYTES)
        } else { remaining };
        let (revision, text) = project_sources::read_bounded_regular_source(path, opened, limit)
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
    let output = prepare(canonical, order).map_err(ClosedComptimePlanError::Fragment)?;
    Ok((output, root, selection.into_tools()))
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

    /// An opt-in producer for actual checked literal bodies only. This
    /// does not invoke or replace a configured runner for computed bodies.
    pub fn write_literal_stdout(&self, out: &mut impl Write, max_bytes: usize) -> io::Result<usize> {
        self.output.write_literal_stdout(out, max_bytes)
    }

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
