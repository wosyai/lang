//! Opt-in producer of a closed fragment, not an evaluator runner.
use crate::project_sources::{self, Error as SourceError};
use std::path::Path;
use wosy_compiler::{
    derive_closed_comptime_fragment, parse_source_with_snapshot, ClosedComptimeFragment,
    ClosedComptimeRejection, ComptimeParameter, Diagnostic, ScalarTargetLayout,
};
use wosy_project::{declared_comptime_parameters, DeclaredComptimeParameter, ProjectConfiguration, ProjectContext};
use wosy_syntax::SourceIdentity;

#[derive(Debug)]
pub enum ClosedComptimePlanError {
    Project(String),
    Parse(Vec<Diagnostic>),
    ImportsUnsupported,
    Fragment(ClosedComptimeRejection),
}

impl From<String> for ClosedComptimePlanError {
    fn from(message: String) -> Self { Self::Project(message) }
}

/// Select the actual consumer target and its authored parameter table once.
/// The test/opt-in evaluator layout is separate from the artifact's runtime
/// profile. This operation does not resolve evaluator identity, launch
/// either configured tool, or write source/cache/artifact files.
pub fn plan_closed_comptime_fragment(
    root: &Path, target: &str, profile: &str, id: &str, evaluator_layout: ScalarTargetLayout,
) -> Result<ClosedComptimeFragment, ClosedComptimePlanError> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let (configuration, document) = ProjectConfiguration::load_with_manifest(&root)?;
    let project = ProjectContext::from_manifest(&root, target, document.as_table())?;
    let parameters = declared_comptime_parameters(&configuration, &document, target, profile)?;
    let mut entry = None;
    project_sources::load_with_reader(&root, &project, &configuration,
        |path, opened| project_sources::read_source(path, opened).map_err(ClosedComptimePlanError::Project),
        |source, revision, text| {
            let path = source.path.as_path().to_str().ok_or("source path is not UTF-8").map_err(str::to_owned)?;
            let identity = SourceIdentity::new(source.project.clone(), source.package.clone(),
                path.into(), revision.0.clone());
            let (fresh, diagnostics) = parse_source_with_snapshot(identity, text, &project.comment_categories);
            if !diagnostics.is_empty() { return Err(ClosedComptimePlanError::Parse(diagnostics)); }
            let imports = wosy_protocol::imports_from_cst(&fresh.result().canonical_cst().root)?;
            if !imports.is_empty() { return Err(ClosedComptimePlanError::ImportsUnsupported); }
            entry = Some(fresh);
            Ok(imports)
        },
    ).map_err(|error| match error {
        SourceError::Observe(error) => error,
        SourceError::Graph(error) => ClosedComptimePlanError::Project(format!("{error:?}")),
    })?;
    let entry = entry.ok_or_else(|| ClosedComptimePlanError::Project("target has no entry".into()))?;
    derive_closed_comptime_fragment(&entry, id, evaluator_layout, |name| {
        parameters.get(name).map(|value| match value {
            DeclaredComptimeParameter::Integer(value) => ComptimeParameter::Integer(*value),
            DeclaredComptimeParameter::Boolean(value) => ComptimeParameter::Boolean(*value),
            DeclaredComptimeParameter::Other => ComptimeParameter::UnsupportedValue,
        })
    }).map_err(ClosedComptimePlanError::Fragment)
}
