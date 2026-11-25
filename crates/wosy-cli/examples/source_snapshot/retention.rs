use super::{milliseconds, Result};
use std::io::Write;
use std::path::Path;
use std::time::Instant;
use wosy_cli::source_preparation::prepare;
use wosy_daemon::{SourceRetentionInventory, SourceRetentionPlan, SourceStore};
use wosy_project::SourceNodeId;
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
    if command == "--source-plan" { return plan(store, scope, args); }
    if command == "--source-plan-readers" {
        let old = NodeId::parse_hex(&args.next().ok_or("expected fixture historical root")?)?;
        return readers(store, scope, old).await;
    }
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

fn plan(store: &SourceStore, scope: NodeId, args: &mut impl Iterator<Item=String>) -> Result<()> {
    let limit = args.next().ok_or("expected fixture retention budget")?.parse()?;
    let mut nominated = Vec::new();
    let mut kept = Vec::new();
    let mut in_keep = false;
    for arg in args {
        if arg == "--keep" {
            in_keep = true;
            continue;
        }
        let ids = if in_keep { &mut kept } else { &mut nominated };
        ids.push(NodeId::parse_hex(&arg)?);
    }
    let start = Instant::now();
    let plan = store.plan_source_retention(scope, &nominated, &kept, limit)?;
    reply(plan_json(&plan, milliseconds(start)), false)
}

fn plan_json(plan: &SourceRetentionPlan, query_ms: f64) -> serde_json::Value {
    serde_json::json!({
        "protection_epoch": plan.protection_epoch.hex(), "nominated_roots": plan.nominated_roots,
        "protected_roots": plan.protected_roots, "active_read_roots": plan.active_read_roots,
        "candidate_roots": plan.candidate_roots,
        "nodes_visited": plan.nodes_visited, "shared_nodes": plan.shared_nodes,
        "candidate_only_nodes": plan.candidate_only_nodes,
        "candidate_only_node_bytes": plan.candidate_only_node_bytes,
        "candidate_only_cas_references": plan.candidate_only_cas_references,
        "query_ms": query_ms,
    })
}

fn inventory_json(inventory: &SourceRetentionInventory, query_ms: f64) -> serde_json::Value {
    serde_json::json!({
        "plan": plan_json(&inventory.plan, query_ms),
        "protected": { "referenced": inventory.protected.referenced, "missing": inventory.protected.missing },
        "candidate_only": {
            "referenced": inventory.candidate_only.referenced, "missing": inventory.candidate_only.missing,
        },
    })
}

async fn readers(store: &SourceStore, scope: NodeId, old: NodeId) -> Result<()> {
    let observe = || -> Result<_> {
        let start = Instant::now();
        let inventory = store.inspect_source_retention(scope, &[old], &[], 20_000)?;
        Ok(inventory_json(&inventory, milliseconds(start)))
    };
    let before = observe()?;
    let direct = store.snapshot(old)?.ok_or("fixture source absent")?;
    let again = store.snapshot(old)?.ok_or("fixture source absent")?;
    let twice = observe()?;
    let graph = store.project_declarations(old).await?.ok_or("fixture declaration graph absent")?;
    drop(direct);
    drop(again);
    let graph_only = observe()?;

    // An optional payload is loaded by the real query (not the inventory).
    let body = graph.declarations(SourceNodeId(1)).await?.ok_or("fixture provider absent")?;
    let disposition = format!("{:?}", body.disposition);
    let body_id = body.id.hex();
    let reopened = observe()?;

    let in_flight = {
        let (admitted, ready) = tokio::sync::oneshot::channel();
        let reader = async {
            let typed = store.rooted_prepared_project(old, "dev").await?
                .ok_or("fixture typed project absent")?;
            assert!(typed.prepared().is_some());
            admitted.send(()).map_err(|_| "fixture observer closed")?;
            std::future::pending::<()>().await;
            drop(typed);
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        };
        tokio::pin!(reader);
        tokio::select! {
            result = &mut reader => { result?; return Err("fixture reader finished unexpectedly".into()); }
            result = ready => { result?; }
        }
        drop(graph);
        // The suspended compiler consumer is now the only holder.
        observe()?
    }; // Cancel the compiler consumer with its future still pending.
    let released = observe()?;
    reply(serde_json::json!({
        "before": before, "twice": twice, "graph_only": graph_only, "reopened": reopened,
        "in_flight": in_flight, "released": released,
        "body_disposition": disposition, "body_id": body_id,
    }), false)
}
