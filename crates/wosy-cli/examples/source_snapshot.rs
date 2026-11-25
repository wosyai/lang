//! Local integration driver, intentionally not wired into public commands.
use std::path::{Path, PathBuf};
use std::time::Instant;
use wosy_cli::source_preparation::prepare;
use rowan::{GreenNodeBuilder, NodeOrToken, WalkEvent};
use wosy_compiler::{ScalarExpression, ScalarItem, ScalarSourceDeclarations};
use wosy_daemon::{BaseDeclarationsView, ProjectDeclarationsView, QueryDisposition, SourceStore};
use wosy_syntax::{CanonicalCstRoot, CstNode, SyntaxKind};
use wosy_project::{PackageRelativePath, ProjectConfiguration, ProjectContext, SourceNodeId};
use wosy_protocol::{FileKey, NodeId};
use std::collections::BTreeMap;

#[path = "source_snapshot/typed_preparation.rs"]
mod typed_preparation;
#[path = "source_snapshot/retention.rs"]
mod retention;
#[path = "source_snapshot/recovery.rs"]
mod recovery;
#[path = "source_snapshot/storage_worker.rs"]
mod storage_worker;
#[path = "source_snapshot/source_state.rs"]
mod source_state;

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

    let executable = without_typed_comments(fresh.result().canonical_cst(), rooted.source().clone());
    let oracle = wosy_compiler::derive_scalar_declarations_from_cst(&executable);
    assert_eq!(serde_json::to_vec(rooted)?, serde_json::to_vec(&oracle)?);
    Ok(())
}

// Independent oracle: use the fresh parser's typed Rowan events, not the
// store's decoder or its detached declaration owner attachment.
fn without_typed_comments(
    authored: &CanonicalCstRoot, owner: wosy_syntax::SourceIdentity,
) -> CanonicalCstRoot {
    let mut green = GreenNodeBuilder::new();
    for event in authored.root.preorder_with_tokens() {
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
    CanonicalCstRoot::new(owner, CstNode::new_root(green.finish()))
}

async fn report_project(
    store: &SourceStore, project: &Path, root: NodeId, mode: Option<&str>,
) -> Result<serde_json::Value> {
    let start = Instant::now();
    let view = store.project_declarations(root).await?.ok_or("unknown rooted project")?;
    let query_ms = milliseconds(start);
    let modules: Vec<_> = view.modules().iter().map(|module| serde_json::json!({
        "key": module.key, "source": module.source, "module": module.module.hex(),
        "projection": module.projection.hex(), "body": module.body.hex(),
        "imports": module.imports.iter().map(|import| serde_json::json!({
            "authored": import.authored, "target": import.target,
        })).collect::<Vec<_>>(),
    })).collect();
    let materialized = match mode {
        Some("--oracle") => Some(materialize_project(&view, project, true).await?),
        Some("--load") => Some(materialize_project(&view, project, false).await?),
        _ => None,
    };
    Ok(serde_json::json!({
        "snapshot": view.snapshot.hex(), "projection": view.entry_projection.hex(),
        "modules_derived": view.report.modules_derived, "modules_reused": view.report.modules_reused,
        "bodies_derived": view.report.bodies_derived, "modules": modules,
        "query_ms": query_ms, "materialized": materialized,
    }))
}

async fn materialize_project(
    view: &ProjectDeclarationsView<'_>, project: &Path, oracle: bool,
) -> Result<serde_json::Value> {
    if oracle {
        let project = project.canonicalize()?;
        let (cfg, manifest) = ProjectConfiguration::load_with_manifest(&project)?;
        let target = cfg.resolve_target(None)?;
        let ctx = ProjectContext::from_manifest(&project, &target, manifest.as_table())?;
        let graph = wosy_cli::project_sources::load(&project, &ctx, &cfg, |source, rev, text| {
            let identity = wosy_syntax::SourceIdentity::new(
                source.project.clone(), source.package.clone(),
                source.path.as_path().to_str().ok_or("expected UTF-8 fixture path")?.to_owned(),
                rev.0.clone(),
            );
            let (fresh, diagnostics) =
                wosy_compiler::parse_source_with_snapshot(identity, text, &ctx.comment_categories);
            if !diagnostics.is_empty() { return Err(format!("{diagnostics:?}")); }
            let declarations = wosy_compiler::derive_scalar_declarations_from_cst(fresh.result().canonical_cst());
            wosy_cli::project_sources::imports(&declarations)
        }).map_err(|error| match error {
            wosy_cli::project_sources::Error::Observe(error) => error,
            wosy_cli::project_sources::Error::Graph(error) => format!("{error:?}"),
        })?;
        assert_eq!(view.modules().len(), graph.nodes.len());
        for (ordinal, (module, node)) in view.modules().iter().zip(&graph.nodes).enumerate() {
            assert_eq!(SourceNodeId(ordinal), graph.initialization_order[ordinal]);
            assert_eq!(module.key, FileKey::from(&node.source));
            assert_eq!(module.source.project, node.source.project);
            assert_eq!(module.source.revision, node.content_revision.0);
            assert_eq!(module.imports.len(), node.namespace_edges.len());
            for (stored, fresh) in module.imports.iter().zip(&node.namespace_edges) {
                assert_eq!(stored.target, fresh.target);
                assert_eq!(stored.authored.binding, fresh.binding);
                assert_eq!(stored.authored.origin, fresh.origin);
                assert_eq!(stored.authored.span.start, fresh.span.range.start);
                assert_eq!(stored.authored.span.end, fresh.span.range.end);
            }
        }
    }
    let start = Instant::now();
    let mut items = 0;
    let mut diagnostics = Vec::new();
    let mut derived = 0;
    for (ordinal, module) in view.modules().iter().enumerate() {
        let body = view.declarations(SourceNodeId(ordinal)).await?.ok_or("missing rooted declaration")?;
        assert_eq!(body.id, module.body);
        assert_eq!(body.source, module.source);
        assert_eq!(body.module, module.module);
        if oracle { verify_declarations(project, &module.key, &body)?; }
        items += body.executable_declarations().items().len();
        derived += usize::from(body.disposition == QueryDisposition::Derived);
        diagnostics.extend(body.authored_diagnostics());
    }
    assert!(view.declarations(SourceNodeId(view.modules().len())).await?.is_none());
    Ok(serde_json::json!({
        "items": items, "diagnostics": diagnostics, "bodies_derived": derived,
        "load_ms": milliseconds(start),
    }))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let process = Instant::now();
    let mut args = std::env::args().skip(1);
    let project = PathBuf::from(args.next().ok_or("expected project path")?);
    let dir = PathBuf::from(args.next().ok_or("expected store path")?);
    let selection = args.next();
    if selection.as_deref() == Some("--persistent-source") {
        return source_state::run(&project, &dir, &mut args).await;
    }
    if selection.as_deref() == Some("--source-worker") {
        let scope = NodeId::parse_hex(&args.next().ok_or("expected fixture scope")?)?;
        let old = NodeId::parse_hex(&args.next().ok_or("expected fixture historical root")?)?;
        return std::thread::spawn(move || storage_worker::run(&dir, scope, old))
            .join().map_err(|_| "storage worker fixture panicked")?;
    }
    if selection.as_deref() == Some("--retained-source-budget") {
        let key = FileKey {
            package: args.next().ok_or("expected a selected source package")?,
            path: PackageRelativePath::new(args.next().ok_or("expected a selected source path")?.into())?,
        };
        return recovery::run_budget(&project, &dir, &key).await;
    }
    if selection.as_deref() == Some("--staged-source-deltas") {
        let replacement = args.next().ok_or("expected a replacement fixture store")?;
        let key = FileKey {
            package: args.next().ok_or("expected a selected source package")?,
            path: PackageRelativePath::new(args.next().ok_or("expected a selected source path")?.into())?,
        };
        return recovery::run_deltas(&project, &dir, Path::new(&replacement), &key).await;
    }
    if selection.as_deref() == Some("--reconcile-source") {
        let replacement = args.next().ok_or("expected a replacement fixture store")?;
        let key = FileKey {
            package: args.next().ok_or("expected a selected source package")?,
            path: PackageRelativePath::new(args.next().ok_or("expected a selected source path")?.into())?,
        };
        return recovery::run(&project, &dir, Path::new(&replacement), &key).await;
    }
    let publish_pin = selection.as_deref() == Some("--publish-pin-exit");
    let prior = if publish_pin { None } else {
        selection.map(|s| NodeId::parse_hex(&s)).transpose()?
    };
    let startup_ms = milliseconds(process);
    let start = Instant::now();
    let store = SourceStore::open(&dir, 8 << 20)?;
    let open_ms = milliseconds(start);
    if publish_pin {
        let name = args.next().ok_or("expected pin name")?;
        return retention::publish_and_pin(&store, &project, &name).await;
    }
    if let Some(package) = args.next() {
        if package == "--installed-git" {
            let root = prior.ok_or("Git selection query needs a root")?;
            let view = store.snapshot(root)?.ok_or("unknown rooted Git selection")?;
            let selected: BTreeMap<_, _> = view.installed_git_selections()?.into_iter().map(|(name, selection)| {
                (name, serde_json::json!({
                    "locked": NodeId(*selection.locked.as_bytes()).hex(),
                    "patches": NodeId(*selection.patches.as_bytes()).hex(),
                }))
            }).collect();
            println!("{}", serde_json::json!(selected));
            drop(view);
            return store.close().await;
        }
        if matches!(package.as_str(), "--source-pin" | "--source-unpin" | "--source-pins"
            | "--source-plan" | "--source-plan-readers") {
            let scope = prior.ok_or("source pin operation needs a root")?;
            retention::run(&store, scope, &package, &mut args).await?;
            return store.close().await;
        }
        if package == "--typed-project" {
            let root = prior.ok_or("typed project query needs a root")?;
            let profile = args.next().ok_or("typed project query needs a profile")?;
            let mode = args.next();
            let other = args.next().map(|text| NodeId::parse_hex(&text)).transpose()?;
            let other_profile = args.next();
            let result = typed_preparation::run(
                &store, &project, root, &profile, mode.as_deref(), other, other_profile.as_deref(),
            ).await?;
            println!("{result}");
            return store.close().await;
        }
        if package == "--project-declarations" {
            let root = prior.ok_or("project query needs a root")?;
            let mode = args.next();
            if mode.as_deref() == Some("--concurrent") {
                let other = args.next().map(|text| NodeId::parse_hex(&text)).transpose()?.unwrap_or(root);
                let (first, second) = tokio::join!(
                    report_project(&store, &project, root, None),
                    report_project(&store, &project, other, None),
                );
                println!("{}", serde_json::json!([first?, second?]));
            } else {
                println!("{}", report_project(&store, &project, root, mode.as_deref()).await?);
            }
            return store.close().await;
        }
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
