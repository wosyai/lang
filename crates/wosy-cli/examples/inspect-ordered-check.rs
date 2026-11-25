use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use wosy_compiler::{MainFlowProofReceipt, ScalarCheckerScope, ScalarOrderedCheckerOutcome};

#[allow(dead_code)]
#[path = "../src/canonical_loader.rs"]
mod canonical_loader;
#[allow(dead_code)]
#[path = "../src/configured_check.rs"]
mod configured_check;
use configured_check::{
    check_target, CheckPublicationError, ConfiguredCheckLoadError, InternalCheckStatus,
    PlannedCheckEffect, PlannedCheckPublication,
};

fn load_error(error: ConfiguredCheckLoadError) -> &'static str {
    match error {
        ConfiguredCheckLoadError::Root => "invalid project root",
        ConfiguredCheckLoadError::Manifest => "invalid manifest",
        ConfiguredCheckLoadError::Target => "invalid target",
        ConfiguredCheckLoadError::Profile => "invalid target profile",
        ConfiguredCheckLoadError::PackageSourceRoot(_) => "invalid declared package source root",
        ConfiguredCheckLoadError::Layout => "unsupported target layout",
        ConfiguredCheckLoadError::Dependency(_) => "invalid dependency check target",
        ConfiguredCheckLoadError::Canonical(_) => "canonical source load failed",
        ConfiguredCheckLoadError::DormantPath => "invalid dormant source path",
        ConfiguredCheckLoadError::Dormant(_) => "explicit dormant source load failed",
    }
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

fn inspect_effects(text: &mut String, publication: &PlannedCheckPublication<'_>) {
    use std::fmt::Write as _;
    let _ = writeln!(text, "source texts: {}", publication.texts.len());
    match &publication.status {
        InternalCheckStatus::Zero => { let _ = writeln!(text, "authored status: 0"); }
        InternalCheckStatus::FirstNonzero(report) => {
            let _ = writeln!(text, "authored status: {}", report.code());
        }
    }
    for effect in &publication.effects {
        let bytes = match effect {
            PlannedCheckEffect::Stdout(bytes) => {
                text.push_str("stdout: ");
                *bytes
            }
            PlannedCheckEffect::Stderr(bytes) => {
                text.push_str("stderr: ");
                *bytes
            }
            PlannedCheckEffect::DeferredFile { relative, request, source } => {
                let _ = write!(text, "file {:?} from {:?}: ", relative, source.path);
                request.text.as_ref()
            }
        };
        for byte in bytes {
            let _ = write!(text, "{byte:02x}");
        }
        text.push('\n');
    }
}

fn main() -> ExitCode {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [root, target, profile, options @ ..] = args.as_slice() else {
        eprintln!("expected explicit PROJECT_ROOT TARGET PROFILE [--main-flow-only] [--dormant PATH]... [--inspect-effects]");
        return ExitCode::from(2);
    };
    let (Some(target), Some(profile)) = (target.to_str(), profile.to_str()) else {
        eprintln!("invalid target or profile");
        return ExitCode::from(2);
    };
    let mut scope = ScalarCheckerScope::CurrentPackage;
    let mut dormant_paths = Vec::new();
    let mut show_effects = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        if option == "--main-flow-only" && scope == ScalarCheckerScope::CurrentPackage {
            scope = ScalarCheckerScope::MainFlowOnly;
        } else if option == "--inspect-effects" && !show_effects {
            show_effects = true;
        } else if option == "--dormant" {
            let Some(path) = options.next() else {
                eprintln!("expected path after --dormant");
                return ExitCode::from(2);
            };
            dormant_paths.push(path.as_os_str());
        } else {
            eprintln!("invalid inspector option");
            return ExitCode::from(2);
        }
    }
    let receipt = match check_target(Path::new(root), Some(target), profile, scope, dormant_paths) {
        Ok(receipt) => receipt,
        Err(error) => {
            eprintln!("ordered check: incomplete: {}", load_error(error));
            return ExitCode::from(2);
        }
    };
    let publication = match receipt.plan_publication() {
        Ok(publication) => publication,
        Err(CheckPublicationError::Incomplete(ScalarOrderedCheckerOutcome::Invalid(_))) => {
            eprintln!("ordered check: incomplete: checker rejected invalid input");
            return ExitCode::from(2);
        }
        Err(CheckPublicationError::Incomplete(_)) => {
            eprintln!("ordered check: incomplete: checker did not complete analysis");
            return ExitCode::from(2);
        }
        Err(CheckPublicationError::InvalidPath { source, request }) => {
            eprintln!("ordered check: invalid output path {:?} in {:?}", request.path, source.path);
            return ExitCode::from(2);
        }
        Err(CheckPublicationError::ConflictingPath { source, request }) => {
            eprintln!("ordered check: conflicting output path {:?} in {:?}", request.path, source.path);
            return ExitCode::from(2);
        }
    };
    let report = publication.report;
    let mut output = render(report);
    if show_effects {
        inspect_effects(&mut output, &publication);
    }
    if io::stdout().lock().write_all(output.as_bytes()).is_err() {
        return ExitCode::from(2);
    }
    if !report.directives.is_empty() || !report.diagnostics.is_empty() || report.symbolic_diagnostic.is_some()
        || !report.additional_symbolic_diagnostics.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
