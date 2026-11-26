use super::{framed_reconciliation::confirm, milliseconds, rooted_llvm::ordinary_worker,
    source_reconciliation::admission_limits, Result};
use std::{cell::Cell, path::Path, sync::Arc, time::{Duration, Instant}};
use wosy_cli::{source_recovery::ClientSourceLimits, source_state::SqliteSourceState};
use wosy_daemon::{InstalledGitScopeRejection, RootedLlvmProjection, SourceStore};
use wosy_project::packages::{match_installed_git_base_files, observe_installed_git_base};
use wosy_protocol::{BuildDiagnosticRequest, BuildDiagnosticResponse, BUILD_DIAGNOSTIC_VERSION};

pub(super) async fn run(
    local: &mut SqliteSourceState, project: &Path, path: &Path, target: &str,
    package: &str, profile: &str, control: &str,
) -> Result<serde_json::Value> {
    let capture = local.capture_acknowledged_source(target)?.ok_or("fixture needs an acknowledged root")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = ordinary_worker(&store)?;
    let lifetime = Duration::from_secs(30);
    let limits = admission_limits(false);
    let started = Instant::now();
    let answer = if control == "stale" {
        let entry = wosy_project::ProjectContext::load_parse_target(project, Some(target))?.source_path();
        let original = std::fs::read_to_string(&entry)?;
        let changed = original.replace("%%start", "%%start\n# WHY: segunda geração do controle ☕");
        assert_ne!(original, changed);
        let mut writer = SqliteSourceState::open(project, ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 })?;
        std::fs::write(&entry, changed)?;
        let b = writer.stage(target)?.pending.ok_or("fixture did not issue B")?;
        assert_ne!(b.root(), capture.selection().root);
        confirm(&store, &writer, target, b).await?;
        std::fs::write(&entry, original)?;
        let a = writer.stage(target)?.pending.ok_or("fixture did not reissue A")?;
        assert_eq!(a.root(), capture.selection().root);
        assert!(a.generation() > capture.generation());
        confirm(&store, &writer, target, a).await?;
        let called = Cell::new(false);
        let opened = capture.open_installed_git_session(local, package, |_, _| {
            called.set(true);
            std::future::ready(Ok(()))
        }).await?;
        assert!(opened.is_none() && !called.get());
        serde_json::json!({ "kind": "stale-unopened", "root": a.root().hex(),
            "from_generation": capture.generation(), "to_generation": a.generation() })
    } else {
        let selected = capture.selection().clone();
        let exact = &selected;
        let service = &worker;
        let opened = capture.open_installed_git_session(local, package, |scope, witness| async move {
            assert_eq!(&scope, exact);
            let count = witness.visited_files().len();
            let revision = witness.revision().to_owned();
            let tree = witness.tree().to_owned();
            if control == "reject-incomplete" {
                let metadata = observe_installed_git_base(project, package)?.ok_or("fixture lost its pristine installation")?;
                let partial = match_installed_git_base_files(project, package, witness.visited_files().iter().take(1))?
                    .ok_or("fixture did not verify the one real member")?;
                assert!(count > 1 && metadata.visited_files().is_empty());
                for insufficient in [metadata, partial] {
                    let Err(error) = service.open_installed_git_build_scope(scope.clone(), insufficient, limits, lifetime).await else {
                        return Err("insufficient installed base witness opened a session".into());
                    };
                    assert_eq!(error.downcast_ref::<InstalledGitScopeRejection>(), Some(&InstalledGitScopeRejection::Files));
                }
            }
            let mut bounded = limits;
            if control == "records" { bounded.records = 1; }
            if control == "bytes" { bounded.record_bytes = 1; }
            let lease = service.open_installed_git_build_scope(scope, witness, bounded, lifetime).await?
                .ok_or("fixture rooted source is not published")?;
            Ok((lease, count, revision, tree))
        }).await?;
        if let Some((lease, count, revision, tree)) = opened {
            let open_ms = milliseconds(started);
            let BuildDiagnosticResponse::BuildDiagnostics { source_root, diagnostics, .. } =
                lease.dispatch(BuildDiagnosticRequest::BuildDiagnostics {
                    version: BUILD_DIAGNOSTIC_VERSION, profile: profile.to_owned(),
                }).await?;
            assert_eq!(source_root, selected.root);
            assert!(diagnostics.is_empty());
            let RootedLlvmProjection::Emitted(partition) =
                lease.project_ordinary_llvm(profile.to_owned(), limits, 2 << 20).await?
            else { return Err("installed ordinary LLVM fixture rejected its valid source".into()); };
            assert_eq!(partition.source_root, source_root);
            lease.release();
            serde_json::json!({ "kind": "partition", "root": source_root.hex(), "digest": partition.digest.hex(),
                "llvm": String::from_utf8(partition.bytes)?, "witnessed_files": count, "revision": revision,
                "tree": tree, "scope_open_ms": open_ms })
        } else {
            serde_json::json!({ "kind": "not-pristine-or-not-current" })
        }
    };
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    Ok(answer)
}
