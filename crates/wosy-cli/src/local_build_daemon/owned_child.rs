//! Private one-shot Linux child for ordinary authored compiler comparison.
//! No path-based socket, service, shell, external builder, or alternate
//! executable can be selected by this transport.
use super::{open_running_image, Result};
use crate::{artifact_inputs::PendingBuilderInputs, source_state::{
    AcknowledgedSourceGeneration, AuthorizedOrdinaryBuilderPeer, SourceReplay, SqliteSourceState,
}};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    env, fs::{self, File, Metadata},
    num::NonZeroUsize,
    os::fd::{AsFd, OwnedFd},
    os::unix::{fs::{DirBuilderExt, MetadataExt}, net::UnixStream as StdUnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus, Stdio},
    rc::Rc, sync::Arc, time::Duration,
};
use tokio::{io::AsyncWriteExt, net::UnixStream, time::{timeout_at, Instant}};
use wosy_daemon::{BaseCstAdmissionLimits, BuildSessionEnd, SourceStorageWorker, SourceStore};
use wosy_protocol::{
    encode_session_frame, write_encoded_session_frame, BuildSourceSelection, NodeId,
    SessionFrameReader, MAX_SESSION_FRAME_BYTES,
};

const CHILD_ARG: &str = "__wosy-linked-ordinary-child-v1";
const VERSION: u16 = 1;
const LIFETIME: Duration = Duration::from_secs(120);
const REPLAY: u8 = 75;
const INELIGIBLE: u8 = 76;
const LIMITS: BaseCstAdmissionLimits = BaseCstAdmissionLimits {
    records: usize::MAX, record_bytes: usize::MAX, graph_edges: usize::MAX,
    expanded_record_bytes: usize::MAX, cst_occurrences: usize::MAX, cst_depth: usize::MAX,
    authored_bytes: usize::MAX,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    version: u16,
    root: NodeId,
    target: String,
    profile: String,
    remaining: Duration,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    version: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome { Eligible, Replay, Ineligible }

struct OwnedChild(Child);

impl OwnedChild {
    async fn finish(mut self, deadline: Instant) -> Result<Outcome> {
        timeout_at(deadline, async {
            loop {
                if let Some(status) = self.0.try_wait()? { return outcome(status); }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await?
    }
}

fn outcome(status: ExitStatus) -> Result<Outcome> {
    match status.code() {
        Some(0) => Ok(Outcome::Eligible),
        Some(code) if code == i32::from(REPLAY) => Ok(Outcome::Replay),
        Some(code) if code == i32::from(INELIGIBLE) => Ok(Outcome::Ineligible),
        _ => Err(format!("linked ordinary compiler child exited with {status}").into()),
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn same_image(left: &Metadata, right: &Metadata) -> bool {
    left.is_file() && right.is_file() && (left.dev(), left.ino()) == (right.dev(), right.ino())
}

/// One cooperative project source-store reservation, independent of any
/// selected target, profile or artifact writer. Its Linux open-file lock
/// travels with a duplicate of the same directory descriptor to the child.
pub struct SourceStoreCustody {
    project: PathBuf,
    owner: File,
}

impl SourceStoreCustody {
    pub fn try_acquire(project: &Path) -> Result<Option<Self>> {
        let project = project.canonicalize()?;
        let directory = project.join(".wosy");
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let owner = open_project_owner(&directory)?;
        match owner.try_lock() {
            Ok(()) => Ok(Some(Self { project, owner })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }

    fn verify_path(&self) -> Result<()> {
        let current = open_project_owner(&self.project.join(".wosy"))?;
        let current = current.metadata()?;
        let held = self.owner.metadata()?;
        if !held.is_dir() || (held.dev(), held.ino()) != (current.dev(), current.ino()) {
            return Err("linked ordinary compiler lost project source-store custody".into());
        }
        Ok(())
    }

    pub fn store_path(&self) -> PathBuf { self.project.join(".wosy/incremental-daemon") }

    pub fn open_store(&self) -> Result<SourceStore> {
        self.verify_path()?;
        let path = self.store_path();
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o777 != 0o700
        {
            return Err("linked ordinary compiler requires a project-owned source store".into());
        }
        SourceStore::open(&path, 8 << 20)
    }
}

// No lock-file names that can be replaced or accidentally removed during
// source-store recovery. Build and its child cooperate on the project owner;
// this is not protection from same-account tools replacing their workspace.
fn open_project_owner(path: &Path) -> Result<File> {
    use rustix::fs::{open, Mode, OFlags};
    let file = File::from(open(path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty())?);
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err("linked ordinary compiler requires a project-owned source store".into());
    }
    Ok(file)
}

async fn send<W: tokio::io::AsyncWrite + Unpin, T: Serialize>(output: &mut W, message: &T, deadline: Instant) -> Result<()> {
    let bytes = encode_session_frame(message)?;
    if write_encoded_session_frame(output, &bytes, || Instant::now() < deadline).await? { Ok(()) }
    else { Err("linked ordinary compiler activation expired".into()) }
}

async fn boot(
    image: &File, custody: &SourceStoreCustody, selected: BuildSourceSelection, profile: &str, deadline: Instant,
) -> Result<(OwnedChild, UnixStream)> {
    if Path::new(&selected.project) != custody.project { return Err("linked ordinary compiler selected a different project".into()); }
    let (local, peer) = StdUnixStream::pair()?;
    local.set_nonblocking(true)?;
    let mut local = UnixStream::from_std(local)?;
    let image_meta = image.metadata()?;
    let child = OwnedChild(Command::new("/proc/self/exe").arg(CHILD_ARG).current_dir(&selected.project)
        .stdin(Stdio::from(OwnedFd::from(peer)))
        .stdout(Stdio::from(custody.owner.try_clone()?)).spawn()?);
    let pid = child.0.id();
    if pid == std::process::id() || !same_image(&File::open(format!("/proc/{pid}/exe"))?.metadata()?, &image_meta) {
        return Err("linked ordinary compiler child is not the selected executable".into());
    }
    // SO_PEERCRED for socketpair records its creator. Parent authority is
    // the fresh endpoint handed only to this owned, unreaped child.
    let activation = Start { version: VERSION, root: selected.root, target: selected.target,
        profile: profile.into(), remaining: deadline.saturating_duration_since(Instant::now()) };
    if activation.remaining.is_zero() { return Err("linked ordinary compiler activation expired".into()); }
    send(&mut local, &activation, deadline).await?;
    let ready = SessionFrameReader::default().read::<_, Ready>(&mut local).await?;
    if !ready.is_some_and(|ready| ready.version == VERSION) {
        return Err("linked ordinary compiler did not acknowledge its private activation".into());
    }
    Ok((child, local))
}

/// The parent has closed its store before activation. A recoverable source
/// challenge discards that child and stream. Replaying the exact acknowledged
/// graph uses the existing local store handoff, then opens another child.
pub async fn compare_owned_child(
    source: &mut SqliteSourceState, custody: &SourceStoreCustody, target: &str, profile: &str, ordinary: &PendingBuilderInputs,
) -> Result<Option<(AcknowledgedSourceGeneration, bool)>> {
    let deadline = Instant::now().checked_add(LIFETIME).ok_or("linked ordinary compiler deadline overflow")?;
    timeout_at(deadline, async {
        let image = Rc::new(open_running_image()?);
        let expected = AuthorizedOrdinaryBuilderPeer::from_open_executable(image.try_clone()?)?;
        let active = Rc::new(RefCell::new(None::<OwnedChild>));
        let mut replayed = false;
        let answer = source.compare_selected_ordinary_llvm(
            target, profile, LIFETIME, LIMITS.authored_bytes, MAX_SESSION_FRAME_BYTES, &expected, ordinary,
            |selected, replay: Option<SourceReplay>| {
                replayed |= replay.is_some();
                let active = Rc::clone(&active);
                let image = Rc::clone(&image);
                async move {
                    let previous = active.borrow_mut().take();
                    match (previous, replay) {
                        (None, None) => {}
                        (Some(previous), Some(replay)) if replay.source.root == selected.root => {
                            if previous.finish(deadline).await? != Outcome::Replay {
                                return Err("linked ordinary compiler replay was not requested".into());
                            }
                            if Path::new(&selected.project) != custody.project {
                                return Err("linked ordinary compiler replay selected a different project".into());
                            }
                            let store = custody.open_store()?;
                            let written = store.publish(selected.root, &replay.source.delta).await;
                            store.close().await?;
                            written?;
                        }
                        _ => return Err("invalid linked ordinary compiler replay transition".into()),
                    }
                    let (child, local) = boot(&image, custody, selected, profile, deadline).await?;
                    *active.borrow_mut() = Some(child);
                    Ok(local)
                }
            },
        ).await?;
        let child = active.borrow_mut().take();
        let status = match child {
            Some(child) => Some(child.finish(deadline).await?),
            None => None,
        };
        // The frame verified the exact image/root/profile/partition and the
        // client generation. The owned child's terminal outcome contributes
        // only the separate positive all-source no-comptime observation.
        Ok(if status == Some(Outcome::Eligible) { answer.map(|scope| (scope, replayed)) } else { None })
    }).await?
}

fn parent_pid() -> Option<u32> {
    rustix::process::getppid().and_then(|pid| u32::try_from(pid.as_raw_pid()).ok())
}

fn authenticate_parent(stream: &UnixStream, parent: u32) -> Result<()> {
    let peer = stream.peer_cred()?;
    if parent_pid() != Some(parent) || peer.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(parent)
        || peer.uid() != rustix::process::geteuid().as_raw()
        || !same_image(&File::open(format!("/proc/{parent}/exe"))?.metadata()?, &open_running_image()?.metadata()?)
    {
        return Err("linked ordinary compiler activation is not from its same-image parent".into());
    }
    Ok(())
}

async fn child_main(socket: StdUnixStream, reservation: File) -> Result<Outcome> {
    let mut socket = UnixStream::from_std(socket)?;
    let parent = parent_pid().ok_or("linked ordinary compiler has no direct parent")?;
    authenticate_parent(&socket, parent)?;
    let outer = Instant::now().checked_add(LIFETIME).ok_or("linked ordinary compiler deadline overflow")?;
    let activation = timeout_at(outer, SessionFrameReader::default().read::<_, Start>(&mut socket)).await??
        .ok_or("linked ordinary compiler activation is missing")?;
    if activation.version != VERSION || activation.target.is_empty() || activation.remaining > LIFETIME {
        return Err("invalid linked ordinary compiler activation".into());
    }
    if parent_pid() != Some(parent) { return Err("linked ordinary compiler lost its owner".into()); }
    let cutoff = outer.min(Instant::now().checked_add(activation.remaining).ok_or("linked ordinary compiler deadline overflow")?);
    timeout_at(cutoff, async {
        let project = env::current_dir()?.canonicalize()?;
        let selected = BuildSourceSelection { root: activation.root, target: activation.target,
            project: project.to_str().ok_or("linked ordinary compiler project is not UTF-8")?.into() };
        let root = selected.root;
        // stdout is reserved for the duplicated owner descriptor; no
        // protocol bytes or compiler output are sent on that handle.
        let custody = SourceStoreCustody { project, owner: reservation };
        let store = Arc::new(custody.open_store()?);
        let one = NonZeroUsize::MIN;
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one)?;
        send(&mut socket, &Ready { version: VERSION }, cutoff).await?;
        let ended = worker.serve_scoped_ordinary_llvm_session(
            &mut socket, selected, None, activation.profile, LIMITS, MAX_SESSION_FRAME_BYTES, cutoff.into_std(),
        ).await;
        // The inherited stdin descriptor still owns a reference to this
        // socket. Explicitly issue EOF before closing store and exiting.
        let half_closed = socket.shutdown().await;
        let ended = ended?;
        if ended != BuildSessionEnd::PeerClosed { half_closed?; }
        let result = match ended {
            BuildSessionEnd::ReplayRequired => Outcome::Replay,
            BuildSessionEnd::Disconnected => {
                match store.snapshot(root)? {
                    Some(view) if store.ordinary_view_has_no_comptime(&view)? => Outcome::Eligible,
                    _ => Outcome::Ineligible,
                }
            }
            BuildSessionEnd::Expired | BuildSessionEnd::PeerClosed | BuildSessionEnd::Refused => Outcome::Ineligible,
        };
        drop(worker);
        Arc::into_inner(store).ok_or("linked ordinary compiler worker retained source store")?.close().await?;
        drop(custody);
        Ok(result)
    }).await?
}

/// Hidden trampoline, not a daemon command. A script or normal invocation
/// cannot enter without the inherited connected socket and same-image parent.
pub fn enter_if_requested() -> Option<ExitCode> {
    if env::var_os("WOSY_INCREMENTAL").as_deref() != Some(std::ffi::OsStr::new("1")) { return None; }
    let mut args = env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(CHILD_ARG)) || args.next().is_some() { return None; }
    let run = (|| -> Result<Outcome> {
        let socket = StdUnixStream::from(std::io::stdin().as_fd().try_clone_to_owned()?);
        socket.set_nonblocking(true)?;
        let owner = File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
        tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?.block_on(child_main(socket, owner))
    })();
    Some(match run {
        Ok(Outcome::Eligible) => ExitCode::SUCCESS,
        Ok(Outcome::Replay) => ExitCode::from(REPLAY),
        Ok(Outcome::Ineligible) => ExitCode::from(INELIGIBLE),
        Err(error) => {
            eprintln!("linked ordinary compiler child: {error}");
            ExitCode::FAILURE
        }
    })
}
