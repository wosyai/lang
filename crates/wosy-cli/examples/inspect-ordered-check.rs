use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use wosy_compiler::{
    check_loaded_scalar_ordered_local_proof_contexts, MainFlowProofReceipt, ScalarOrderedCheckerOutcome,
    ScalarPreparationInputs, ScalarTargetLayout,
};
use wosy_project::{ProjectConfiguration, ProjectContext};

#[allow(dead_code)]
#[path = "../src/canonical_loader.rs"]
mod canonical_loader;
use canonical_loader::{load_canonical_project_inputs, CanonicalLoadPurpose};

fn inspect(root: &Path, target: &str, profile: &str) -> Result<MainFlowProofReceipt, &'static str> {
    let root = root.canonicalize().map_err(|_| "invalid project root")?;
    let (configuration, manifest) =
        ProjectConfiguration::load_with_manifest(&root).map_err(|_| "invalid manifest")?;
    let project = ProjectContext::from_manifest(&root, target, manifest.as_table())
        .map_err(|_| "invalid target")?;
    let (_, _, artifact, _) = configuration.artifact_profile(target, profile)
        .map_err(|_| "invalid target profile")?;
    let layout = artifact.target_layout().map_err(|_| "unsupported target layout")?;
    let target_layout = ScalarTargetLayout {
        pointer_size: layout.pointer_size, pointer_alignment: layout.pointer_alignment,
    };
    let loaded = load_canonical_project_inputs(
        &root, &project, &configuration, CanonicalLoadPurpose::LocalProofs, || {},
    ).map_err(|_| "canonical source load failed")?;
    check_loaded_scalar_ordered_local_proof_contexts(
        loaded.declarations, loaded.proofs.into_values(), loaded.initialization_order, ScalarPreparationInputs { target_layout },
    ).map_err(|outcome| match outcome {
        ScalarOrderedCheckerOutcome::Invalid(_) => "checker rejected invalid input",
        _ => "checker did not complete analysis",
    })
}

fn render(report: &MainFlowProofReceipt) -> String {
    let actual = report.diagnostics.len();
    let nested = report.diagnostics.iter().filter(|failure| failure.invocation.outer_call.is_some()).count();
    let unused = usize::from(report.symbolic_diagnostic.is_some()) + report.additional_symbolic_diagnostics.len();
    let mut text = format!(
        "ordered check: complete\ndirective failures: {}\nactual proof failures: {actual} ({nested} nested)\nunused proof failures: {unused}\n",
        report.directives.len(),
    );
    if let Some(entry) = &report.entry_evidence {
        use std::fmt::Write as _;
        let _ = writeln!(text, "directive {} entry: {}..{}; failure-time: {}",
            entry.directive_index + 1, entry.entry_lower, entry.entry_upper, entry.failure_value);
    }
    text
}

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [root, target, profile] = args.as_slice() else {
        eprintln!("expected explicit PROJECT_ROOT TARGET PROFILE");
        return ExitCode::from(2);
    };
    let (Some(target), Some(profile)) = (target.to_str(), profile.to_str()) else {
        eprintln!("invalid target or profile");
        return ExitCode::from(2);
    };
    let report = match inspect(Path::new(root), target, profile) {
        Ok(report) => report,
        Err(reason) => {
            eprintln!("ordered check: incomplete: {reason}");
            return ExitCode::from(2);
        }
    };
    if io::stdout().lock().write_all(render(&report).as_bytes()).is_err() {
        return ExitCode::from(2);
    }
    if !report.directives.is_empty() || !report.diagnostics.is_empty() || report.symbolic_diagnostic.is_some()
        || !report.additional_symbolic_diagnostics.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
