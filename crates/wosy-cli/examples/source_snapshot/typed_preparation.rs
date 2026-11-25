use super::{milliseconds, without_typed_comments, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;
use wosy_compiler::{
    derive_scalar_declarations_from_cst, parse_source_with_snapshot, prepare_scalar_project,
    PreparedScalarProject, ScalarNamespaceBinding, ScalarPreparationInputs,
    ScalarProjectPreparationRejection, ScalarSourceDeclarations, ScalarTargetLayout, ScalarType,
};
use wosy_daemon::{RootedPreparedProject, SourceStore};
use wosy_project::{ProjectConfiguration, ProjectContext};
use wosy_protocol::{FileEntry, FileKey, NodeId};
use wosy_syntax::SourceIdentity;

fn fresh_preparation(
    project: &Path, profile: &str, executable: Option<&BTreeMap<FileKey, FileEntry>>,
) -> Result<std::result::Result<PreparedScalarProject, ScalarProjectPreparationRejection>> {
    let project = project.canonicalize()?;
    let (cfg, manifest) = ProjectConfiguration::load_with_manifest(&project)?;
    let target = cfg.resolve_target(None)?;
    let ctx = ProjectContext::from_manifest(&project, &target, manifest.as_table())?;
    let target_layout = cfg.artifact_profile(&target, profile)?.2.target_layout()?;
    let target_layout = ScalarTargetLayout {
        pointer_size: target_layout.pointer_size, pointer_alignment: target_layout.pointer_alignment,
    };
    let mut declarations = BTreeMap::<_, ScalarSourceDeclarations>::new();
    let graph = wosy_cli::project_sources::load(&project, &ctx, &cfg, true, |source, rev, text| {
        let key = FileKey::from(source);
        let path = source.path.as_path().to_str().ok_or("expected UTF-8 fixture path")?;
        let authored = SourceIdentity::new(
            source.project.clone(), source.package.clone(), path.to_owned(), rev.0.clone(),
        );
        let (fresh, diagnostics) = parse_source_with_snapshot(authored.clone(), text, &ctx.comment_categories);
        if !diagnostics.is_empty() { return Err(format!("{diagnostics:?}")); }
        let cst = fresh.result().canonical_cst();
        let body = if let Some(files) = executable {
            let mut owner = authored;
            owner.revision = files[&key].executable.hex();
            derive_scalar_declarations_from_cst(&without_typed_comments(cst, owner))
        } else {
            derive_scalar_declarations_from_cst(cst)
        };
        let imports = wosy_cli::project_sources::imports(&body)?;
        declarations.insert(key, body);
        Ok(imports)
    }).map_err(|error| match error {
        wosy_cli::project_sources::Error::Observe(error) => error,
        wosy_cli::project_sources::Error::Graph(error) => format!("{error:?}"),
    })?;
    let sources: Vec<_> = graph.nodes.iter().map(|node| {
        declarations[&FileKey::from(&node.source)].source().clone()
    }).collect();
    let modules = graph.nodes.iter().map(|node| {
        let body = declarations.remove(&FileKey::from(&node.source)).expect("fresh module");
        let bindings = node.namespace_edges.iter().map(|edge| ScalarNamespaceBinding {
            binding: edge.binding.clone(),
            target: sources[edge.target.0].clone(),
            span: wosy_syntax::ByteSpan::new(edge.span.range.start, edge.span.range.end),
        }).collect();
        (body, bindings)
    }).collect();
    let order = graph.initialization_order.iter().map(|id| sources[id.0].clone()).collect();
    Ok(prepare_scalar_project(modules, order, ScalarPreparationInputs { target_layout }))
}

fn assert_ordinary(
    store: &SourceStore, view: &RootedPreparedProject<'_>, project: &Path, profile: &str,
) -> Result<()> {
    let snapshot = store.snapshot(view.declarations().snapshot)?.expect("retained oracle root");
    let native = fresh_preparation(project, profile, Some(&snapshot.files))?;
    match (view.prepared(), native) {
        (Some(rooted), Ok(ordinary)) => {
            assert_eq!(rooted.project(), ordinary.project());
            for module in &rooted.project().modules {
                let author = view.authored_source(&module.source).expect("typed module author");
                let file = &snapshot.files[&FileKey {
                    package: module.source.package.clone(),
                    path: wosy_project::PackageRelativePath::new(module.source.path.clone().into())?,
                }];
                assert_eq!(author.project, file.metadata.source.project);
                assert_eq!(author.revision, file.metadata.revision.hex());
            }
        }
        (None, Err(_)) => {}
        _ => panic!("fresh and rooted executable preparation differ"),
    }
    let authored = fresh_preparation(project, profile, None)?;
    if let Err(ordinary) = authored {
        assert_eq!(view.authored_diagnostics()?, ordinary.into_inspection().diagnostics);
    } else {
        assert!(view.prepared().is_some());
        assert!(view.authored_diagnostics()?.is_empty());
    }
    Ok(())
}

fn report(view: &RootedPreparedProject<'_>, elapsed_ms: f64) -> Result<serde_json::Value> {
    let modules: Vec<_> = view.prepared().map_or_else(Vec::new, |prepared| {
        prepared.project().modules.iter().map(|module| {
            let members: BTreeMap<_, _> = module.members.iter().filter_map(|(name, ty)| match ty {
                ScalarType::Struct(id) => Some((name, &id.source)),
                ScalarType::Enum(id) => Some((name, &id.source)),
                _ => None,
            }).collect();
            let structs: Vec<_> = module.structs.iter().map(|structure| serde_json::json!({
                "name": structure.name, "owner": structure.id.source,
                "layout": module.layouts.aggregate(&structure.id).and_then(|value| value.layout.as_ref()),
                "copy_policy": structure.copy_policy,
                "field_owners": structure.fields.iter().map(|field| &field.id.structure.source).collect::<Vec<_>>(),
            })).collect();
            serde_json::json!({
                "owner": module.source, "author": view.authored_source(&module.source),
                "structs": structs, "members": members,
            })
        }).collect()
    });
    let out = &view.report;
    let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
    Ok(serde_json::json!({
        "snapshot": view.declarations().snapshot.hex(), "project_ast": view.declarations().entry_projection.hex(),
        "prepared_identity": view.identity.hex(), "ready": view.prepared().is_some(),
        "layout": view.target_layout, "reused_live": out.reused_live_preparation,
        "projected_ms": ms(out.projected_declarations), "materialized_ms": ms(out.materialized_bodies),
        "compiler_ms": ms(out.compiler_preparation), "query_ms": elapsed_ms,
        "bodies_derived": view.declarations().report.bodies_derived,
        "modules": modules, "diagnostics": view.authored_diagnostics()?,
    }))
}

pub(super) async fn run(
    store: &SourceStore, project: &Path, root: NodeId, profile: &str, mode: Option<&str>,
    other_root: Option<NodeId>, other_profile: Option<&str>,
) -> Result<serde_json::Value> {
    let start = Instant::now();
    if mode == Some("--held") {
        let (first, second) = tokio::join!(
            store.rooted_prepared_project(root, profile),
            store.rooted_prepared_project(other_root.unwrap_or(root), other_profile.unwrap_or(profile)),
        );
        let first = first?.ok_or("unknown first rooted preparation")?;
        let second = second?.ok_or("unknown second rooted preparation")?;
        if let (Some(one), Some(two)) = (first.prepared(), second.prepared()) {
            assert_eq!(std::ptr::eq(one, two), first.identity == second.identity);
        }
        let elapsed = milliseconds(start);
        return Ok(serde_json::json!([report(&first, elapsed)?, report(&second, elapsed)?]));
    }
    let first = store.rooted_prepared_project(root, profile).await?.ok_or("unknown rooted preparation")?;
    let elapsed = milliseconds(start);
    if mode == Some("--oracle") { assert_ordinary(store, &first, project, profile)?; }
    let first_report = report(&first, elapsed)?;
    if mode != Some("--released") { return Ok(first_report); }
    drop(first);
    let start = Instant::now();
    let next = store.rooted_prepared_project(root, profile).await?.ok_or("reopen rooted preparation")?;
    Ok(serde_json::json!([first_report, report(&next, milliseconds(start))?]))
}
