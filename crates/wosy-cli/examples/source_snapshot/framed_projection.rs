use super::{milliseconds, Result};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{self, AsyncWriteExt, DuplexStream};
use tokio::task::JoinHandle;
use wosy_cli::source_state::{SourceAttempt, SqliteSourceState};
use wosy_compiler::Diagnostic;
use wosy_daemon::{
    encode_session_frame, BaseCstAdmissionLimits, BuildSessionEnd, SessionFrameReader, SourceStorageWorker, SourceStore,
};
use wosy_protocol::{
    BaseCstProposal, BaseCstProposalBinding, BaseDeclarationConnectionId,
    BuildSessionCommand as Command, BuildSessionCommandId as Id,
    BuildSessionRefusal as Refusal, BuildSessionReply as Reply, BuildSessionRequest as Request,
    BuildSessionResponse, BASE_DECLARATION_PROJECTION_VERSION, BUILD_DIAGNOSTIC_VERSION,
};

type Answer = BuildSessionResponse<Diagnostic>;
const LIFETIME: Duration = Duration::from_secs(30);

const LIMITS: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
    records: 20_000, record_bytes: 8 << 20, graph_edges: 64_000,
    expanded_record_bytes: 16 << 20, cst_occurrences: 64_000,
    cst_depth: 128, authored_bytes: 1 << 20,
};

struct ClientStream {
    stream: DuplexStream,
    reader: SessionFrameReader,
    host: JoinHandle<Result<BuildSessionEnd>>,
    connection: Option<BaseDeclarationConnectionId>,
}

impl ClientStream {
    async fn open(worker: &Arc<SourceStorageWorker>, scope: BaseCstProposalBinding,
        bound: bool, lifetime: Duration) -> Result<Self>
    {
        let (stream, server) = io::duplex(4096);
        let root = scope.selected.root;
        let worker = Arc::clone(worker);
        let host = tokio::spawn(async move {
            if bound { worker.serve_base_declaration_session(server, scope, LIMITS, 1 << 20, lifetime).await }
            else { worker.serve_build_session(server, scope.selected, lifetime).await }
        });
        let mut client = Self { stream, reader: SessionFrameReader::default(), host, connection: None };
        client.send(1, Request::Start { version: BUILD_DIAGNOSTIC_VERSION }).await?;
        match client.receive(1).await?.reply {
            Reply::BaseDeclarationSessionStarted { version: BUILD_DIAGNOSTIC_VERSION, source_root, connection }
                if bound && source_root == root => client.connection = Some(connection),
            Reply::Started { version: BUILD_DIAGNOSTIC_VERSION, source_root } if !bound && source_root == root => {}
            _ => return Err("unexpected selected stream start".into()),
        }
        Ok(client)
    }

    async fn send(&mut self, id: u64, request: Request) -> Result<()> {
        self.stream.write_all(&encode_session_frame(&Command { id: Id(id), request })?).await?;
        Ok(())
    }

    async fn receive(&mut self, id: u64) -> Result<Answer> {
        let message: Answer = self.reader.read(&mut self.stream).await?.ok_or("missing framed reply")?;
        assert_eq!(message.id, Id(id));
        Ok(message)
    }

    async fn close(mut self, id: u64) -> Result<()> {
        self.send(id, Request::Disconnect).await?;
        assert!(matches!(self.receive(id).await?.reply, Reply::Closed));
        assert_eq!(self.host.await??, BuildSessionEnd::Disconnected);
        Ok(())
    }

    async fn eof(self) -> Result<()> {
        drop(self.stream);
        assert_eq!(self.host.await??, BuildSessionEnd::PeerClosed);
        Ok(())
    }
}

fn request(proposal: BaseCstProposal) -> Request {
    Request::ProjectBaseDeclarations { version: BASE_DECLARATION_PROJECTION_VERSION, proposal }
}

pub(super) async fn verify(
    dir: &Path, client: &mut SqliteSourceState, target: &str, attempt: SourceAttempt,
) -> Result<serde_json::Value> {
    let (old, initial) = client.pending_base_cst_proposal(target, attempt)?;
    let store = Arc::new(SourceStore::open(dir, 8 << 20)?);
    let size = |value| NonZeroUsize::new(value).unwrap();
    let worker = Arc::new(SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), size(1), size(8), size(25_000),
    )?);
    let base = old.selected.root;
    let proposed = old.proposed;
    let catalog = store.plan_source_retention(base, &[], &[], 25_000)?;

    let mut sessions = Vec::new();
    for _ in 0..63 {
        sessions.push(worker.open_build_diagnostics(old.selected.clone(), LIFETIME).await?.expect("base exists"));
    }
    let mut waiting = ClientStream::open(&worker, old.clone(), true, LIFETIME).await?;
    let extra = worker.open_build_diagnostics(old.selected.clone(), LIFETIME).await.err()
        .expect("same store's lease quota is full");
    assert_eq!(extra.to_string(), "Build diagnostic session quota exceeded");
    waiting.send(2, request(initial)).await?;
    // The wire request was already sent with the old authenticated binding.
    // A real same-content restage occurs before the client consumes its reply.
    assert!(client.abandon(target, attempt)?);
    let refreshed = client.stage(target)?.pending.expect("restaged source");
    assert_eq!(refreshed.root(), proposed);
    assert!(refreshed.generation() > attempt.generation());
    let delayed = waiting.receive(2).await?;
    let Reply::BaseDeclarationProjection { projection: ref old_projection } = delayed.reply else {
        return Err("projection on the existing full lease quota did extra admission".into());
    };
    assert_eq!(old_projection.attempt, attempt.generation());
    let prior_identity = old_projection.preparation;
    let prior_entry = old_projection.entry_projection;
    let old_connection = waiting.connection.expect("old bound connection");
    assert_eq!(old_projection.connection, old_connection);
    assert!(client.consume_base_declaration_projection(&old, old_connection, Id(2), delayed)?.is_none());
    drop(sessions);

    let source = Path::new(&old.selected.project).join(".wosy");
    let metadata_file = source.join("client.sqlite");
    let nodes_file = source.join("cst/nodes.sqlite");
    let sqlite_before = (std::fs::read(&metadata_file)?, std::fs::read(&nodes_file)?);
    let (current, proposal) = client.pending_base_cst_proposal(target, refreshed)?;
    let mut connected = ClientStream::open(&worker, current.clone(), true, LIFETIME).await?;
    let start = Instant::now();
    connected.send(2, request(proposal)).await?;
    let reply = connected.receive(2).await?;
    let elapsed_ms = milliseconds(start);
    let response_bytes = encode_session_frame(&reply)?.len();
    let connection = connected.connection.expect("current bound connection");
    assert_ne!(connection, old_connection);
    let answer = serde_json::to_vec(&reply)?;
    let wrong_id: Answer = serde_json::from_slice(&answer)?;
    assert!(client.consume_base_declaration_projection(&current, connection, Id(19), wrong_id)?.is_none());
    let wrong_stream: Answer = serde_json::from_slice(&answer)?;
    assert!(client.consume_base_declaration_projection(&current, old_connection, Id(2), wrong_stream)?.is_none());
    let accepted = client.consume_base_declaration_projection(&current, connection, Id(2), reply)?
        .ok_or("fresh projection was not consumed")?;
    assert_eq!(accepted.entry_projection, prior_entry);
    assert_ne!(accepted.preparation, prior_identity);
    assert_ne!(accepted.preparation, proposed);
    let state = client.selection(target)?;
    assert_eq!((state.selected, state.pending), (Some(base), Some(refreshed)));
    connected.send(3, request(client.pending_base_cst_proposal(target, refreshed)?.1)).await?;
    assert!(matches!(connected.receive(3).await?.reply,
        Reply::Refused { reason: Refusal::BaseProjectionAlreadyHeld }));
    let mut unbound = ClientStream::open(&worker, current.clone(), false, LIFETIME).await?;
    unbound.send(2, request(client.pending_base_cst_proposal(target, refreshed)?.1)).await?;
    assert!(matches!(unbound.receive(2).await?.reply, Reply::Refused { reason: Refusal::BaseProjectionUnavailable }));
    unbound.close(3).await?;

    async fn hold(
        worker: &SourceStorageWorker, client: &mut SqliteSourceState, target: &str, attempt: SourceAttempt,
    ) -> Result<wosy_daemon::PreparedBaseDeclarations> {
        let (binding, proposal) = client.pending_base_cst_proposal(target, attempt)?;
        worker.prepare_base_cst_declarations(binding, proposal, LIMITS, 1 << 20, LIFETIME).await
    }
    let mut owners = Vec::new();
    for _ in 0..6 { owners.push(hold(&worker, client, target, refreshed).await?); }
    let cap = hold(&worker, client, target, refreshed).await.err().expect("both framed owners count");
    assert_eq!(cap.to_string(), "live base declaration preparations are at capacity");
    waiting.close(3).await?;
    owners.push(hold(&worker, client, target, refreshed).await?);
    connected.eof().await?;
    owners.push(hold(&worker, client, target, refreshed).await?);
    drop(owners.pop());
    let mut expiring = ClientStream::open(&worker, current.clone(), true, Duration::from_millis(500)).await?;
    expiring.send(2, request(client.pending_base_cst_proposal(target, refreshed)?.1)).await?;
    assert!(matches!(expiring.receive(2).await?.reply, Reply::BaseDeclarationProjection { .. }));
    assert!(tokio::time::timeout(Duration::from_secs(3),
        expiring.reader.read::<_, Answer>(&mut expiring.stream)).await??.is_none());
    assert_eq!(expiring.host.await??, BuildSessionEnd::Expired);
    owners.push(hold(&worker, client, target, refreshed).await?);
    drop(owners);
    assert!(store.snapshot(proposed)?.is_none());
    assert_eq!(store.plan_source_retention(base, &[], &[], 25_000)?, catalog);
    drop(worker);
    Arc::try_unwrap(store).ok().expect("connection-owned preparation released").close().await?;
    let reopened = SourceStore::open(dir, 8 << 20)?;
    assert!(reopened.snapshot(proposed)?.is_none());
    assert_eq!(reopened.plan_source_retention(base, &[], &[], 25_000)?, catalog);
    reopened.close().await?;

    let state = client.selection(target)?;
    assert_eq!((state.selected, state.pending), (Some(base), Some(refreshed)));
    assert_eq!((std::fs::read(metadata_file)?, std::fs::read(nodes_file)?), sqlite_before);
    Ok(serde_json::json!({
        "base": base.hex(), "proposed": proposed.hex(), "old_attempt": attempt.generation(),
        "new_attempt": refreshed.generation(), "stale_refused": true, "scope_refused": true,
        "wrong_connection_and_id_refused": true,
        "single_lease_at_quota": true, "close_eof_expiry_released": true,
        "projection": accepted, "framed_ms": elapsed_ms, "response_bytes": response_bytes,
    }))
}
