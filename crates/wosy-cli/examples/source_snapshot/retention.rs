use super::{milliseconds, Result};
use std::io::Write;
use std::path::Path;
use std::time::Instant;
use wosy_cli::source_preparation::prepare;
use wosy_daemon::SourceStore;
use wosy_protocol::NodeId;

fn reply(value: serde_json::Value, without_close: bool) -> Result<()> {
    println!("{value}");
    if without_close {
        std::io::stdout().flush()?;
        // Exit after acknowledgment without giving the tree a graceful flush.
        std::process::exit(0);
    }
    Ok(())
}

pub(super) async fn publish_and_pin(store: &SourceStore, project: &Path, name: &str) -> Result<()> {
    let prepared = prepare(project, None, None)?;
    let start = Instant::now();
    let published = store.publish(prepared.root, &prepared.delta).await?;
    let published_ms = milliseconds(start);
    let start = Instant::now();
    let pin = store.pin_source(prepared.root, name).await?;
    reply(serde_json::json!({
        "name": pin.name, "root": pin.root.hex(), "ack_ms": milliseconds(start),
        "published_ms": published_ms, "published_nodes": published.written,
    }), true)
}

pub(super) async fn run(
    store: &SourceStore, scope: NodeId, command: &str, args: &mut impl Iterator<Item=String>,
) -> Result<()> {
    if command == "--source-pins" {
        let start = Instant::now();
        let pins: Vec<_> = store.source_pins(scope)?.into_iter().map(|pin| {
            serde_json::json!({ "name": pin.name, "root": pin.root.hex() })
        }).collect();
        return reply(serde_json::json!({ "pins": pins, "query_ms": milliseconds(start) }), false);
    }
    let name = args.next().ok_or("expected pin name")?;
    let without_close = args.next().as_deref() == Some("--without-close");
    let start = Instant::now();
    let value = if command == "--source-pin" {
        let pin = store.pin_source(scope, &name).await?;
        serde_json::json!({ "name": pin.name, "root": pin.root.hex(), "ack_ms": milliseconds(start) })
    } else {
        let removed = store.unpin_source(scope, &name).await?;
        serde_json::json!({ "removed": removed, "ack_ms": milliseconds(start) })
    };
    reply(value, without_close)
}
