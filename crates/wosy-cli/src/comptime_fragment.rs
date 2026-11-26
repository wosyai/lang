//! Opt-in producer of a closed fragment, not an evaluator runner.
use crate::{
    artifact_inputs::project_configuration_identity,
    project_sources::{self, Error as SourceError},
};
use std::{ffi::OsStr, io::{self, Write}, path::{Path, PathBuf}};
use wosy_compiler::{
    derive_closed_comptime_fragment, parse_source_with_snapshot, ClosedComptimeFragment,
    ClosedComptimeRejection, ComptimeParameter, Diagnostic, ScalarTargetLayout,
};
use wosy_project::{
    declared_comptime_parameters, DeclaredComptimeParameter, ObservedClosedComptimeProfile,
    ProjectConfiguration, ProjectContext, SelectedComptimeTools,
};
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
    Ok(observe(root, target, profile, id, evaluator_layout, false)?.fragment)
}

/// One explicit local observation binding selected configuration and source
/// to the compiler's closed IR. No format for a comptime manifest or result
/// is defined here, and the literal process plans grant no launch authority.
pub struct ClosedComptimeStaging {
    fragment: ClosedComptimeFragment,
    selected: StagingSelection,
}

struct StagingSelection {
    workspace: PathBuf,
    configuration_identity: String,
    tools: SelectedComptimeTools,
}

struct FragmentObservation {
    fragment: ClosedComptimeFragment,
    staging: Option<StagingSelection>,
}

pub fn plan_closed_comptime_staging(
    root: &Path, target: &str, profile: &str, id: &str, evaluator_layout: ScalarTargetLayout,
) -> Result<ClosedComptimeStaging, ClosedComptimePlanError> {
    let observation = observe(root, target, profile, id, evaluator_layout, true)?;
    Ok(ClosedComptimeStaging { fragment: observation.fragment, selected: observation.staging.expect("staging was selected") })
}

fn observe(
    root: &Path, target: &str, profile: &str, id: &str, evaluator_layout: ScalarTargetLayout, staging: bool,
) -> Result<FragmentObservation, ClosedComptimePlanError> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let (configuration, document) = ProjectConfiguration::load_with_manifest(&root)?;
    let project = ProjectContext::from_manifest(&root, target, document.as_table())?;
    let stage_profile = if staging {
        Some(ObservedClosedComptimeProfile::select(&configuration, &document, target, profile, &project.package, id)?)
    } else { None };
    let preview_parameters;
    let parameters = match &stage_profile {
        Some(selected) => selected.parameters(),
        None => {
            preview_parameters = declared_comptime_parameters(&configuration, &document, target, profile)?;
            &preview_parameters
        }
    };
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
    let fragment = derive_closed_comptime_fragment(&entry, id, evaluator_layout, |name| {
        parameters.get(name).map(|value| match value {
            DeclaredComptimeParameter::Integer(value) => ComptimeParameter::Integer(*value),
            DeclaredComptimeParameter::Boolean(value) => ComptimeParameter::Boolean(*value),
            DeclaredComptimeParameter::Other => ComptimeParameter::UnsupportedValue,
        })
    }).map_err(ClosedComptimePlanError::Fragment)?;
    let staging = if let Some(profile) = stage_profile {
        let configuration_identity = project_configuration_identity(document.raw().as_bytes(), &configuration)?;
        Some(StagingSelection { workspace: root, configuration_identity, tools: profile.into_tools() })
    } else { None };
    Ok(FragmentObservation { fragment, staging })
}

/// Paths describe the caller-owned staging area and are never opened here.
pub struct ClosedComptimePaths<'a> {
    pub evaluator_ir: &'a Path,
    pub output_dir: &'a Path,
    pub manifest: &'a Path,
}

/// A shell-free, literal description of the already selected declaration.
pub struct ClosedComptimeInvocation<'a> {
    pub executable: &'a OsStr,
    pub working_directory: &'a Path,
    pub arguments: Vec<&'a OsStr>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct StagedEvaluatorIr {
    pub bytes: usize,
    pub blake3: blake3::Hash,
}

impl ClosedComptimeStaging {
    pub fn fragment(&self) -> &ClosedComptimeFragment { &self.fragment }
    pub fn tools(&self) -> &SelectedComptimeTools { &self.selected.tools }
    pub fn configuration_identity(&self) -> &str { &self.selected.configuration_identity }

    pub fn builder<'a>(&'a self, paths: ClosedComptimePaths<'a>) -> ClosedComptimeInvocation<'a> {
        let (_, builder) = self.tools().builder();
        self.invocation(&builder.command, &builder.args, [paths.evaluator_ir, paths.output_dir, paths.manifest])
    }

    pub fn runner<'a>(&'a self, manifest: &'a Path) -> ClosedComptimeInvocation<'a> {
        let (_, runner) = self.tools().runner();
        self.invocation(&runner.command, &runner.args, [manifest])
    }

    fn invocation<'a, const N: usize>(
        &'a self, command: &'a [String], configured: &'a [String], paths: [&'a Path; N],
    ) -> ClosedComptimeInvocation<'a> {
        ClosedComptimeInvocation {
            executable: OsStr::new(&command[0]), // the selected declaration has an executable
            working_directory: &self.selected.workspace,
            arguments: command[1..].iter().chain(configured).map(OsStr::new)
                .chain(paths.map(Path::as_os_str)).collect(),
        }
    }

    /// Stream the ordinary compiler IR exactly once to a caller-owned sink.
    /// Failed writes have no receipt; the caller owns any partial scratch
    /// output. The byte bound is egress only, not total compiler memory.
    pub fn write_evaluator_ir(&self, output: &mut impl Write, max_bytes: usize) -> io::Result<StagedEvaluatorIr> {
        let limit = max_bytes.min(1 << 20);
        let mut writer = EvaluatorWriter { output, remaining: limit, digest: blake3::Hasher::new() };
        self.fragment.llvm().write_text(&mut writer)?;
        writer.flush()?;
        Ok(StagedEvaluatorIr { bytes: limit - writer.remaining, blake3: writer.digest.finalize() })
    }
}

struct EvaluatorWriter<'a, W> {
    output: &'a mut W,
    remaining: usize,
    digest: blake3::Hasher,
}

impl<W: Write> Write for EvaluatorWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(io::Error::other("closed comptime evaluator IR exceeds byte budget"));
        }
        let count = self.output.write(bytes)?;
        self.remaining -= count;
        self.digest.update(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> { self.output.flush() }
}
