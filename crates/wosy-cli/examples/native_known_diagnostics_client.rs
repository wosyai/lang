//! Developer-only client. Build with
//! `--features native-diagnostics --example native_known_diagnostics_client`.
//! Run with PROJECT TARGET PROFILE PRIVATE_DIRECTORY after the daemon example
//! prints LISTENING. The project must have an acknowledged live source, and
//! the daemon must already own that same published root.
#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use codespan_reporting::term::termcolor::Buffer;
    use std::{io::Write, path::PathBuf, time::Duration};
    use wosy_cli::{source_recovery::ClientSourceLimits,
        source_state::{AcknowledgedBuildDiagnostics as Outcome, SqliteSourceState}};
    use wosy_protocol::CstExpansionBudget;

    let args: [std::ffi::OsString; 4] = std::env::args_os().skip(1).collect::<Vec<_>>().try_into()
        .map_err(|_| "expected PROJECT TARGET PROFILE PRIVATE_DIRECTORY")?;
    let [project, target, profile, directory] = args;
    let target = target.into_string().map_err(|_| "target must be UTF-8")?;
    let profile = profile.into_string().map_err(|_| "profile must be UTF-8")?;
    let mut local = SqliteSourceState::open(&PathBuf::from(project),
        ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 })?;
    match local.native_known_build_diagnostics(&PathBuf::from(directory), &target, &profile,
        Duration::from_secs(10), 8 << 20, CstExpansionBudget::new(16 << 20, 200_000, 1024, 8 << 20)).await?
    {
        Outcome::Report(report) => {
            let mut buffer = Buffer::no_color();
            report.emit_to(&mut buffer)?;
            std::io::stdout().lock().write_all(buffer.as_slice())?;
            Ok(())
        }
        Outcome::NotAcknowledged => Err("no acknowledged source".into()),
        Outcome::StaleSelection => Err("stale acknowledged source".into()),
        Outcome::WorkspaceChanged => Err("workspace changed after acknowledging source".into()),
        Outcome::ProfileNotSelected => Err("profile is not in the acknowledged source".into()),
        Outcome::NativeReplayRequired => Err("daemon needs source replay; this endpoint does not accept replay".into()),
        Outcome::ReplayNotAccepted => Err("unexpected replay retry".into()),
        Outcome::Refused(reason) => Err(format!("diagnostic session refused: {reason:?}").into()),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("Native known-root diagnostics are supported only on Linux");
    std::process::ExitCode::FAILURE
}
