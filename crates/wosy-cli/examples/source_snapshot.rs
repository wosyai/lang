//! Local integration driver, intentionally not wired into public commands.
use std::path::{Path, PathBuf};
use std::time::Instant;
use wosy_cli::source_preparation::prepare;
use wosy_daemon::SourceStore;
use wosy_project::{PackageRelativePath, ProjectConfiguration, ProjectContext};
use wosy_protocol::{FileKey, NodeId};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn milliseconds(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1_000.0
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let process = Instant::now();
    let mut args = std::env::args().skip(1);
    let project = PathBuf::from(args.next().ok_or("expected project path")?);
    let dir = PathBuf::from(args.next().ok_or("expected store path")?);
    let prior = args.next().map(|s| NodeId::parse_hex(&s)).transpose()?;
    let startup_ms = milliseconds(process);
    let start = Instant::now();
    let store = SourceStore::open(&dir, 8 << 20)?;
    let open_ms = milliseconds(start);
    let start = Instant::now();
    let base = prior.map(|r| store.snapshot(r)).transpose()?.flatten();
    let load_base_ms = milliseconds(start);
    if let Some(package) = args.next() {
        let key = FileKey {
            package,
            path: PackageRelativePath::new(args.next().ok_or("expected file path")?.into())?,
        };
        let start = Instant::now();
        let authored = base.as_ref().ok_or("unknown retained snapshot")?.authored(&key)?;
        let readback_ms = milliseconds(start);
        let cst = serde_json::to_value(authored.cst.snapshot_json_serializable())?;
        if args.next().as_deref() == Some("--oracle") {
            // The independent fresh parser comparison is outside the timed readback.
            let (cfg, manifest) = ProjectConfiguration::load_with_manifest(&project)?;
            let target = cfg.resolve_target(None)?;
            let ctx = ProjectContext::from_manifest(&project, &target, manifest.as_table())?;
            let source = &authored.cst.source;
            let text = std::fs::read_to_string(Path::new(&source.project).join(key.path.as_path()))?;
            let (fresh, diagnostics) = wosy_compiler::parse_source_with_snapshot(
                source.clone(), text, &ctx.comment_categories,
            );
            assert!(diagnostics.is_empty());
            assert_eq!(cst, serde_json::to_value(fresh.snapshot_json_serializable())?);
            assert_eq!(authored.comments, fresh.result().comments);
        }
        println!("{}", serde_json::json!({
            "cst": cst, "comments": authored.comments, "text": authored.cst.root.text().to_string(),
            "open_ms": open_ms, "load_base_ms": load_base_ms, "readback_ms": readback_ms,
        }));
        drop(base);
        return store.close().await;
    }

    let start = Instant::now();
    let prepared = prepare(&project, None, base.as_ref().map(|b| &b.files))?;
    let observe_ms = milliseconds(start);
    drop(base);
    let start = Instant::now();
    let writes = store.publish(prepared.root, &prepared.delta).await?;
    let publish_ms = milliseconds(start);
    let start = Instant::now();
    let view = store.snapshot(prepared.root)?.ok_or("new source root missing")?;
    let files: Vec<_> = view.files.iter().map(|(key, f)| serde_json::json!({
        "package": key.package, "path": key.path, "module": view.modules[key].hex(),
        "authored": f.authored.hex(), "executable": f.executable.hex(),
        "imports": f.metadata.imports,
    })).collect();
    let read_index_ms = milliseconds(start);
    let (executable, annotation) = (view.executable, view.annotation);
    drop(view);
    let start = Instant::now();
    let result = serde_json::json!({
        "source_root": prepared.root.hex(), "parsed": prepared.parsed, "reused": prepared.reused,
        "written": writes.written, "namespace_derived": writes.namespace_derived,
        "namespace_reused": writes.namespace_reused, "executable": executable.hex(), "annotations": annotation.hex(),
        "files": files, "startup_ms": startup_ms, "open_ms": open_ms, "load_base_ms": load_base_ms,
        "observe_ms": observe_ms, "publish_ms": publish_ms, "read_index_ms": read_index_ms,
    });
    store.close().await?;
    println!("{}", serde_json::json!({"close_ms": milliseconds(start), "snapshot": result}));
    Ok(())
}
