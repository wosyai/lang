//! Developer-only server for a root that is already published in the store.
//! Build with `--features native-diagnostics --example native_known_diagnostics_server`.
//! Run the daemon example with STORE PROJECT TARGET ROOT NEW_PRIVATE_DIRECTORY,
//! then run the client example after the daemon prints LISTENING. Programmatic
//! callers use SourceStorageWorker::listen_native_known_diagnostics instead.
#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::{num::NonZeroUsize, path::PathBuf, sync::Arc, time::Duration};
    use wosy_daemon::{SourceStorageWorker, SourceStore};
    use wosy_protocol::{BuildSourceSelection, NodeId};

    let args: [std::ffi::OsString; 5] = std::env::args_os().skip(1).collect::<Vec<_>>().try_into()
        .map_err(|_| "expected STORE PROJECT TARGET ROOT NEW_PRIVATE_DIRECTORY")?;
    let [store, project, target, root, directory] = args;
    let store = PathBuf::from(store);
    if !store.is_dir() { return Err("expected an existing daemon source store".into()); }
    let project = PathBuf::from(project).canonicalize()?.into_os_string().into_string()
        .map_err(|_| "project path must be UTF-8")?;
    let target = target.into_string().map_err(|_| "target must be UTF-8")?;
    let root = NodeId::parse_hex(&root.into_string().map_err(|_| "root must be UTF-8")?)?;
    let directory = PathBuf::from(directory);
    let store = Arc::new(SourceStore::open(&store, 8 << 20)?);
    let positive = NonZeroUsize::new(1).unwrap();
    let worker = SourceStorageWorker::new(Arc::clone(&store),
        tokio::runtime::Handle::current(), positive, positive, positive)?;
    let listener = worker.listen_native_known_diagnostics(&directory, BuildSourceSelection { project, target, root })?;
    println!("LISTENING");
    let result = listener.serve(Duration::from_secs(30)).await;
    drop(worker);
    Arc::into_inner(store).ok_or("a daemon worker is still draining")?.close().await?;
    eprintln!("{:?}", result?);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("Native known-root diagnostics are supported only on Linux");
    std::process::ExitCode::FAILURE
}
