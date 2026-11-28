use super::*;
use crate::{source_recovery::ClientSourceLimits, source_state::AcknowledgedOrdinaryLlvm};
use std::{fs, future::{poll_fn, Future}, io, path::PathBuf, task::Poll, time::{SystemTime, UNIX_EPOCH}};
use tokio::io::AsyncWriteExt;
use wosy_compiler::Diagnostic;
use wosy_protocol::{
    encode_session_frame, write_encoded_session_frame, BuildSessionCommand as Command,
    BuildSessionCommandId as Id, BuildSessionRefusal as Refusal, BuildSessionReply as Reply,
    BuildSessionRequest as Request, BuildSessionResponse, SessionFrameError, SessionFrameReader,
    BUILD_RECONCILIATION_VERSION, ORDINARY_LLVM_PROJECTION_VERSION,
};

const LIMITS: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
    records: 10_000, record_bytes: 2 << 20, graph_edges: 10_000, expanded_record_bytes: 2 << 20,
    cst_occurrences: 10_000, cst_depth: 256, authored_bytes: 2 << 20,
};
const WIDTH: usize = 1 << 20;
const BUDGET: Duration = Duration::from_secs(15);
const MANIFEST: &str = r#"
[package]
name = "app"
default_target = "app"
[comments]
categories = []
[builders.unused]
command = ["fixture-must-not-execute"]
toolchain_identity = "projection only"
[runners.unused]
command = ["fixture-must-not-execute"]
[targets.app]
root = "main.w"
[targets.app.artifacts.main]
artifact = "App"
[targets.app.artifacts.main.profiles.dev]
backend = "llvm"
output = "native"
builder = "unused"
run_runner = "unused"
[targets.app.artifacts.main.profiles.verify]
backend = "llvm"
output = "wasm-wasip1"
builder = "unused"
run_runner = "unused"
[targets.app.profiles.dev]
main_artifact = "main"
[targets.app.profiles.verify]
main_artifact = "main"
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("wosy-ordinary-unix-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

async fn command(
    peer: &mut UnixStream, frames: &mut SessionFrameReader, id: u64, request: Request,
) -> Reply<Diagnostic> {
    let id = Id(id);
    let encoded = encode_session_frame(&Command { id, request }).unwrap();
    assert!(write_encoded_session_frame(peer, &encoded, || true).await.unwrap());
    let response = frames.read::<_, BuildSessionResponse<Diagnostic>>(peer).await.unwrap().unwrap();
    assert_eq!(response.id, id);
    response.reply
}

async fn run() {
    let scratch = Scratch::new();
    let project = scratch.0.join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("wosy.toml"), MANIFEST).unwrap();
    fs::write(project.join("main.w"), "%%start\ni32 answer = 42;\n%%end\n").unwrap();
    let mut client = SqliteSourceState::open(&project, ClientSourceLimits { nodes: 10_000, encoded_bytes: 2 << 20 }).unwrap();

    // Establish the ordinary project's actual acknowledgment, then simulate
    // reconnecting to a fresh store that has never seen this graph.
    let original = SourceStore::open(&scratch.0.join("original"), 8 << 20).unwrap();
    assert!(establish_local_build_source(&mut client, &original, "app").await.unwrap());
    original.close().await.unwrap();
    let acknowledged = client.capture_acknowledged_source("app").unwrap().unwrap();
    let store = Arc::new(SourceStore::open(&scratch.0.join("reopened"), 8 << 20).unwrap());
    let one = NonZeroUsize::MIN;
    let worker = Arc::new(SourceStorageWorker::new(
        Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one,
    ).unwrap());
    let mut jobs = JoinSet::new();
    let deadline = Instant::now() + BUDGET;
    let mut expired = open_scoped_session(&worker, &mut jobs, acknowledged.selection().clone(), None,
        "dev", LIMITS, WIDTH, Instant::now()).unwrap();
    assert!(SessionFrameReader::default().read::<_, BuildSessionResponse<Diagnostic>>(&mut expired).await.unwrap().is_none());
    assert_eq!(jobs.join_next().await.unwrap().unwrap().unwrap(), BuildSessionEnd::Expired);

    let mut bad = open_scoped_session(&worker, &mut jobs, acknowledged.selection().clone(), None,
        "dev", LIMITS, WIDTH, deadline).unwrap();
    bad.write_all(&[0, 0, 0, 12, b'{']).await.unwrap();
    bad.shutdown().await.unwrap();
    let truncated = jobs.join_next().await.unwrap().unwrap().unwrap_err();
    assert!(matches!(truncated.downcast_ref::<SessionFrameError>(),
        Some(SessionFrameError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof));
    assert!(store.snapshot(acknowledged.selection().root).unwrap().is_none());

    let mut opens = Vec::new();
    let result = client.live_ordinary_llvm("app", "dev", BUDGET, LIMITS.authored_bytes, WIDTH,
        |selected, replay| {
            assert_eq!(&selected, acknowledged.selection());
            opens.push(replay.is_some());
            std::future::ready(open_scoped_session(&worker, &mut jobs, selected, replay,
                "dev", LIMITS, WIDTH, deadline))
        }).await.unwrap();
    let AcknowledgedOrdinaryLlvm::Partition(plan) = result else { panic!("missing scoped LLVM"); };
    assert_eq!(opens, [false, true]);
    assert_eq!(plan.source().generation(), acknowledged.generation());
    assert_eq!(plan.projection().profile, "dev");
    assert_eq!(plan.projection().source_root, acknowledged.selection().root);
    assert_eq!(plan.projection().compiler_image,
        wosy_protocol::compiler_image_digest(open_running_image().unwrap()).unwrap());
    let mut ends = Vec::new();
    while let Some(end) = jobs.join_next().await { ends.push(end.unwrap().unwrap()); }
    assert_eq!(ends.len(), 2);
    assert!(ends.contains(&BuildSessionEnd::ReplayRequired));
    assert!(ends.contains(&BuildSessionEnd::Disconnected));

    let mut peer = open_scoped_session(&worker, &mut jobs, acknowledged.selection().clone(), None,
        "dev", LIMITS, WIDTH, deadline).unwrap();
    let mut frames = SessionFrameReader::default();
    assert!(matches!(command(&mut peer, &mut frames, 11,
        Request::StartRecoverable { version: BUILD_RECONCILIATION_VERSION }).await,
        Reply::RecoverableStarted { source_root, .. } if source_root == acknowledged.selection().root));
    let project_ir = |version, profile: &str| Request::ProjectOrdinaryLlvm { version, profile: profile.into() };
    assert!(matches!(command(&mut peer, &mut frames, 12, project_ir(u16::MAX, "dev")).await,
        Reply::Refused { reason: Refusal::UnsupportedVersion }));
    assert!(matches!(command(&mut peer, &mut frames, 13, project_ir(ORDINARY_LLVM_PROJECTION_VERSION, "verify")).await,
        Reply::Refused { reason: Refusal::OrdinaryLlvmUnavailable }));
    assert!(matches!(command(&mut peer, &mut frames, 14, project_ir(ORDINARY_LLVM_PROJECTION_VERSION, "dev")).await,
        Reply::OrdinaryLlvmProjection { projection } if &projection == plan.projection()));
    assert!(matches!(command(&mut peer, &mut frames, 15, project_ir(ORDINARY_LLVM_PROJECTION_VERSION, "dev")).await,
        Reply::Refused { reason: Refusal::OrdinaryLlvmUnavailable }));
    assert!(matches!(command(&mut peer, &mut frames, 16, Request::Disconnect).await, Reply::Closed));
    assert!(frames.read::<_, BuildSessionResponse<Diagnostic>>(&mut peer).await.unwrap().is_none());
    assert_eq!(jobs.join_next().await.unwrap().unwrap().unwrap(), BuildSessionEnd::Disconnected);
    // Pause only the server. The actual client has issued Start for A when
    // the writer publishes B and then identical A. A complete valid reply
    // to that historical request still cannot cross the generation boundary.
    let (client_end, server_end) = UnixStream::pair().unwrap();
    let mut once = Some(client_end);
    let mut request = Box::pin(client.live_ordinary_llvm("app", "dev", BUDGET, LIMITS.authored_bytes, WIDTH,
        |selection, replay| {
            assert_eq!(&selection, acknowledged.selection());
            assert!(replay.is_none());
            std::future::ready(Ok(once.take().expect("single existing native connection")))
        }));
    assert!(poll_fn(|context| Poll::Ready(request.as_mut().poll(context).is_pending())).await);
    let entry = project.join("main.w");
    let previous = fs::read(&entry).unwrap();
    let mut writer = SqliteSourceState::open(&project, ClientSourceLimits {
        nodes: 10_000, encoded_bytes: 2 << 20,
    }).unwrap();
    fs::write(&entry, "%%start\ni32 answer = 71;\n%%end\n").unwrap();
    assert!(establish_local_build_source(&mut writer, &store, "app").await.unwrap());
    assert_ne!(writer.capture_acknowledged_source("app").unwrap().unwrap().selection(), acknowledged.selection());
    fs::write(&entry, previous).unwrap();
    assert!(establish_local_build_source(&mut writer, &store, "app").await.unwrap());
    let current = writer.capture_acknowledged_source("app").unwrap().unwrap();
    assert_eq!(current.selection(), acknowledged.selection());
    assert!(current.generation() > acknowledged.generation());
    let (closed, result) = tokio::join!(worker.serve_scoped_ordinary_llvm_session(
        server_end, acknowledged.selection().clone(), None, "dev".into(), LIMITS, WIDTH, deadline.into_std(),
    ), request.as_mut());
    assert_eq!(closed.unwrap(), BuildSessionEnd::Disconnected);
    assert!(matches!(result.unwrap(), AcknowledgedOrdinaryLlvm::StaleSelection));
    drop(request);
    assert!(!project.join(".wosy/artifacts").exists());
    drop(worker);
    Arc::into_inner(store).unwrap_or_else(|| panic!("socket worker retained the store")).close().await.unwrap();
}

#[test]
fn unix_stream_frame_refusals_and_replay_are_bound_to_the_selected_project() {
    tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap()
        .block_on(async { timeout_at(Instant::now() + BUDGET, run()).await.unwrap() });
}

mod process;
