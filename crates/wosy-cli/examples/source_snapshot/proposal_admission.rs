use super::{milliseconds, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wosy_daemon::{
    encode_session_frame, BaseCstAdmissionLimits, SessionFrameReader, SourceStorageWorker, SourceStore,
};
use wosy_protocol::{BaseCstProposal, BaseCstProposalBinding};

async fn decode(mut frame: &[u8]) -> Result<BaseCstProposal> {
    SessionFrameReader::default().read(&mut frame).await?.ok_or("missing framed CST proposal".into())
}

pub(super) async fn verify(
    dir: &Path, binding: BaseCstProposalBinding, frame: &[u8], superseded: Option<&[u8]>,
) -> Result<serde_json::Value> {
    let store = Arc::new(SourceStore::open(dir, 8 << 20)?);
    let limit = |value| NonZeroUsize::new(value).unwrap();
    let worker = SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), limit(1), limit(8), limit(50_000),
    )?;
    let base = binding.selected.root;
    let root = binding.proposed;
    let admission = BaseCstAdmissionLimits {
        records: 25_000, record_bytes: 16 << 20, graph_edges: 100_000,
        expanded_record_bytes: 32 << 20, cst_occurrences: 100_000,
        cst_depth: 128, authored_bytes: 2 << 20,
    };
    let ttl = Duration::from_secs(30);
    if store.snapshot(base)?.is_none() {
        // An exact valid proposal can't substitute for a missing published
        // base; cold recovery is a different operation.
        worker.validate_base_cst_proposal(binding, decode(frame).await?, admission, ttl).await?;
        return Err("unknown base unexpectedly admitted".into());
    }
    let before = store.plan_source_retention(base, &[], &[], 50_000)?;
    assert!(store.snapshot(root)?.is_none());
    if let Some(old) = superseded {
        let stale = worker.validate_base_cst_proposal(binding.clone(), decode(old).await?, admission, ttl)
            .await.err().expect("old generation rejected");
        assert_eq!(stale.to_string(), "base CST proposal does not match the bound source attempt");
    }
    let request = decode(frame).await?;
    let start = Instant::now();
    let result = worker.validate_base_cst_proposal(binding.clone(), request, admission, ttl).await?;
    let validation_ms = milliseconds(start);
    let mut reordered = decode(frame).await?;
    let tested_index_order = reordered.source_index.len() > 1;
    if tested_index_order {
        // Identical canonical source records and scope, only a different
        // declared index order from the selected client's real proposal.
        reordered.source_index.swap(0, 1);
        let altered = encode_session_frame(&reordered)?;
        let error = worker.validate_base_cst_proposal(binding.clone(), decode(&altered).await?, admission, ttl)
            .await.err().expect("noncanonical ordering rejected");
        assert_eq!(error.to_string(), "base CST proposal source index is not in canonical order");
    }
    for (limits, reason) in [
        (BaseCstAdmissionLimits { records: 0, ..admission }, "record budget"),
        (BaseCstAdmissionLimits { record_bytes: 0, ..admission }, "encoded byte budget"),
        (BaseCstAdmissionLimits { graph_edges: 0, ..admission }, "graph edge budget"),
    ] {
        let error = worker.validate_base_cst_proposal(binding.clone(), decode(frame).await?, limits, ttl)
            .await.err().expect("reduced graph limits rejected");
        assert!(error.to_string().contains(reason), "{error}");
    }
    if result.validated_files > 0 {
        for (limits, reason) in [
            (BaseCstAdmissionLimits { expanded_record_bytes: 0, ..admission }, "expanded record read budget"),
            (BaseCstAdmissionLimits { cst_occurrences: 1, ..admission }, "occurrence budget"),
            (BaseCstAdmissionLimits { cst_depth: 1, ..admission }, "syntax depth budget"),
            (BaseCstAdmissionLimits { authored_bytes: 0, ..admission }, "authored byte budget"),
        ] {
            let error = worker.validate_base_cst_proposal(binding.clone(), decode(frame).await?, limits, ttl)
                .await.err().expect("reduced changed-file work limits rejected");
            assert!(error.to_string().contains(reason), "{error}");
        }
    }
    assert_eq!((result.base, result.proposed, result.attempt), (base, root, binding.attempt));
    assert_eq!(store.plan_source_retention(base, &[], &[], 50_000)?, before);
    assert!(store.snapshot(root)?.is_none());
    drop(worker);
    Arc::try_unwrap(store).ok().expect("only fixture store owner").close().await?;
    let reopened = SourceStore::open(dir, 8 << 20)?;
    assert!(reopened.snapshot(root)?.is_none());
    assert_eq!(reopened.plan_source_retention(base, &[], &[], 50_000)?, before);
    reopened.close().await?;
    Ok(serde_json::json!({
        "base_records": result.base_records, "novel_records": result.novel_records,
        "validated_files": result.validated_files, "validation_ms": validation_ms,
        "stale_attempt_rejected": superseded.is_some(), "noncanonical_order_rejected": tested_index_order,
    }))
}
