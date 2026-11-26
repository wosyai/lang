use super::{milliseconds, source_reconciliation::admission_limits, Result};
use std::{io::{self, Write}, num::NonZeroUsize, path::Path, sync::Arc, time::Instant};
use wosy_cli::source_state::{LocalSourcePin, ReplaySource, SqliteSourceState};
use wosy_daemon::{SourceStorageWorker, SourceStore};
use wosy_protocol::{SourcePinIntent, SourcePinReflection, SourcePinRetention};

fn worker(store: &Arc<SourceStore>) -> Result<SourceStorageWorker> {
    let bound = |n| NonZeroUsize::new(n).unwrap();
    SourceStorageWorker::new(Arc::clone(store), tokio::runtime::Handle::current(), bound(1), bound(2), bound(20_000))
}

async fn apply(
    worker: &SourceStorageWorker, local: &mut SqliteSourceState, intent: &SourcePinIntent,
) -> Result<(SourcePinReflection, Option<usize>)> {
    let limits = admission_limits(false);
    let observed = worker.reflect_source_pin(intent.clone(), None, limits).await?;
    if observed != SourcePinReflection::ReplayRequired { return Ok((observed, None)); }
    let replay = local.replay_source_pin_intent(intent)?.ok_or("fixture source pin advanced before replay")?;
    assert_eq!(replay.source.parsed, 0);
    Ok((worker.reflect_source_pin(intent.clone(), Some(replay.source.delta), limits).await?, Some(replay.usage.nodes)))
}

pub(super) async fn reflect(
    local: &mut SqliteSourceState, path: &Path, name: &str, without_close: bool,
) -> Result<serde_json::Value> {
    let intent = local.source_pin_intent(name)?.ok_or("fixture needs a local named source pin")?;
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let worker = worker(&store)?;
    let start = Instant::now();
    let (outcome, replayed) = apply(&worker, local, &intent).await?;
    assert_eq!(outcome, SourcePinReflection::Durable(intent.clone()));
    assert!(local.confirms_source_pin_reflection(&outcome)?);
    let result = serde_json::json!({
        "root": intent.source.root.hex(), "generation": intent.generation,
        "retained": intent.retention == SourcePinRetention::Retained,
        "replayed_nodes": replayed, "reflection_ms": milliseconds(start),
    });
    if without_close {
        println!("{}", serde_json::json!({ "result": result }));
        io::stdout().flush()?;
        std::process::exit(0);
    }
    drop(worker);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    Ok(result)
}

pub(super) async fn order(
    local: &mut SqliteSourceState, path: &Path, target: &str, name: &str,
) -> Result<serde_json::Value> {
    let old = local.source_pin_intent(name)?.ok_or("fixture needs a historical pin")?;
    assert_eq!(old.retention, SourcePinRetention::Retained);
    let root = old.source.root;
    let limits = admission_limits(false);
    let unrelated = local.replay(target, ReplaySource::Selected)?.ok_or("fixture needs a separately selected source")?;
    let direct_root = unrelated.source.root;
    assert_ne!(root, direct_root);
    let direct = Arc::new(SourceStore::open(&path.with_extension("direct-owner"), 8 << 20)?);
    direct.publish(direct_root, &unrelated.source.delta).await?;
    direct.pin_source(direct_root, name).await?;
    let direct_service = worker(&direct)?;
    assert!(direct_service.reflect_source_pin(old.clone(), None, limits).await.is_err());
    let store = Arc::new(SourceStore::open(path, 8 << 20)?);
    let service = worker(&store)?;
    {
        let source = local.replay_source_pin_intent(&old)?.ok_or("fixture historical replay unavailable")?;
        let mut small = admission_limits(false);
        small.authored_bytes = 1;
        assert!(service.reflect_source_pin(old.clone(), Some(source.source.delta), small).await.is_err());
        assert!(store.snapshot(root)?.is_none());
        assert_eq!(service.reflect_source_pin(old.clone(), None, admission_limits(false)).await?,
            SourcePinReflection::ReplayRequired);
    }
    let (ack, n) = apply(&service, local, &old).await?;
    assert_eq!(ack, SourcePinReflection::Durable(old.clone()));
    assert!(n.is_some());
    assert!(local.confirms_source_pin_reflection(&ack)?);
    assert!(local.unpin_local_source(&LocalSourcePin {
        name: name.into(), target: old.source.target.clone(), root, generation: old.generation,
    })?);
    let release = local.source_pin_intent(name)?.ok_or("fixture needs an exact release tombstone")?;
    assert_eq!((release.source.root, release.generation, release.retention), (root, old.generation, SourcePinRetention::Released));
    assert!(!local.confirms_source_pin_reflection(&ack)?);
    assert!(local.replay_source_pin_intent(&old)?.is_none());
    assert!(local.replay_source_pin_intent(&release)?.is_none());
    assert!(direct_service.reflect_source_pin(release.clone(), None, limits).await.is_err());
    assert!(direct.snapshot(root)?.is_none());
    assert_eq!(direct.source_pins(direct_root)?[0].root, direct_root);
    let (closed, replay) = apply(&service, local, &release).await?;
    assert_eq!(closed, SourcePinReflection::Durable(release.clone()));
    assert!(replay.is_none() && local.confirms_source_pin_reflection(&closed)?);
    assert!(store.source_pins(root)?.is_empty());
    assert_eq!(service.reflect_source_pin(old.clone(), None, limits).await?, SourcePinReflection::Superseded);

    // A fresh catalog can persist the release before seeing any source. A
    // delayed prior update then cannot resurrect that root or name.
    let empty = Arc::new(SourceStore::open(&path.with_extension("release-first"), 8 << 20)?);
    let replacement = worker(&empty)?;
    assert_eq!(apply(&replacement, local, &release).await?.0, SourcePinReflection::Durable(release.clone()));
    assert_eq!(replacement.reflect_source_pin(old.clone(), None, limits).await?, SourcePinReflection::Superseded);
    assert!(empty.snapshot(root)?.is_none());
    drop(replacement);
    Arc::try_unwrap(empty).ok().expect("fixture owns release-first catalog").close().await?;
    let empty = Arc::new(SourceStore::open(&path.with_extension("release-first"), 8 << 20)?);
    let replacement = worker(&empty)?;
    assert_eq!(replacement.reflect_source_pin(old.clone(), None, limits).await?, SourcePinReflection::Superseded);
    assert!(empty.snapshot(root)?.is_none());
    drop(replacement);
    Arc::try_unwrap(empty).ok().expect("fixture owns reopened release-first catalog").close().await?;

    let repinned = local.pin_selected_source(target, name)?;
    let next = local.source_pin_intent(name)?.ok_or("fixture failed to create a newer pin intent")?;
    assert!(repinned.generation > old.generation);
    assert_ne!(next.source.root, root);
    assert!(direct_service.reflect_source_pin(next.clone(), None, limits).await.is_err());
    assert_eq!(direct.source_pins(direct_root)?[0].root, direct_root);
    drop(direct_service);
    Arc::try_unwrap(direct).ok().expect("fixture owns direct catalog").close().await?;
    assert!(!local.confirms_source_pin_reflection(&closed)?);
    let updated = apply(&service, local, &next).await?.0;
    assert_eq!(updated, SourcePinReflection::Durable(next.clone()));
    assert!(local.confirms_source_pin_reflection(&updated)?);
    assert_eq!(service.reflect_source_pin(release, None, limits).await?, SourcePinReflection::Superseded);
    assert_eq!(store.source_pins(next.source.root)?[0].root, next.source.root);
    assert_eq!(store.plan_source_retention(next.source.root, &[root], &[], 20_000)?.candidate_roots, 1);
    assert!(store.unpin_source(next.source.root, name).await.is_err());
    assert!(store.pin_source(root, name).await.is_err());
    let latest = next.source.root.hex();
    drop(service);
    Arc::try_unwrap(store).ok().expect("fixture owns its catalog").close().await?;
    Ok(serde_json::json!({
        "past": root.hex(), "latest": latest, "release_before_source": true,
        "stale_receipts_and_intents_refused": true, "released_root_not_retained": true,
        "over_budget_rollback": true, "reopened_release_tombstone": true,
        "legacy_and_reflected_names_protect_both_owners": true,
    }))
}
