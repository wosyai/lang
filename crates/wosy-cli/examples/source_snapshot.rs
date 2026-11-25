//! Local integration driver, intentionally not wired into public commands.
use std::path::{Path, PathBuf};
use std::time::Instant;
use wosy_cli::source_preparation::prepare;
use rowan::{GreenNodeBuilder, NodeOrToken, WalkEvent};
use wosy_compiler::{ScalarExpression, ScalarItem, ScalarSourceDeclarations};
use wosy_daemon::{BaseDeclarationsView, QueryDisposition, SourceStore};
use wosy_syntax::{CanonicalCstRoot, CstNode, SyntaxKind};
use wosy_project::{PackageRelativePath, ProjectConfiguration, ProjectContext};
use wosy_protocol::{FileKey, NodeId};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn milliseconds(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1_000.0
}

fn selected_parse(project: &Path, source: &wosy_syntax::SourceIdentity, key: &FileKey)
    -> Result<wosy_syntax::FreshParseResult>
{
    let (cfg, manifest) = ProjectConfiguration::load_with_manifest(project)?;
    let target = cfg.resolve_target(None)?;
    let ctx = ProjectContext::from_manifest(project, &target, manifest.as_table())?;
    let text = std::fs::read_to_string(Path::new(&source.project).join(key.path.as_path()))?;
    let (fresh, diagnostics) =
        wosy_compiler::parse_source_with_snapshot(source.clone(), text, &ctx.comment_categories);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    Ok(fresh)
}

fn float_bits(declarations: &ScalarSourceDeclarations) -> Vec<String> {
    declarations.items().iter().filter_map(|item| match item {
        ScalarItem::Binding(binding) => match &binding.value {
            ScalarExpression::Float { value, .. } => Some(format!("{:016x}", value.to_bits())),
            _ => None,
        },
        _ => None,
    }).collect()
}

fn verify_declarations(project: &Path, key: &FileKey, view: &BaseDeclarationsView) -> Result<()> {
    let fresh = selected_parse(project, &view.source, key)?;
    let ordinary = wosy_compiler::derive_scalar_declarations_from_cst(fresh.result().canonical_cst());
    assert_eq!(view.authored_diagnostics(), ordinary.diagnostics());
    let rooted = view.executable_declarations();
    assert_eq!(float_bits(rooted), float_bits(&ordinary));
    assert_eq!(rooted.namespaces().count(), ordinary.namespaces().count());
    for (executable, authored) in rooted.namespaces().zip(ordinary.namespaces()) {
        assert_eq!(view.authored_span(executable.span), authored.span);
        assert_eq!(view.authored_span(executable.binding_span), authored.binding_span);
        assert_eq!(view.authored_span(executable.package_span), authored.package_span);
        assert_eq!(view.authored_span(executable.path_span), authored.path_span);
    }

    // Independent test oracle: use the ordinary fresh parser's typed Rowan
    // events, not the store's decoder or cached declaration output.
    let mut green = GreenNodeBuilder::new();
    for event in fresh.result().canonical_cst().root.preorder_with_tokens() {
        match event {
            WalkEvent::Enter(NodeOrToken::Node(node)) => {
                green.start_node(rowan::SyntaxKind(node.kind() as u16));
            }
            WalkEvent::Enter(NodeOrToken::Token(token)) if token.kind() != SyntaxKind::TypedComment => {
                green.token(rowan::SyntaxKind(token.kind() as u16), token.text());
            }
            WalkEvent::Leave(NodeOrToken::Node(_)) => green.finish_node(),
            _ => {}
        }
    }
    let executable = CanonicalCstRoot::new(rooted.source().clone(), CstNode::new_root(green.finish()));
    let oracle = wosy_compiler::derive_scalar_declarations_from_cst(&executable);
    assert_eq!(serde_json::to_vec(rooted)?, serde_json::to_vec(&oracle)?);
    Ok(())
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
    if let Some(package) = args.next() {
        let key = FileKey {
            package,
            path: PackageRelativePath::new(args.next().ok_or("expected file path")?.into())?,
        };
        let command = args.next();
        let root = prior.ok_or("file selection needs a root")?;
        let start = Instant::now();
        if command.as_deref() == Some("--declarations-pair") {
            let stale = if args.next().as_deref() == Some("--stale-temp") {
                let dir = dir.join("cas");
                std::fs::create_dir_all(&dir)?;
                let path = dir.join(format!(".projection-{}-0.tmp", std::process::id()));
                std::fs::OpenOptions::new().create_new(true).write(true).open(&path)?;
                Some(path)
            } else {
                None
            };
            let start = Instant::now();
            let (first, second) = tokio::join!(
                store.base_declarations(root, &key),
                store.base_declarations(root, &key),
            );
            let first = first?.ok_or("unknown first rooted file")?;
            let second = second?.ok_or("unknown second rooted file")?;
            let query_ms = milliseconds(start);
            assert_eq!(first.id, second.id);
            assert_eq!(serde_json::to_vec(first.executable_declarations())?,
                       serde_json::to_vec(second.executable_declarations())?);
            let derived = [first.disposition, second.disposition].into_iter()
                .filter(|disposition| *disposition == QueryDisposition::Derived).count();
            let first_bits = float_bits(first.executable_declarations());
            assert_eq!(first_bits, float_bits(second.executable_declarations()));
            if let Some(path) = stale {
                assert_eq!(std::fs::metadata(&path)?.len(), 0);
                std::fs::remove_file(path)?;
            }
            println!("{}", serde_json::json!({
                "declarations": first.id.hex(), "derived": derived, "query_ms": query_ms,
                "float_bits": first_bits,
            }));
            return store.close().await;
        }
        if command.as_deref() == Some("--declarations") {
            let view = store.base_declarations(root, &key).await?.ok_or("unknown rooted file")?;
            let query_ms = milliseconds(start);
            if args.next().as_deref() == Some("--oracle") {
                verify_declarations(&project, &key, &view)?;
            }
            println!("{}", serde_json::json!({
                "declarations": view.id.hex(),
                "executable": view.executable.hex(),
                "module": view.module.hex(),
                "source": view.source,
                "disposition": format!("{:?}", view.disposition),
                "diagnostics": view.authored_diagnostics(),
                "float_bits": float_bits(view.executable_declarations()),
                "query_ms": query_ms,
            }));
            return store.close().await;
        }
        let base = store.snapshot(root)?.ok_or("unknown retained snapshot")?;
        let load_base_ms = milliseconds(start);
        let start = Instant::now();
        let authored = base.authored(&key)?;
        let readback_ms = milliseconds(start);
        let cst = serde_json::to_value(authored.cst.snapshot_json_serializable())?;
        if command.as_deref() == Some("--oracle") {
            let fresh = selected_parse(&project, &authored.cst.source, &key)?;
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
    let base = prior.map(|r| store.snapshot(r)).transpose()?.flatten();
    let load_base_ms = milliseconds(start);
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
