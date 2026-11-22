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
    check_target, ConfiguredCheckLoadError, InternalCheckEffect, InternalCheckStatus, OrderedCheckEffects,
};

fn load_error(error: ConfiguredCheckLoadError) -> &'static str {
    match error {
        ConfiguredCheckLoadError::Root => "invalid project root",
        ConfiguredCheckLoadError::Manifest => "invalid manifest",
        ConfiguredCheckLoadError::Target => "invalid target",
        ConfiguredCheckLoadError::Profile => "invalid target profile",
        ConfiguredCheckLoadError::Layout => "unsupported target layout",
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

fn inspect_effects(text: &mut String, report: &MainFlowProofReceipt, sources: usize) {
    use std::fmt::Write as _;
    let effects = OrderedCheckEffects::new(&report.directives);
    let _ = writeln!(text, "source texts: {sources}");
    match effects.selected_status() {
        InternalCheckStatus::Zero => { let _ = writeln!(text, "authored status: 0"); }
        InternalCheckStatus::FirstNonzero(report) => {
            let _ = writeln!(text, "authored status: {}", report.code());
        }
    }
    let result = OrderedCheckEffects::new(&report.directives).visit_authored_effects(|effect| {
        let bytes = match effect {
            InternalCheckEffect::Stdout(bytes) => {
                text.push_str("stdout: ");
                bytes
            }
            InternalCheckEffect::Stderr(bytes) => {
                text.push_str("stderr: ");
                bytes
            }
            InternalCheckEffect::DeferredFile(file) => {
                write!(text, "file {:?}: ", file.path)?;
                &file.text
            }
        };
        for byte in bytes {
            write!(text, "{byte:02x}")?;
        }
        text.push('\n');
        Ok::<(), std::fmt::Error>(())
    });
    debug_assert!(result.is_ok());
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
    let report = match receipt.outcome {
        Ok(report) => report,
        Err(ScalarOrderedCheckerOutcome::Invalid(_)) => {
            eprintln!("ordered check: incomplete: checker rejected invalid input");
            return ExitCode::from(2);
        }
        Err(_) => {
            eprintln!("ordered check: incomplete: checker did not complete analysis");
            return ExitCode::from(2);
        }
    };
    let mut output = render(&report);
    if show_effects {
        inspect_effects(&mut output, &report, receipt.texts.len());
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
