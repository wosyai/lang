#![cfg(all(target_os = "linux", feature = "native-diagnostics"))]

use std::{
    fs::{self, DirBuilder, File},
    io::{BufRead, BufReader, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wosy_protocol::native_diagnostics::{connect_owned_child, NativeDiagnosticListener};

fn io_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap()
}

#[test]
#[ignore = "child entry for an explicitly selected native gate"]
fn serving_child() {
    let endpoint = PathBuf::from(std::env::var_os("WOSY_NATIVE_GATE_ENDPOINT").unwrap());
    let expects_frame = std::env::var_os("WOSY_NATIVE_GATE_FRAME").is_some();
    io_runtime().block_on(async { tokio::time::timeout(Duration::from_secs(20), async {
        let listener = NativeDiagnosticListener::bind_new(&endpoint).unwrap();
        println!("OWNED_LISTENER_READY");
        std::io::stdout().flush().unwrap();
        let mut stream = listener.accept().await.unwrap();
        let mut received = Vec::new();
        stream.read_to_end(&mut received).await.unwrap();
        let expected: &[u8] = if expects_frame { b"fixture-frame" } else { b"" };
        assert_eq!(received, expected);
    }).await.expect("native test child deadline"); });
}

struct OwnedChild(Child);

impl OwnedChild {
    fn wait_until(&mut self, deadline: Instant) {
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "owned native child did not exit by deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn launch(executable: &Path, endpoint: &Path, expects_frame: bool, deadline: Instant) -> OwnedChild {
    let mut command = Command::new(executable);
    command.args(["--exact", "serving_child", "--ignored", "--nocapture"])
        .env("WOSY_NATIVE_GATE_ENDPOINT", endpoint)
        .env_remove("WOSY_NATIVE_GATE_FRAME")
        .stdout(Stdio::piped());
    if expects_frame {
        command.env("WOSY_NATIVE_GATE_FRAME", "1");
    }
    let mut child = OwnedChild(command.spawn().unwrap());
    let pipe = child.0.stdout.take().unwrap();
    let (ready, wait_ready) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut pipe = BufReader::new(pipe);
        let mut line = String::new();
        while pipe.read_line(&mut line).ok().is_some_and(|read| read != 0) {
            if line.trim() == "OWNED_LISTENER_READY" {
                let _ = ready.send(());
                break;
            }
            line.clear();
        }
        // Keep draining so a dropped pipe cannot mask child success.
        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
    });
    wait_ready.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .expect("owned native child did not reach readiness by deadline");
    child
}

#[test]
#[ignore = "requires explicitly selected protected Linux socket parent"]
fn verifies_owned_child_and_refuses_explicit_mismatches() {
    let parent = PathBuf::from(std::env::var_os("WOSY_NATIVE_GATE_PARENT").unwrap());
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let work = parent.join(format!("w6p-{}-{nonce:x}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&work).unwrap();
    let scratch = Scratch(work);
    let work = &scratch.0;
    let executable = std::env::current_exe().unwrap();
    let opened = File::open(&executable).unwrap();
    // Deliberately wrong regular file; never executed.
    let image = work.join("wrong-image");
    fs::write(&image, b"not the selected test executable").unwrap();
    let wrong = File::open(&image).unwrap();

    let time_limit = Duration::from_secs(15);
    let deadline = Instant::now() + time_limit;
    io_runtime().block_on(async { tokio::time::timeout(time_limit, async {
        let good_path = work.join("good");
        let wrong_image_path = work.join("image");
        let sibling_path = work.join("sibling");
        let mut good = launch(&executable, &good_path, true, deadline);
        let mut wrong_image = launch(&executable, &wrong_image_path, false, deadline);
        let mut sibling = launch(&executable, &sibling_path, false, deadline);

        let problem = connect_owned_child(&sibling_path, &mut good.0, &opened).await.unwrap_err();
        assert_eq!(problem.to_string(), "local diagnostic peer is not the owned child");
        sibling.wait_until(deadline);

        let problem = connect_owned_child(&wrong_image_path, &mut wrong_image.0, &wrong).await.unwrap_err();
        assert_eq!(problem.to_string(), "local diagnostic child executable does not match the opened image");
        wrong_image.wait_until(deadline);

        let mut stream = connect_owned_child(&good_path, &mut good.0, &opened).await.unwrap();
        stream.write_all(b"fixture-frame").await.unwrap();
        stream.shutdown().await.unwrap();
        drop(stream);
        good.wait_until(deadline);

        let problem = connect_owned_child(&good_path, &mut good.0, &opened).await.unwrap_err();
        assert_eq!(problem.to_string(), "owned local diagnostic child already exited");
    }).await.expect("native owned-child test deadline"); });
    assert!(!work.join("good").exists() && !work.join("image").exists() && !work.join("sibling").exists());
    fs::remove_file(image).unwrap();
}
