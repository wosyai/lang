use super::{milliseconds, typed_preparation, Result};
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;
use wosy_cli::source_recovery::{ClientSourceError, ClientSourceLimits, RetainedSources, SourceLimit};
use wosy_daemon::SourceStore;
use wosy_protocol::FileKey;

const FIXTURE_LIMITS: ClientSourceLimits = ClientSourceLimits { nodes: 20_000, encoded_bytes: 16 << 20 };

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
    let usage = client.usage();
    assert_eq!(usage.nodes, state.delta.nodes.len());
    assert_eq!(usage.encoded_bytes, state.delta.nodes.values().map(Vec::len).sum::<usize>());
    serde_json::json!({
        "root": state.root.hex(), "parsed": state.parsed, "reused": state.reused,
        "retained_nodes": usage.nodes, "retained_node_bytes": usage.encoded_bytes,
    })
}

pub(super) async fn run(
    project: &Path, primary: &Path, replacement: &Path, key: &FileKey,
) -> Result<()> {
    let store = SourceStore::open(primary, 8 << 20)?;
    let start = Instant::now();
    let mut client = RetainedSources::observe(project, None, FIXTURE_LIMITS)?;
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

fn check_limit(error: ClientSourceError, field: SourceLimit, limit: usize) {
    match error {
        ClientSourceError::LimitExceeded { field: exceeded, limit: actual } => {
            assert_eq!((exceeded, actual), (field, limit));
        }
        other => panic!("expected fixture limit rejection: {other}"),
    }
}

pub(super) async fn run_budget(project: &Path, primary: &Path, key: &FileKey) -> Result<()> {
    // Derive boundary limits from the actual canonical generation, not a
    // duplicate fixture-side size estimate.
    let usage = RetainedSources::observe(project, None, FIXTURE_LIMITS)?.usage();
    let exact = ClientSourceLimits { nodes: usage.nodes, encoded_bytes: usage.encoded_bytes };
    let at_limit = RetainedSources::observe(project, None, exact)?;
    let initial = selected(&at_limit);
    drop(at_limit);
    for (field, limit, limits) in [
        (SourceLimit::Nodes, exact.nodes - 1, ClientSourceLimits { nodes: exact.nodes - 1, ..exact }),
        (SourceLimit::EncodedBytes, exact.encoded_bytes - 1,
         ClientSourceLimits { encoded_bytes: exact.encoded_bytes - 1, ..exact }),
    ] {
        let error = RetainedSources::observe(project, None, limits)
            .err().ok_or("expected exact fixture limit rejection")?;
        check_limit(error, field, limit);
    }
    let limits = ClientSourceLimits { encoded_bytes: exact.encoded_bytes + 512, ..FIXTURE_LIMITS };
    let mut client = RetainedSources::observe(project, None, limits)?;
    assert_eq!(selected(&client), initial);
    report(serde_json::json!({
        "initial": initial, "exact_limits": true, "both_one_less_rejected": true,
        "refresh_byte_limit": limits.encoded_bytes,
    }))?;
    next_step()?;

    let start = Instant::now();
    let error = client.stage_refresh(project, None)
        .err().ok_or("expected fixture edited payload limit rejection")?;
    check_limit(error, SourceLimit::EncodedBytes, limits.encoded_bytes);
    let rejected_ms = milliseconds(start);
    let after_rejection = selected(&client);
    assert_eq!(after_rejection, initial);
    let store = SourceStore::open(primary, 8 << 20)?;
    assert!(store.snapshot(client.selected().root)?.is_none());
    let original_publish = submit(&store, &client).await?;
    let original_state = inspect(&store, project, &client, key).await?;
    report(serde_json::json!({
        "after_rejection": after_rejection, "rejected_ms": rejected_ms,
        "original_publish": original_publish, "original_state": original_state,
    }))?;
    next_step()?;

    let mut candidate = None;
    {
        let observer = async {
            let staged = client.stage_refresh(project, None)?;
            candidate = Some((staged.root(), staged.usage()));
            std::future::pending::<()>().await;
            staged.commit();
            Ok::<_, ClientSourceError>(())
        };
        tokio::pin!(observer);
        let polled = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(observer.as_mut(), cx))
        }).await;
        if let std::task::Poll::Ready(result) = polled {
            result?;
            return Err("staging fixture unexpectedly completed".into());
        }
    }
    let (candidate_root, candidate_usage) = candidate.ok_or("missing staged fixture generation")?;
    let after_cancel = selected(&client);
    assert_eq!(after_cancel, initial);
    assert_ne!(candidate_root, client.selected().root);
    assert!(store.snapshot(candidate_root)?.is_none());
    let cancelled_repeat = submit(&store, &client).await?;
    let start = Instant::now();
    let staged = client.stage_refresh(project, None)?;
    let stage_ms = milliseconds(start);
    assert_eq!(staged.root(), candidate_root);
    assert_eq!(staged.usage(), candidate_usage);
    let start = Instant::now();
    staged.commit();
    let commit_ms = milliseconds(start);
    let committed = selected(&client);
    let new_publish = submit(&store, &client).await?;
    let new_state = inspect(&store, project, &client, key).await?;
    report(serde_json::json!({
        "after_cancel": after_cancel, "candidate": candidate_root.hex(), "committed": committed,
        "cancelled_repeat": cancelled_repeat, "new_publish": new_publish, "new_state": new_state,
        "stage_ms": stage_ms, "commit_ms": commit_ms,
    }))?;
    store.close().await
}
