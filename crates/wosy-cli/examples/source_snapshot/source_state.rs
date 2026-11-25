use super::{milliseconds, typed_preparation, Result};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;
use wosy_cli::source_recovery::ClientSourceLimits;
use wosy_cli::source_state::{ReplaySource, SourceAttempt, SourceReplay, SqliteSourceState};
use wosy_daemon::SourceStore;
use wosy_project::PackageRelativePath;
use wosy_protocol::{FileKey, NodeId};

pub(super) async fn run(
    project: &Path, dir: &Path, args: &mut impl Iterator<Item = String>,
) -> Result<()> {
    let target = args.next().ok_or("expected fixture target")?;
    let mode = args.next().ok_or("expected local source action")?;
    let start = Instant::now();
    let limits = if mode == "stage-limited" {
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
        "submit-selected" | "submit-pending" | "publish-before-confirm" => {
            let pending = mode != "submit-selected";
            let which = if pending { ReplaySource::Pending } else { ReplaySource::Selected };
            let SourceReplay { source, usage, attempt } =
                client.replay(&target, which)?.ok_or("fixture has no replayable root")?;
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
    if matches!(mode.as_str(), "stage" | "publish-before-confirm") { std::process::exit(0); }
    Ok(())
}
