use super::{milliseconds, typed_preparation, Result};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;
use wosy_cli::source_recovery::RetainedSources;
use wosy_daemon::SourceStore;
use wosy_protocol::FileKey;

fn report(value: serde_json::Value) -> Result<()> {
    println!("{value}");
    io::stdout().flush()?;
    Ok(())
}

fn next_step() -> Result<()> {
    if io::stdin().read_line(&mut String::new())? == 0 {
        return Err("reconciliation fixture ended before the next step".into());
    }
    Ok(())
}

async fn submit(store: &SourceStore, client: &RetainedSources) -> Result<serde_json::Value> {
    let selected = client.selected();
    let start = Instant::now();
    let output = store.publish(selected.root, &selected.delta).await?;
    Ok(serde_json::json!({
        "written": output.written, "namespace_derived": output.namespace_derived,
        "elapsed_ms": milliseconds(start),
    }))
}

async fn inspect(
    store: &SourceStore, project: &Path, client: &RetainedSources, key: &FileKey,
) -> Result<serde_json::Value> {
    let root = client.selected().root;
    let snapshot = store.snapshot(root)?.ok_or("selected fixture root was not reconciled")?;
    let file = snapshot.authored(key)?;
    let authored = file.cst.root.text().to_string();
    let state = typed_preparation::run(store, project, root, "dev", None, None, None).await?;
    Ok(serde_json::json!({
        "authored": authored, "comments": file.comments, "source": file.cst.source,
        "identity": state["prepared_identity"], "project_ast": state["project_ast"],
        "ready": state["ready"], "modules": state["modules"], "layout": state["layout"],
    }))
}

fn selected(client: &RetainedSources) -> serde_json::Value {
    let state = client.selected();
    serde_json::json!({
        "root": state.root.hex(), "parsed": state.parsed, "reused": state.reused,
        "retained_nodes": state.delta.nodes.len(),
        "retained_node_bytes": state.delta.nodes.values().map(Vec::len).sum::<usize>(),
    })
}

pub(super) async fn run(
    project: &Path, primary: &Path, replacement: &Path, key: &FileKey,
) -> Result<()> {
    let store = SourceStore::open(primary, 8 << 20)?;
    let start = Instant::now();
    let mut client = RetainedSources::observe(project, None)?;
    let cold_observe_ms = milliseconds(start);
    let initial_root = client.selected().root;
    let initial = selected(&client);
    let publish_initial = submit(&store, &client).await?;
    let initial_state = inspect(&store, project, &client, key).await?;
    let start = Instant::now();
    client.refresh(project, None)?;
    let warm_observe_ms = milliseconds(start);
    let warm = selected(&client);
    store.close().await?;

    let store = SourceStore::open(primary, 8 << 20)?;
    let reopened = submit(&store, &client).await?;
    report(serde_json::json!({
        "initial": initial, "initial_state": initial_state, "publish_initial": publish_initial,
        "warm": warm, "reopened": reopened,
        "cold_observe_ms": cold_observe_ms, "warm_observe_ms": warm_observe_ms,
    }))?;
    next_step()?;

    let start = Instant::now();
    client.refresh(project, None)?;
    let edit_observe_ms = milliseconds(start);
    let edited = selected(&client);
    let drops_prior_root = !client.selected().delta.nodes.contains_key(&initial_root);
    let publish_edit = submit(&store, &client).await?;
    let edited_state = inspect(&store, project, &client, key).await?;
    let historical = store.snapshot(initial_root)?.ok_or("original fixture root missing")?
        .authored(key)?.cst.root.text().to_string();
    store.close().await?;
    report(serde_json::json!({
        "edited": edited, "edited_state": edited_state, "historical": historical,
        "publish_edit": publish_edit, "drops_prior_root": drops_prior_root,
        "edit_observe_ms": edit_observe_ms,
    }))?;
    next_step()?;

    // The replacement store has never received this project. The fixture
    // changes its workspace first: recover the selected bytes, not a rescan.
    let start = Instant::now();
    let store = SourceStore::open(replacement, 8 << 20)?;
    let replacement_open_ms = milliseconds(start);
    let missing = store.snapshot(client.selected().root)?.is_none();
    let recovered = submit(&store, &client).await?;
    let restored = inspect(&store, project, &client, key).await?;
    let repeat = submit(&store, &client).await?;
    report(serde_json::json!({
        "missing": missing, "recovered": recovered, "repeat": repeat, "restored": restored,
        "selected": selected(&client), "replacement_open_ms": replacement_open_ms,
    }))?;
    store.close().await
}
