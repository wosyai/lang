#![cfg(all(target_os = "linux", feature = "native-diagnostics"))]

use std::{
    fs::{self, DirBuilder, File},
    io::{BufRead, BufReader, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wosy_protocol::native_diagnostics::{connect_owned_child, NativeDiagnosticListener};

fn io_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap()
}

#[test]
#[ignore = "child entry for an explicitly selected native gate"]
fn serving_child() {
    let endpoint = PathBuf::from(std::env::var_os("WOSY_NATIVE_GATE_ENDPOINT").unwrap());
    let expects_frame = std::env::var_os("WOSY_NATIVE_GATE_FRAME").is_some();
    io_runtime().block_on(async {
        let listener = NativeDiagnosticListener::bind_new(&endpoint).unwrap();
        println!("OWNED_LISTENER_READY");
        std::io::stdout().flush().unwrap();
        let mut stream = listener.accept().await.unwrap();
        let mut received = Vec::new();
        stream.read_to_end(&mut received).await.unwrap();
        let expected: &[u8] = if expects_frame { b"fixture-frame" } else { b"" };
        assert_eq!(received, expected);
    });
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn launch(executable: &Path, endpoint: &Path, expects_frame: bool) -> OwnedChild {
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
    let mut pipe = BufReader::new(pipe);
    loop {
        let mut line = String::new();
        assert_ne!(pipe.read_line(&mut line).unwrap(), 0, "child exited before readiness");
        if line.trim() == "OWNED_LISTENER_READY" { break; }
    }
    // Keep draining so a dropped readiness pipe cannot mask child success.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
    });
    child
}

#[test]
#[ignore = "requires explicitly selected protected Linux socket parent"]
fn verifies_owned_child_and_refuses_explicit_mismatches() {
    let parent = PathBuf::from(std::env::var_os("WOSY_NATIVE_GATE_PARENT").unwrap());
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let work = parent.join(format!("w6p-{}-{nonce:x}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&work).unwrap();
    let executable = std::env::current_exe().unwrap();
    let opened = File::open(&executable).unwrap();
    // Deliberately wrong regular file; never executed.
    let image = work.join("wrong-image");
    fs::write(&image, b"not the selected test executable").unwrap();
    let wrong = File::open(&image).unwrap();

    io_runtime().block_on(async {
        let good_path = work.join("good");
        let wrong_image_path = work.join("image");
        let sibling_path = work.join("sibling");
        let mut good = launch(&executable, &good_path, true);
        let mut wrong_image = launch(&executable, &wrong_image_path, false);
        let mut sibling = launch(&executable, &sibling_path, false);

        let problem = connect_owned_child(&sibling_path, &mut good.0, &opened).await.unwrap_err();
        assert_eq!(problem.to_string(), "local diagnostic peer is not the owned child");
        assert!(sibling.0.wait().unwrap().success());

        let problem = connect_owned_child(&wrong_image_path, &mut wrong_image.0, &wrong).await.unwrap_err();
        assert_eq!(problem.to_string(), "local diagnostic child executable does not match the opened image");
        assert!(wrong_image.0.wait().unwrap().success());

        let mut stream = connect_owned_child(&good_path, &mut good.0, &opened).await.unwrap();
        stream.write_all(b"fixture-frame").await.unwrap();
        stream.shutdown().await.unwrap();
        drop(stream);
        assert!(good.0.wait().unwrap().success());

        let problem = connect_owned_child(&good_path, &mut good.0, &opened).await.unwrap_err();
        assert_eq!(problem.to_string(), "owned local diagnostic child already exited");
    });
    assert!(!work.join("good").exists() && !work.join("image").exists() && !work.join("sibling").exists());
    fs::remove_file(image).unwrap();
    fs::remove_dir(work).unwrap();
}
