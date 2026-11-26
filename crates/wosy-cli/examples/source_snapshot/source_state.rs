use super::{milliseconds, typed_preparation, Result};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;
use wosy_cli::source_recovery::ClientSourceLimits;
use wosy_cli::source_state::{LocalSourcePin, ReplaySource, SourceAttempt, SourceReplay, SqliteSourceState};
use wosy_daemon::{encode_session_frame, SessionFrameReader, SourceStore};
use wosy_project::PackageRelativePath;
use wosy_protocol::{BaseCstProposal, Delta, FileKey, NodeId, SourceUpdate, BASE_CST_DELTA_VERSION};

fn local_pin(pin: &LocalSourcePin) -> serde_json::Value {
    serde_json::json!({
        "name": pin.name, "target": pin.target, "root": pin.root.hex(), "generation": pin.generation,
    })
}

pub(super) async fn run(
    project: &Path, dir: &Path, args: &mut impl Iterator<Item = String>,
) -> Result<()> {
    let target = args.next().ok_or("expected fixture target")?;
    let mode = args.next().ok_or("expected local source action")?;
    let start = Instant::now();
    let limits = if matches!(mode.as_str(), "stage-limited" | "proposal-limited" | "git-base-projection-limited") {
        ClientSourceLimits {
            nodes: args.next().ok_or("expected fixture node limit")?.parse()?,
            encoded_bytes: args.next().ok_or("expected fixture byte limit")?.parse()?,
        }
    } else { ClientSourceLimits { nodes: 25_000, encoded_bytes: 16 << 20 } };
    let mut client = SqliteSourceState::open(project, limits)?;
    let open_ms = milliseconds(start);
    let start = Instant::now();
    let output = match mode.as_str() {
        "stage" | "stage-limited" => {
            let stage = client.stage(&target)?;
            serde_json::json!({
                "root": stage.root.hex(), "parsed": stage.parsed, "reused": stage.reused,
                "new_nodes": stage.new_nodes, "new_bytes": stage.new_bytes,
                "storage_reads": stage.storage_reads,
                "attempt": stage.pending.map(SourceAttempt::generation),
                "stage_ms": milliseconds(start),
            })
        }
        "state" => serde_json::json!({}),
        "current-entry-cst" | "current-entry-cst-restage" => {
            let encoding: usize = args.next().ok_or("expected fixture JSON byte limit")?.parse()?;
            let expanded: usize = args.next().ok_or("expected fixture record byte limit")?.parse()?;
            let authored: usize = args.next().ok_or("expected fixture authored byte limit")?.parse()?;
            let budget = || wosy_protocol::CstExpansionBudget::new(expanded, 20_000, 128, authored);
            let captured = client.capture_acknowledged_source(&target)?.ok_or("fixture needs an acknowledged entry")?;
            let current = if mode.ends_with("-restage") {
                let alternate = args.next().ok_or("expected a disposable fixture's alternate source")?;
                let store = SourceStore::open(dir, 8 << 20)?;
                let entry = wosy_project::ProjectContext::load_parse_target(project, Some(&target))?.source_path();
                let original = std::fs::read(&entry)?;
                let mut writer = SqliteSourceState::open(project, limits)?;
                std::fs::write(&entry, alternate)?;
                let next = writer.stage(&target)?.pending.ok_or("fixture did not issue the alternate root")?;
                assert_ne!(next.root(), captured.selection().root);
                super::framed_reconciliation::confirm(&store, &writer, &target, next).await?;
                std::fs::write(&entry, original)?;
                let restaged = writer.stage(&target)?.pending.ok_or("fixture did not reissue the original root")?;
                assert_eq!(restaged.root(), captured.selection().root);
                assert!(restaged.generation() > captured.generation());
                super::framed_reconciliation::confirm(&store, &writer, &target, restaged).await?;
                assert!(captured.entry_cst_current_json(&mut client, budget(), encoding)?.is_none());
                store.close().await?;
                client.capture_acknowledged_source(&target)?.ok_or("fixture lost the new acknowledgement")?
            } else { captured.clone() };
            let started = Instant::now();
            let json = current.entry_cst_current_json(&mut client, budget(), encoding)?
                .ok_or("fixture's current entry became stale")?;
            serde_json::json!({ "root": current.selection().root.hex(), "generation": current.generation(),
                "prior_generation": captured.generation(), "json": String::from_utf8(json)?, "view_ms": milliseconds(started) })
        }
        "reconcile-acknowledged" | "reconcile-acknowledged-limited" => {
            super::source_reconciliation::run(&mut client, dir, &target, mode.ends_with("-limited")).await?
        }
        "reconcile-open-build" | "reconcile-open-build-limited" | "reconcile-open-build-quota" => {
            let profile = args.next().ok_or("expected Build profile")?;
            super::source_reconciliation::run_build(&mut client, dir, &target, &profile, &mode).await?
        }
        "reconcile-framed-build" | "reconcile-framed-stale" => {
            let profile = args.next().ok_or("expected Build profile")?;
            super::framed_reconciliation::run(&mut client, dir, &target, &profile, mode.ends_with("-stale")).await?
        }
        "reconcile-framed-generation" => {
            let profile = args.next().ok_or("expected Build profile")?;
            let edited = args.next().ok_or("expected disposable fixture edit")?;
            super::framed_reconciliation::run_generation(&mut client, project, dir, &target, &profile, &edited).await?
        }
        "reconcile-terminal-report" => {
            let profile = args.next().ok_or("expected Build profile")?;
            let mode = args.next().ok_or("expected terminal report control")?;
            super::framed_reconciliation::run_terminal_report(&mut client, project, dir, &target, &profile, &mode).await?
        }
        "replay-acknowledged-for" => {
            let root = NodeId::parse_hex(&args.next().ok_or("expected captured root")?)?;
            let owner = args.next().ok_or("expected captured workspace")?;
            let selected = wosy_protocol::BuildSourceSelection { root, project: owner, target: target.clone() };
            serde_json::json!({ "matches": client.replay_acknowledged_source(&selected)?.is_some() })
        }
        "git-base-projection" | "git-base-projection-limited" => {
            let root = NodeId::parse_hex(&args.next().ok_or("expected fixture proposal")?)?;
            let generation = args.next().ok_or("expected fixture attempt")?.parse()?;
            let package = args.next().ok_or("expected fixture package")?;
            let projection = client.pending_git_base_projection(&target, SourceAttempt::new(root, generation), &package)?;
            let projection = projection.map(|projection| {
                let files: Vec<_> = projection.files.into_iter().map(|(path, file)| serde_json::json!({
                    "path": path, "revision": file.revision.hex(), "authored": file.authored.hex(),
                    "executable": file.executable.hex(),
                })).collect();
                serde_json::json!({
                    "project": projection.project, "target": projection.target, "package": projection.package,
                    "root": projection.attempt.root().hex(), "attempt": projection.attempt.generation(),
                    "policy": projection.parser_policy.hex(), "tree": projection.installed.tree(),
                    "locked": NodeId(*projection.installed.locked().as_bytes()).hex(),
                    "revision": projection.installed.revision(), "files": files,
                })
            });
            serde_json::json!({ "projection": projection, "project_ms": milliseconds(start) })
        }
        "framed-base-projection" => {
            let root = NodeId::parse_hex(&args.next().ok_or("expected fixture proposal")?)?;
            let generation = args.next().ok_or("expected fixture attempt")?.parse()?;
            super::framed_projection::verify(dir, &mut client, &target, SourceAttempt::new(root, generation)).await?
        }
        "proposal" | "proposal-oracle" | "proposal-limited" | "proposal-admission" | "proposal-stale"
        | "proposal-preparation" => {
            let root = NodeId::parse_hex(&args.next().ok_or("expected fixture proposal")?)?;
            let generation = args.next().ok_or("expected fixture attempt")?.parse()?;
            let attempt = SourceAttempt::new(root, generation);
            let (binding, proposed) = client.pending_base_cst_proposal(&target, attempt)?;
            let proposal_ms = milliseconds(start);
            let encoded = encode_session_frame(&proposed)?;
            let admission = match mode.as_str() {
                "proposal-admission" => Some(super::proposal_admission::verify(dir, binding, &encoded, None).await?),
                "proposal-preparation" => {
                    let full = client.replay(&target, ReplaySource::Pending)?.ok_or("expected pending source")?;
                    Some(super::prepared_proposal::verify(project, dir, binding, &encoded, full.source).await?)
                }
                "proposal-stale" => {
                    assert!(client.abandon(&target, attempt)?);
                    let restaged = client.stage(&target)?.pending.ok_or("fixture did not restage")?;
                    assert_eq!(restaged.root(), root);
                    assert!(restaged.generation() > attempt.generation());
                    let (current, renewed) = client.pending_base_cst_proposal(&target, restaged)?;
                    let next = encode_session_frame(&renewed)?;
                    Some(super::proposal_admission::verify(dir, current, &next, Some(&encoded)).await?)
                }
                _ => None,
            };
            let restored = SessionFrameReader::default().read::<_, BaseCstProposal>(&mut encoded.as_ref())
                .await?.ok_or("expected a framed base CST proposal")?;
            assert_eq!(restored.version, BASE_CST_DELTA_VERSION);
            assert_eq!(encode_session_frame(&restored)?, encoded);
            let mut added = Delta::default();
            for (id, bytes) in restored.files.iter()
                .flat_map(|file| file.added.iter().map(|node| (node.id(), node.encoded())))
                .chain(restored.source_index.iter().map(|node| (node.id(), node.encoded())))
            {
                assert_eq!(wosy_protocol::Node::id(bytes), id);
                assert!(added.nodes.insert(id, bytes.to_vec()).is_none());
            }
            let files: Vec<_> = restored.files.iter().map(|file| serde_json::json!({
                "file": file.file, "old": file.old_file_root.map(NodeId::hex),
                "new": file.new_file_root.map(NodeId::hex), "added": file.added.len(),
            })).collect();
            let oracle = if mode == "proposal-oracle" {
                let base = client.replay(&target, ReplaySource::Selected)?.ok_or("expected selected source")?;
                let full = client.replay(&target, ReplaySource::Pending)?.ok_or("expected pending source")?;
                let exact: Vec<_> = full.source.delta.nodes.iter()
                    .filter(|(id, _)| !base.source.delta.nodes.contains_key(id)).collect();
                assert!(added.nodes.iter().eq(exact));
                assert_eq!((restored.base, restored.root), (base.source.root, full.source.root));
                let store = SourceStore::open(dir, 8 << 20)?;
                assert!(store.snapshot(restored.root)?.is_none());
                let result = store.publish_update(SourceUpdate {
                    base: restored.base, root: restored.root, added: Some(&added),
                    base_payload: &base.source.delta,
                }).await?;
                assert_eq!(result.mode, wosy_daemon::SourcePublicationMode::Incremental);
                let from_delta = typed_preparation::run(&store, project, restored.root, "dev", Some("--oracle"), None, None).await?;
                store.close().await?;
                let other = dir.with_extension("oracle");
                let store = SourceStore::open(&other, 8 << 20)?;
                store.publish(full.source.root, &full.source.delta).await?;
                let from_full = typed_preparation::run(&store, project, full.source.root, "dev", None, None, None).await?;
                for key in ["snapshot", "project_ast", "prepared_identity", "layout", "modules", "ready", "diagnostics"] {
                    assert_eq!(from_delta[key], from_full[key]);
                }
                store.close().await?;
                true
            } else {
                false
            };
            serde_json::json!({
                "root": root.hex(), "base": restored.base.hex(), "files": files,
                "added": added.nodes.len(), "index": restored.source_index.len(),
                "wire_bytes": encoded.len(), "proposal_ms": proposal_ms, "oracle": oracle, "admission": admission,
            })
        }
        "leased-build" => {
            let profile = args.next().ok_or("expected fixture Build profile")?;
            let (mut oracle, mut framed) = (false, false);
            for option in args {
                match option.as_str() {
                    "--oracle" => oracle = true,
                    "--framed" => framed = true,
                    _ => return Err("unknown leased diagnostic fixture option".into()),
                }
            }
            let selected = client.acknowledged_build_source(&target)?.ok_or("fixture must have an acknowledged source")?;
            super::leased_build::run(project, dir, selected, &profile, oracle, framed).await?
        }
        "leased-build-report" | "leased-build-report-limited" => {
            let profile = args.next().ok_or("expected Build profile")?;
            super::leased_build::report(&mut client, dir, &target, &profile, mode.ends_with("-limited")).await?
        }
        "publish-pending-base" | "publish-pending-base-restage" => {
            let profile = args.next().ok_or("expected Build profile")?;
            super::pending_publication::run(&mut client, project, dir, &target, &profile, mode.ends_with("-restage")).await?
        }
        "pin-local" => {
            let name = args.next().ok_or("expected local fixture pin name")?;
            local_pin(&client.pin_selected_source(&target, &name)?)
        }
        "local-pins" => {
            let pins = client.local_source_pins()?;
            serde_json::json!(pins.iter().map(local_pin).collect::<Vec<_>>())
        }
        "reconcile-pin-catalog" | "reconcile-pin-catalog-collision" => super::pin_catalog::run(
            &mut client, project, dir, &target, mode.ends_with("-collision"),
        ).await?,
        "reflect-durable-source-pin" | "reflect-durable-source-pin-exit" | "reflect-durable-source-pin-order" => {
            let name = args.next().ok_or("expected fixture pin name")?;
            match mode.as_str() {
                "reflect-durable-source-pin-order" => super::pin_reflection::order(&mut client, dir, &target, &name).await?,
                _ => super::pin_reflection::reflect(&mut client, dir, &name, mode.ends_with("-exit")).await?,
            }
        }
        "unpin-local" => {
            let name = args.next().ok_or("expected local fixture pin name")?;
            let root = NodeId::parse_hex(&args.next().ok_or("expected local fixture pin root")?)?;
            let generation = args.next().ok_or("expected local fixture pin generation")?.parse()?;
            let pin = LocalSourcePin { name, target: target.clone(), root, generation };
            serde_json::json!({ "released": client.unpin_local_source(&pin)? })
        }
        "abandon" | "delayed-ack" => {
            let root = NodeId::parse_hex(&args.next().ok_or("expected fixture proposal")?)?;
            let generation = args.next().ok_or("expected fixture attempt")?.parse()?;
            let attempt = SourceAttempt::new(root, generation);
            if mode == "abandon" {
                serde_json::json!({ "abandoned": client.abandon(&target, attempt)? })
            } else {
                serde_json::json!({ "confirmed": client.acknowledge(&target, attempt)? })
            }
        }
        "submit-selected" | "submit-pending" | "publish-before-confirm" | "reflect-pin" => {
            let (SourceReplay { source, usage, attempt }, pin) = if mode == "reflect-pin" {
                let name = args.next().ok_or("expected local fixture pin name")?;
                let pinned = client.replay_local_source_pin(&name)?.ok_or("fixture has no named local source pin")?;
                (pinned.replay, Some(pinned.pin))
            } else {
                let which = if mode == "submit-selected" { ReplaySource::Selected } else { ReplaySource::Pending };
                (client.replay(&target, which)?.ok_or("fixture has no replayable root")?, None)
            };
            let replay_ms = milliseconds(start);
            let key = FileKey {
                package: args.next().ok_or("expected fixture package")?,
                path: PackageRelativePath::new(args.next().ok_or("expected fixture path")?.into())?,
            };
            let store = SourceStore::open(dir, 8 << 20)?;
            let known = store.snapshot(source.root)?.is_some();
            let start = Instant::now();
            let report = store.publish(source.root, &source.delta).await?;
            let publish_ms = milliseconds(start);
            let start = Instant::now();
            let reflected = if let Some(pin) = pin.as_ref() {
                let result = store.pin_source(pin.root, &pin.name).await?;
                assert_eq!(result.root, pin.root);
                assert_eq!(result.name, pin.name);
                Some(local_pin(pin))
            } else { None };
            let pin_ms = milliseconds(start);
            let snapshot = store.snapshot(source.root)?.ok_or("published fixture root is missing")?;
            let authored = snapshot.authored(&key)?;
            let typed = typed_preparation::run(&store, project, source.root, "dev", None, None, None).await?;
            let text = authored.cst.root.text().to_string();
            drop(snapshot);
            store.close().await?;
            let confirmed = match (mode.as_str(), attempt) {
                ("submit-pending", Some(attempt)) => client.acknowledge(&target, attempt)?,
                _ => false,
            };
            serde_json::json!({
                "root": source.root.hex(), "parsed": source.parsed, "known": known,
                "nodes": usage.nodes, "encoded_bytes": usage.encoded_bytes,
                "written": report.written, "namespace_derived": report.namespace_derived,
                "replay_ms": replay_ms, "publish_ms": publish_ms,
                "reflected_pin": reflected, "pin_ms": pin_ms,
                "confirmed": confirmed, "authored": text, "comments": authored.comments,
                "layout": typed["layout"], "ready": typed["ready"], "modules": typed["modules"],
            })
        }
        _ => return Err("unknown local source fixture action".into()),
    };
    let state = client.selection(&target)?;
    println!("{}", serde_json::json!({
        "result": output, "selected": state.selected.map(NodeId::hex),
        "pending": state.pending.map(|attempt| attempt.root().hex()),
        "attempt": state.pending.map(SourceAttempt::generation), "open_ms": open_ms,
    }));
    io::stdout().flush()?;
    // Simulate a cleanly completed local SQLite commit followed by client
    // exit before any process-local cache/connection cleanup.
    if matches!(mode.as_str(), "stage" | "publish-before-confirm" | "pin-local") { std::process::exit(0); }
    Ok(())
}
