use super::{milliseconds, typed_preparation, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_cli::source_preparation::PreparedSources;
use wosy_daemon::{BaseCstAdmissionLimits, SessionFrameReader, SourceStorageWorker, SourceStore};
use wosy_project::SourceNodeId;
use wosy_protocol::{BaseCstProposal, BaseCstProposalBinding};

async fn decode(mut frame: &[u8]) -> Result<BaseCstProposal> {
    SessionFrameReader::default().read(&mut frame).await?.ok_or("missing framed proposal".into())
}

fn cas_files(store: &Path) -> Result<Vec<(std::ffi::OsString, u64)>> {
    let path = store.join("cas");
    if !path.exists() { return Ok(Vec::new()); }
    let mut files = std::fs::read_dir(path)?.map(|entry| {
        let entry = entry?;
        Ok((entry.file_name(), entry.metadata()?.len()))
    }).collect::<std::io::Result<Vec<_>>>()?;
    files.sort_unstable();
    Ok(files)
}

pub(super) async fn verify(
    project: &Path, dir: &Path, binding: BaseCstProposalBinding, frame: &[u8], full: PreparedSources,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(dir, 8 << 20)?);
    let limit = |value| NonZeroUsize::new(value).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), limit(1), limit(8), limit(50_000),
    )?;
    let limits = BaseCstAdmissionLimits {
        records: 25_000, record_bytes: 16 << 20, graph_edges: 125_000,
        expanded_record_bytes: 48 << 20, cst_occurrences: 150_000,
        cst_depth: 128, authored_bytes: 2 << 20,
    };
    let lifetime = Duration::from_secs(30);
    let base = binding.selected.root;
    let root = binding.proposed;
    assert_eq!(root, full.root);
    let before = store.plan_source_retention(base, &[], &[], 50_000)?;
    let cas = cas_files(dir)?;
    let request = decode(frame).await?;
    let start = Instant::now();
    let owner = worker.prepare_base_cst_declarations(binding.clone(), request, limits, 1 << 20,
        Duration::from_millis(500)).await?;
    let prepare_ms = milliseconds(start);
    let failed = worker.prepare_base_cst_declarations(binding.clone(), decode(frame).await?, limits,
        owner.declaration_bytes() - 1, lifetime).await.err().expect("tighter prepared bytes refused");
    assert!(failed.to_string().contains("prepared base declaration payload exceeds its encoded byte budget"), "{failed}");
    let inspected = worker.try_inspect(base, &[], &[])?.finish().await?;
    assert_eq!(inspected.inventory.plan.active_read_roots, before.active_read_roots + 1);
    // The actual one-slot worker is free; the retained owner holds the source
    // read, not the execution permit. Its protection outlives lease expiry.
    tokio::time::timeout(Duration::from_secs(3), async {
        while owner.lease_is_live() { tokio::time::sleep(Duration::from_millis(10)).await; }
    }).await?;
    let mut other_owners = Vec::new();
    for _ in 1..8 {
        let next = worker.prepare_base_cst_declarations(binding.clone(), decode(frame).await?, limits,
            1 << 20, lifetime).await?;
        assert_eq!(next.identity(), owner.identity());
        other_owners.push(next);
    }
    let refusal = worker.prepare_base_cst_declarations(binding.clone(), decode(frame).await?, limits,
        1 << 20, lifetime).await.err().expect("expired but retained owner still uses a quota slot");
    assert_eq!(refusal.to_string(), "live base declaration preparations are at capacity");
    drop(other_owners.pop().expect("held companion"));
    let freed = worker.prepare_base_cst_declarations(binding.clone(), decode(frame).await?, limits,
        1 << 20, lifetime).await?;
    drop(freed);
    drop(other_owners);
    assert_eq!(store.plan_source_retention(base, &[], &[], 50_000)?.active_read_roots,
        before.active_read_roots + 1);
    assert_eq!(cas_files(dir)?, cas);
    assert!(store.snapshot(root)?.is_none());
    assert_eq!((owner.admission().base, owner.admission().proposed, owner.admission().attempt),
        (base, root, binding.attempt));
    let identity = owner.identity().digest();
    assert_ne!(identity, root);
    assert_ne!(identity, base);

    // Independent, ordinary rooted declaration/typed preparation only on our
    // full-source oracle. The candidate path never publishes this root.
    let oracle_path = dir.with_extension("base-owner-oracle");
    let oracle = SourceStore::open(&oracle_path, 8 << 20)?;
    oracle.publish(full.root, &full.delta).await?;
    {
        let published = oracle.project_declarations(root).await?.ok_or("ordinary root missing")?;
        assert_eq!(published.entry_projection, owner.entry_projection());
        assert_eq!(published.modules().len(), owner.modules().len());
        for (ordinal, candidate) in owner.modules().iter().enumerate() {
            let module = candidate.module();
            let rooted = &published.modules()[ordinal];
            let ordinary = published.declarations(SourceNodeId(ordinal)).await?.expect("ordinary module");
            assert_eq!((&module.key, &module.source, module.module, module.body, module.projection),
                (&rooted.key, &rooted.source, rooted.module, rooted.body, rooted.projection));
            assert_eq!(serde_json::to_vec(candidate.executable_declarations())?,
                serde_json::to_vec(ordinary.executable_declarations())?);
            assert_eq!(candidate.authored_diagnostics(), ordinary.authored_diagnostics());
            assert_eq!(module.imports.len(), rooted.imports.len());
            for (link, expected) in module.imports.iter().zip(&rooted.imports) {
                assert_eq!((&link.authored, link.target), (&expected.authored, expected.target));
            }
            for site in candidate.executable_declarations().namespaces() {
                assert_eq!(candidate.authored_span(site.span), ordinary.authored_span(site.span));
            }
        }
    }
    let profiles = ["dev", "native"];
    for profile in profiles {
        let typed = typed_preparation::run(&oracle, project, root, profile, Some("--oracle"), None, None).await?;
        assert_eq!(typed["project_ast"], owner.entry_projection().hex());
        assert_eq!(typed["ready"], true);
    }
    oracle.close().await?;
    let result = serde_json::json!({
        "identity": identity.hex(), "entry_projection": owner.entry_projection().hex(),
        "modules": owner.modules().len(), "unique_bodies": owner.unique_bodies(),
        "declaration_bytes": owner.declaration_bytes(), "validated_files": owner.admission().validated_files,
        "prepare_ms": prepare_ms, "profiles": profiles, "quota_and_passive_guard": true,
    });
    drop(owner);
    assert_eq!(store.plan_source_retention(base, &[], &[], 50_000)?, before);
    assert_eq!(cas_files(dir)?, cas);
    drop(worker);
    Arc::try_unwrap(store).ok().expect("no retained source owner").close().await?;
    let reopened = SourceStore::open(dir, 8 << 20)?;
    assert!(reopened.snapshot(root)?.is_none());
    assert_eq!(reopened.plan_source_retention(base, &[], &[], 50_000)?, before);
    reopened.close().await?;
    Ok(result)
}
