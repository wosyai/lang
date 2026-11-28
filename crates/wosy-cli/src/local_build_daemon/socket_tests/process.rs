//! Authentic process boundary for the existing scoped authored-LLVM comparison.
//! No product listener or public child entry is installed by this fixture.
use super::*;
use crate::artifact_inputs::{pending_builder_inputs, project_configuration_identity};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    os::fd::{AsFd, OwnedFd},
    os::unix::{fs::{DirBuilderExt, MetadataExt, OpenOptionsExt}, net::UnixStream as StdUnixStream},
    path::Path,
    process::{Child, Command, Stdio},
    rc::Rc,
};
use wosy_project::{artifact_manifest_path, load_artifact_manifest, ProjectConfiguration};
use wosy_protocol::{Delta, NodeId};

const CHILD: &str = "local_build_daemon::socket_tests::process::comparison_child";
const LAUNCH: &str = "WOSY_SCOPED_COMPARISON_TEST_ACTIVATION";
const CLI: &str = "WOSY_SCOPED_COMPARISON_TEST_CLI";
const CLI_HASH: &str = "WOSY_SCOPED_COMPARISON_TEST_CLI_SHA256";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Activation {
    parent: u32,
    store: PathBuf,
    root: NodeId,
    project: String,
    target: String,
    profile: String,
    replay: Option<Vec<(NodeId, Vec<u8>)>>,
    remaining: Duration,
    terminal: ExpectedEnd,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum ExpectedEnd { ReplayRequired, Disconnected, PeerClosed, Expired }

impl ExpectedEnd {
    fn session(self) -> BuildSessionEnd {
        match self {
            Self::ReplayRequired => BuildSessionEnd::ReplayRequired,
            Self::Disconnected => BuildSessionEnd::Disconnected,
            Self::PeerClosed => BuildSessionEnd::PeerClosed,
            Self::Expired => BuildSessionEnd::Expired,
        }
    }
}

struct OwnedChild {
    child: Child,
    activation: Option<PathBuf>,
}

impl OwnedChild {
    async fn wait(mut self, deadline: Instant) -> Result<()> {
        timeout_at(deadline, async {
            loop {
                if let Some(status) = self.child.try_wait()? {
                    return if status.success() { Ok(()) }
                    else { Err(format!("scoped comparison child exited with {status}").into()) };
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await?
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // One owner reaps the exact child on all paths, including cancellation.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(path) = &self.activation { let _ = fs::remove_file(path); }
    }
}

fn boot(path: &Path, setup: &Activation) -> Result<(OwnedChild, UnixStream)> {
    let running_image = open_running_image()?;
    let expected_image = running_image.metadata()?;
    let (local, daemon) = StdUnixStream::pair()?;
    local.set_nonblocking(true)?;
    let local = UnixStream::from_std(local)?;
    let request = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    let launch = (|| -> Result<_> {
        serde_json::to_writer(request, setup)?;
        let child = Command::new("/proc/self/exe")
            .args(["--exact", CHILD, "--ignored", "--nocapture"])
            .env(LAUNCH, path)
            .stdin(Stdio::from(OwnedFd::from(daemon)))
            .stdout(Stdio::null())
            .spawn()?;
        let owned = OwnedChild { child, activation: Some(path.to_owned()) };
        let pid = owned.child.id();
        let actual_image = File::open(format!("/proc/{pid}/exe"))?.metadata()?;
        if pid == std::process::id() || !actual_image.is_file()
            || (actual_image.dev(), actual_image.ino()) != (expected_image.dev(), expected_image.ino())
        {
            return Err("scoped comparison child does not own the chosen compiler image".into());
        }
        // SO_PEERCRED for socketpair is stamped at creation, not a child PID
        // assertion. Authority comes from the fresh fd handed exclusively
        // to the unreaped child running the checked image above.
        Ok((owned, local))
    })();
    if launch.is_err() { let _ = fs::remove_file(path); }
    launch
}

#[test]
#[ignore = "owned child entry for the explicit cross-process fixture"]
fn comparison_child() {
    let activation = std::env::var_os(LAUNCH).expect("missing fixture activation");
    let setup: Activation = serde_json::from_reader(File::open(activation).unwrap()).unwrap();
    let cutoff = std::time::Instant::now().checked_add(setup.remaining).unwrap();
    let socket = StdUnixStream::from(std::io::stdin().as_fd().try_clone_to_owned().unwrap());
    socket.set_nonblocking(true).unwrap();
    tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap().block_on(async {
        let mut socket = UnixStream::from_std(socket).unwrap();
        let peer = socket.peer_cred().unwrap().pid().and_then(|id| u32::try_from(id).ok());
        assert_eq!(peer, Some(setup.parent));
        assert_ne!(peer, Some(std::process::id()));
        let store = Arc::new(SourceStore::open(&setup.store, 8 << 20).unwrap());
        let one = NonZeroUsize::MIN;
        let worker = SourceStorageWorker::new(Arc::clone(&store), tokio::runtime::Handle::current(), one, one, one).unwrap();
        let selected = BuildSourceSelection { root: setup.root, project: setup.project, target: setup.target };
        let replay = setup.replay.map(|nodes| Delta { nodes: nodes.into_iter().collect() });
        let ended = worker.serve_scoped_ordinary_llvm_session(
            &mut socket, selected, replay, setup.profile, LIMITS, WIDTH, cutoff,
        ).await;
        // stdin still names this same kernel socket; closing just our dup
        // would not signal EOF. Explicitly finish the write side even on
        // error, before closing our SourceStore and exiting.
        let half_closed = socket.shutdown().await;
        let ended = ended.unwrap();
        if ended != BuildSessionEnd::PeerClosed { half_closed.unwrap(); }
        assert_eq!(ended, setup.terminal.session());
        drop(worker);
        Arc::into_inner(store).unwrap_or_else(|| panic!("child retained the source store")).close().await.unwrap();
    });
}

struct Fixture {
    private: PathBuf,
    project: PathBuf,
    cli: PathBuf,
    acknowledged_store: PathBuf,
    remote: PathBuf,
    child_count: usize,
}

impl Fixture {
    fn new(scratch: &Path) -> Self {
        let cli = PathBuf::from(std::env::var_os(CLI).expect("provide the verified ordinary release CLI"));
        assert!(cli.is_absolute());
        let expected = std::env::var(CLI_HASH).expect("provide the verified SHA256 of the ordinary release CLI");
        let digest = Command::new("sha256sum").arg(&cli).output().unwrap();
        assert!(digest.status.success());
        assert_eq!(String::from_utf8(digest.stdout).unwrap().split_whitespace().next(), Some(expected.as_str()));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/build/short_circuit_project/project");
        let private = scratch.join("private");
        fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
        let project = private.join("project");
        fs::create_dir(&project).unwrap();
        for folder in ["src", "scripts"] { fs::create_dir(project.join(folder)).unwrap(); }
        for file in ["wosy.toml", "src/main.w", "src/child.w", "scripts/build-native", "scripts/run-native"] {
            fs::copy(source.join(file), project.join(file)).unwrap();
        }
        let manifest = project.join("wosy.toml");
        fs::write(&manifest, fs::read_to_string(&manifest).unwrap() + r#"
[targets.app.artifacts.main.profiles.verify]
backend = "llvm"
output = "native"
builder = "native"
run_runner = "native"
[targets.app.profiles.verify]
main_artifact = "main"
"#).unwrap();
        let builder = project.join("scripts/build-native");
        let script = fs::read_to_string(&builder).unwrap().replace(
            "partition, output_dir", "with open('builder-calls', 'a') as f: f.write('called\\n')\npartition, output_dir",
        );
        fs::write(builder, script).unwrap();
        Self { project, cli, acknowledged_store: private.join("acknowledged"),
            remote: private.join("daemon"), private, child_count: 0 }
    }

    async fn cli(&self, command: &str, profile: &str, incremental: bool) {
        let mut cmd = Command::new(&self.cli);
        cmd.current_dir(&self.project).args([command, "app", "--profile", profile]).env_remove("WOSY_INCREMENTAL");
        if incremental { cmd.env("WOSY_INCREMENTAL", "1"); }
        let child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).spawn().unwrap();
        let owned = OwnedChild { child, activation: None };
        owned.wait(Instant::now() + BUDGET).await
            .unwrap_or_else(|error| panic!("{command} {profile}: {error}"));
    }

    async fn prepare_ordinary(&self, profile: &str) -> PendingBuilderInputs {
        self.cli("build", profile, false).await;
        self.cli("run", profile, false).await;
        let path = artifact_manifest_path(&self.project, "app", profile, "main");
        let manifest = load_artifact_manifest(&path).unwrap();
        let output = path.parent().unwrap();
        let partition = fs::read(output.join("partition.ll")).unwrap();
        let partition = blake3::hash(&partition).to_hex().to_string();
        assert_eq!(manifest.identity.llvm_input_identity, partition);
        let (config, doc) = ProjectConfiguration::load_with_manifest(&self.project).unwrap();
        let (_, artifact, selected, _) = config.artifact_profile("app", profile).unwrap();
        let ordinary = pending_builder_inputs(&config, project_configuration_identity(doc.raw().as_bytes(), &config).unwrap(),
            "app", profile, artifact, selected, manifest.source_observations.clone(), partition, output).unwrap();
        assert_eq!(ordinary.manifest, manifest);
        ordinary
    }

    async fn publish_source(&self, source: &mut SqliteSourceState) -> AcknowledgedSourceGeneration {
        let store = SourceStore::open(&self.acknowledged_store, 8 << 20).unwrap();
        assert!(establish_local_build_source(source, &store, "app").await.unwrap());
        store.close().await.unwrap();
        source.capture_acknowledged_source("app").unwrap().unwrap()
    }

    async fn compare(
        &mut self, source: &mut SqliteSourceState, expected: &AcknowledgedSourceGeneration,
        profile: &str, ordinary: &PendingBuilderInputs, expect_replay: bool,
    ) {
        let before_builder = fs::read(self.project.join("builder-calls")).unwrap();
        let parent = std::process::id();
        let deadline = Instant::now() + BUDGET;
        let expected_peer = AuthorizedOrdinaryBuilderPeer::from_open_executable(open_running_image().unwrap()).unwrap();
        let active = Rc::new(RefCell::new(None::<OwnedChild>));
        let mut opened = Vec::new();
        let result = source.compare_selected_ordinary_llvm(
            "app", profile, BUDGET, LIMITS.authored_bytes, WIDTH, &expected_peer, ordinary,
            |selected, replay| {
                assert_eq!(&selected, expected.selection());
                let replayed = replay.is_some();
                opened.push(replayed);
                self.child_count += 1;
                let path = self.private.join(format!("activation-{}", self.child_count));
                let terminal = if expect_replay && !replayed { ExpectedEnd::ReplayRequired } else { ExpectedEnd::Disconnected };
                let mut setup = Activation { parent, store: self.remote.clone(),
                    root: selected.root, project: selected.project, target: selected.target, profile: profile.into(),
                    replay: replay.map(|resume| resume.source.delta.nodes.into_iter().collect()),
                    remaining: Duration::ZERO, terminal };
                let active = Rc::clone(&active);
                async move {
                    let previous = active.borrow_mut().take();
                    if let Some(previous) = previous { previous.wait(deadline).await?; }
                    setup.remaining = deadline.saturating_duration_since(Instant::now());
                    if setup.remaining.is_zero() { return Err("scope expired before opening child".into()); }
                    let (owner, local) = boot(&path, &setup)?;
                    *active.borrow_mut() = Some(owner);
                    Ok(local)
                }
            },
        ).await;
        let child = active.borrow_mut().take().expect("activated linked peer in another process");
        child.wait(deadline).await.unwrap();
        let compared = result.unwrap().expect("exact cross-process ordinary LLVM comparison");
        assert_eq!(compared.selection(), expected.selection());
        assert_eq!(compared.generation(), expected.generation());
        if expect_replay { assert_eq!(opened, [false, true]); }
        else { assert_eq!(opened, [false]); }
        assert_eq!(fs::read(self.project.join("builder-calls")).unwrap(), before_builder);
    }
}

async fn run_project_across_processes() {
    let scratch = Scratch::new();
    let mut fixture = Fixture::new(&scratch.0);
    let mut source = SqliteSourceState::open(&fixture.project, ClientSourceLimits {
        nodes: 10_000, encoded_bytes: 2 << 20,
    }).unwrap();
    let entry = fixture.project.join("src/child.w");
    let original = fs::read_to_string(&entry).unwrap();

    let dev = fixture.prepare_ordinary("dev").await;
    let first = fixture.publish_source(&mut source).await;
    fixture.compare(&mut source, &first, "dev", &dev, true).await;
    fixture.compare(&mut source, &first, "dev", &dev, false).await;

    let verify = fixture.prepare_ordinary("verify").await;
    fixture.compare(&mut source, &first, "verify", &verify, false).await;

    let image = AuthorizedOrdinaryBuilderPeer::from_open_executable(open_running_image().unwrap()).unwrap();
    let wrong_profile = source.compare_selected_ordinary_llvm(
        "app", "dev", BUDGET, LIMITS.authored_bytes, WIDTH, &image, &verify,
        |_, _| -> std::future::Ready<Result<UnixStream>> { panic!("wrongly scoped builder contacted a peer") },
    ).await.unwrap();
    assert!(wrong_profile.is_none());

    fixture.cli("build", "dev", true).await;
    fixture.cli("run", "dev", true).await;
    let pointer = artifact_manifest_path(&fixture.project, "app", "dev", "main")
        .parent().unwrap().join("incremental/artifact-manifest.json");
    let published = load_artifact_manifest(&pointer).unwrap();
    assert_eq!(published.identity, dev.manifest.identity);
    let old_binary = fs::read(&published.executable).unwrap();

    let edited = original.replace("calls + 1", "calls + 2");
    assert_ne!(edited, original);
    fs::write(&entry, edited).unwrap();
    let dev_new = fixture.prepare_ordinary("dev").await;
    let second = fixture.publish_source(&mut source).await;
    assert_ne!(first.selection().root, second.selection().root);
    assert!(second.generation() > first.generation());
    assert_ne!(dev.manifest.identity.llvm_input_identity, dev_new.manifest.identity.llvm_input_identity);
    fixture.compare(&mut source, &second, "dev", &dev_new, true).await;

    // The cross-process comparison does not itself publish any artifact.
    assert_eq!(load_artifact_manifest(&pointer).unwrap(), published);
    fixture.cli("build", "dev", true).await;
    fixture.cli("run", "dev", true).await;
    let changed = load_artifact_manifest(&pointer).unwrap();
    assert_ne!(changed.executable, published.executable);
    assert_eq!(changed.identity, dev_new.manifest.identity);
    assert_eq!(fs::read(&published.executable).unwrap(), old_binary);

    fs::write(&entry, original).unwrap();
    let dev_reverted = fixture.prepare_ordinary("dev").await;
    let third = fixture.publish_source(&mut source).await;
    assert_eq!(first.selection(), third.selection());
    assert!(third.generation() > second.generation());
    assert_eq!(dev.manifest.identity.llvm_input_identity, dev_reverted.manifest.identity.llvm_input_identity);
    fixture.compare(&mut source, &third, "dev", &dev_reverted, false).await;

    let deadline = Instant::now() + BUDGET;
    let setup = |terminal, remaining| Activation {
        parent: std::process::id(), store: fixture.remote.clone(), root: third.selection().root,
        project: third.selection().project.clone(), target: "app".into(), profile: "dev".into(),
        replay: None, remaining, terminal,
    };
    let (disconnect, peer) = boot(&fixture.private.join("disconnect"),
        &setup(ExpectedEnd::PeerClosed, BUDGET)).unwrap();
    drop(peer);
    // The exact peer-closed terminal state is checked inside the child.
    disconnect.wait(deadline).await.unwrap();

    let (expiry, mut peer) = boot(&fixture.private.join("expiry"),
        &setup(ExpectedEnd::Expired, Duration::ZERO)).unwrap();
    assert!(SessionFrameReader::default().read::<_, BuildSessionResponse<Diagnostic>>(&mut peer).await.unwrap().is_none());
    expiry.wait(deadline).await.unwrap();
    assert_eq!(fixture.child_count + 2, 9);
    assert_eq!(fs::read_to_string(fixture.project.join("builder-calls")).unwrap().lines().count(), 6);
    assert!(!fs::read_dir(&fixture.private).unwrap().any(|item| {
        let name = item.unwrap().file_name();
        name.to_string_lossy().starts_with("activation-") || name == "disconnect" || name == "expiry"
    }));
    println!("owned compiler children=9, scoped comparisons=5, independent real builders=6, generations={:?}",
        [first.generation(), second.generation(), third.generation()]);
}

#[test]
#[ignore = "requires an exact verified local release wosy and its ordinary Clang toolchain"]
fn compiler_comparison_survives_owned_process_reopen_and_current_source_changes() {
    let limit = Duration::from_secs(90);
    tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap()
        .block_on(async { timeout_at(Instant::now() + limit, run_project_across_processes()).await.unwrap() });
}
