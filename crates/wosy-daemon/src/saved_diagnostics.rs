//! Explicit historical Build diagnostic retention, separate from query reuse.
use super::{
    build_session::BuildRequestRejection, child_edge, file_source, key,
    pins::{workspace_of_source_config, Owner},
    read_verified_node_with, retention_plan::{SourceLease, SourceRead},
    rooted_llvm::{ScalarPlanning, RootedLlvmRejection, ScalarOutcome},
    source_reconciliation::{bounded_source_records, checked_source_header_using}, storage_worker::QueryWorker,
    validate_file_using, write_edge, write_node, BaseCstAdmissionLimits, FileTree, Result, RootedLlvmIdentity,
    SourceStore, NODES, SNAPSHOT_ROOTS,
};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashMap}, future::{poll_fn, Future}, io::{self, Write}, pin::pin, sync::Arc, task::Poll};
use surrealkv::{Durability, Mode, Transaction};
use wosy_compiler::{Diagnostic, COMPILER_IDENTITY};
use wosy_project::PackageRelativePath;
use wosy_protocol::{source_preparation_parts, FileEntry, FileKey, Kind, Node, NodeId, SourceConfig};
use wosy_syntax::{ByteSpan, CstNode, SourceIdentity};

const FORMAT: u16 = 1;
pub(super) const MAX_REPORT_BYTES: usize = 64 << 10;
const MAX_NAME_BYTES: usize = 1024;
const ARCHIVE_ENVELOPE: usize = 9 + 32;
const REPORT_BUDGET_ERROR: &str = "saved Build diagnostic exceeds report byte budget";

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedBuildDiagnostic {
    version: u16,
    pub source_root: NodeId,
    pub profile: String,
    pub compiler: String,
    pub compiler_image: NodeId,
    pub phase: SavedBuildPhase,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedBuildPhase { Declarations, Preparation, Analysis }

struct ReportBytes {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for ReportBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other(REPORT_BUDGET_ERROR));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

pub(super) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES { return Err("invalid saved Build diagnostic pin name".into()); }
    Ok(())
}

fn effective_report_limit(max_bytes: usize) -> usize { max_bytes.min(MAX_REPORT_BYTES) }

pub(super) fn validate_profile(profile: &str, max_bytes: usize) -> Result<()> {
    // This borrowed UTF-8 field alone cannot fit the serialized report.
    if profile.len() > effective_report_limit(max_bytes) { return Err(REPORT_BUDGET_ERROR.into()); }
    Ok(())
}

struct Archive {
    owner: Owner,
    report: SavedBuildDiagnostic,
    id: NodeId,
    bytes: Vec<u8>,
}

struct SavePlan {
    transaction: Transaction,
    _read: SourceRead,
}

pub(super) async fn save(
    worker: &QueryWorker, lease: Arc<SourceLease>, name: &str, profile: String,
    limits: BaseCstAdmissionLimits, max_bytes: usize,
) -> Result<SavedBuildDiagnostic> {
    let active = Arc::clone(&lease);
    let archive = worker.phase(None, move |store| {
        observe_archive(store, &active, profile, limits, max_bytes).map(Arc::new)
    }).await?;
    let name: Arc<str> = name.into();
    for conflicts in 0..=1 {
        let (copy, plan_lease, name) = (Arc::clone(&archive), Arc::clone(&lease), Arc::clone(&name));
        let plan = worker.phase(None, move |store| plan_save(store, &plan_lease, &name, &copy)).await?;
        match commit_save(plan, &lease, || worker.check_active()).await? {
            true => return Arc::into_inner(archive).map(|archive| archive.report)
                .ok_or("saved diagnostic held by another worker".into()),
            false if conflicts == 0 => continue,
            false => return Err(surrealkv::Error::TransactionWriteConflict.into()),
        }
    }
    unreachable!("a second commit conflict is returned")
}

/// Always produce a fresh ordinary outcome on authenticated authored CST.
/// No optional diagnostic, declaration, or LLVM result can supply it.
fn observe_archive(
    store: &SourceStore, lease: &SourceLease, profile: String,
    limits: BaseCstAdmissionLimits, max_bytes: usize,
) -> Result<Archive> {
    let grant = lease.grant();
    let (tx, _read) = store.begin_leased_source(grant, Mode::ReadOnly)?.ok_or(BuildRequestRejection::Expired)?;
    let plan = match store.plan_scalar(&tx, grant.root, profile, limits, RootedLlvmIdentity::Authored)? {
        ScalarPlanning::Ready(plan) => plan,
        ScalarPlanning::Unsupported(reason) => return Err(format!("unsupported saved Build observation: {reason:?}").into()),
    };
    let mut sources = HashMap::new();
    let observation = store.observe_planned_scalar(&tx, plan, limits, None,
        |source, tree| {
            if sources.insert(source.clone(), tree.clone()).is_some() {
                return Err("duplicate source in fresh Build observation".into());
            }
            Ok(())
        }, |_, _, _| Ok(()))?;
    let (phase, diagnostics) = match observation.outcome {
        ScalarOutcome::Rejected(RootedLlvmRejection::Declarations(report)) => (SavedBuildPhase::Declarations, report),
        ScalarOutcome::Rejected(RootedLlvmRejection::Preparation(report)) => (SavedBuildPhase::Preparation, report),
        ScalarOutcome::Rejected(RootedLlvmRejection::Analysis(report)) => (SavedBuildPhase::Analysis, report),
        ScalarOutcome::Unsupported(reason) => return Err(format!("unsupported saved Build observation: {reason:?}").into()),
        // The observer never reaches an emitter. A clean analysis is not a
        // standalone Build verdict and supplies no diagnostic to retain.
        ScalarOutcome::Value(()) => return Err("no ordinary Build diagnostics to save".into()),
        ScalarOutcome::Rejected(RootedLlvmRejection::Emitter(_)) | ScalarOutcome::AlreadyEmitted =>
            unreachable!("no emitter or projection receipt in saved observations"),
    };
    check_labels(&diagnostics, &sources)?;
    let report = SavedBuildDiagnostic {
        version: FORMAT, source_root: observation.root, profile: observation.profile,
        compiler: COMPILER_IDENTITY.to_owned(), compiler_image: observation.image, phase, diagnostics,
    };
    let mut encoded = ReportBytes { bytes: Vec::new(), limit: effective_report_limit(max_bytes) };
    serde_json::to_writer(&mut encoded, &report)?;
    let node = Node { kind: Kind::SavedBuildDiagnostic, data: encoded.bytes, children: vec![report.source_root] };
    let bytes = node.encode();
    Ok(Archive { owner: Owner::new(observation.workspace), report, id: Node::id(&bytes), bytes })
}

fn plan_save(store: &SourceStore, lease: &SourceLease, name: &str, archive: &Archive) -> Result<SavePlan> {
    let grant = lease.grant();
    let (mut tx, read) = store.begin_leased_source(grant, Mode::ReadWrite)?.ok_or(BuildRequestRejection::Expired)?;
    if archive.report.source_root != grant.root { return Err("Build observation belongs to a different root".into()); }
    let owner = &archive.owner;
    let pin_key = owner.pin_key(name)?;
    let old = tx.get_for_update(&pin_key)?;
    let same = if let Some(value) = old.as_deref() {
        let pin = owner.stored(value)?;
        if pin.retained_root() != Some(grant.root) || pin.saved_diagnostic() != Some(archive.id) {
            return Err("name already retains a different object".into());
        }
        true
    } else { false };
    if let Some(existing) = tx.get_for_update(&key(NODES, archive.id))? {
        if existing.as_slice() != archive.bytes { return Err("saved diagnostic differs from its immutable identity".into()); }
    } else {
        if same { return Err("saved diagnostic pin is missing its record".into()); }
        write_node(&mut tx, archive.id, &archive.bytes)?;
        write_edge(&mut tx, archive.id, child_edge(Kind::SavedBuildDiagnostic, 0).0, grant.root)?;
    }
    if !same { tx.set(&pin_key, owner.saved_value(grant.root, archive.id))?; }

    // Every dependency needed to display this historical report is inline
    // in the already admitted authored source graph. No optional AST/body
    // CAS blob is needed. Sync dependencies before admitting the new pin.
    store.tree.flush_wal(true)?;
    tx.set_durability(Durability::Immediate);
    Ok(SavePlan { transaction: tx, _read: read })
}

async fn commit_save(
    SavePlan { mut transaction, _read }: SavePlan, lease: &SourceLease, check_active: impl Fn() -> Result<()>,
) -> Result<bool> {
    let mut commit = pin!(transaction.commit());
    let mut started = false;
    let result = poll_fn(|cx| {
        if started { return commit.as_mut().poll(cx).map(Ok); }
        // Keep only this capped one-record transaction's first poll under
        // the registry gate. Once admitted it can drain after expiry or a
        // dropped caller; no await happens with the registry lock held.
        lease.finish_current(|| {
            if let Err(error) = check_active() { return Poll::Ready(Err(error)); }
            started = true;
            commit.as_mut().poll(cx).map(Ok)
        }).unwrap_or_else(|| Poll::Ready(Err(BuildRequestRejection::Expired.into())))
    }).await?;
    match result {
        Ok(()) => Ok(true),
        Err(surrealkv::Error::TransactionWriteConflict) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn read(
    store: &SourceStore, lease: &SourceLease, name: &str, limits: BaseCstAdmissionLimits, max_bytes: usize,
) -> Result<Option<SavedBuildDiagnostic>> {
    let grant = lease.grant();
    let (tx, _read) = store.begin_leased_source(grant, Mode::ReadOnly)?.ok_or(BuildRequestRejection::Expired)?;
    let mut record_bytes = bounded_source_records(&tx, limits);
    let (selected_tree, selected_config) = checked_source_header_using(grant.root, &mut record_bytes)?;
    let owner = Owner::new(workspace_of_source_config(&selected_config)?.to_owned());
    let Some(value) = tx.get(&owner.pin_key(name)?)? else { return Ok(None); };
    let pin = owner.stored(&value)?;
    let Some(record) = pin.saved_diagnostic() else { return Ok(None); };
    let root = pin.retained_root().ok_or("saved diagnostic lost its authored source")?;
    let (tree, config) = if root == grant.root {
        (selected_tree, selected_config)
    } else {
        let marker = tx.get(&key(SNAPSHOT_ROOTS, root))?.ok_or("saved diagnostic lost its published source")?;
        super::check_source_marker(root, &marker)?;
        let (tree, config) = checked_source_header_using(root, &mut record_bytes)?;
        if workspace_of_source_config(&config)? != workspace_of_source_config(&selected_config)? {
            return Err("saved diagnostic source belongs to another workspace".into());
        }
        (tree, config)
    };
    let (report, _) = read_record(&tx, record, root, max_bytes, |_| Ok(()))?;
    if !config.profiles.contains_key(&report.profile) { return Err("saved diagnostic profile is outside its source".into()); }
    let files = source_preparation_parts(&tree).expect("checked header").files;
    let sources = load_label_sources(&tx, &mut record_bytes, files, &config, &report.diagnostics, limits)?;
    check_labels(&report.diagnostics, &sources)?;
    lease.finish_current(|| report).ok_or_else(|| BuildRequestRejection::Expired.into()).map(Some)
}

fn load_label_sources(
    tx: &Transaction, mut read: impl FnMut(NodeId) -> Result<Node>, files: &[NodeId],
    config: &SourceConfig, diagnostics: &[Diagnostic], limits: BaseCstAdmissionLimits,
) -> Result<HashMap<SourceIdentity, CstNode>> {
    let mut requested = BTreeMap::new();
    for source in diagnostics.iter().flat_map(|diagnostic| diagnostic.labels.iter().map(|label| &label.span.source)) {
        let key = FileKey { package: source.package.clone(), path: PackageRelativePath::new(source.path.clone().into())? };
        if requested.insert(key, source).is_some_and(|old| old != source) {
            return Err("saved diagnostic has conflicting source revisions".into());
        }
    }
    let mut sources = HashMap::new();
    if requested.is_empty() { return Ok(sources); }
    let mut work = limits.cst_work();
    let policy = config.parser.id();
    for &id in files {
        let file = FileEntry::from_node(id, read(id)?)?;
        let key = FileKey::from(&file.metadata.source);
        let Some(&source) = requested.get(&key) else { continue; };
        if file.metadata.policy != policy || *source != file_source(&file)?
            || config.namespaces.get(&key.package).is_none_or(|ns| ns.project != source.project)
        {
            return Err("saved diagnostic label does not match the authored source".into());
        }
        if sources.contains_key(source) { return Err("duplicate source in saved diagnostic index".into()); }
        let checked = validate_file_using(&file, &config.parser.categories, FileTree::Authored, Some(&mut work),
            |id, work| read_verified_node_with(tx, id, |bytes| work.expect("bounded archive CST").read(bytes)))?;
        sources.insert(source.clone(), checked.authored.expect("authored tree was requested"));
    }
    Ok(sources)
}

fn check_labels(diagnostics: &[Diagnostic], sources: &HashMap<SourceIdentity, CstNode>) -> Result<()> {
    for label in diagnostics.iter().flat_map(|diagnostic| &diagnostic.labels) {
        let tree = sources.get(&label.span.source).ok_or("saved diagnostic label is outside the authored source")?;
        check_span(tree, label.span.range)?;
    }
    Ok(())
}

fn check_span(tree: &CstNode, span: ByteSpan) -> Result<()> {
    let extent = u32::from(tree.text_range().end());
    let boundary = |offset: u32| offset == 0 || offset == extent || (offset < extent
        && tree.token_at_offset(offset.into()).right_biased().is_some_and(|token| {
            token.text().is_char_boundary((offset - u32::from(token.text_range().start())) as usize)
        }));
    if span.start > span.end || !boundary(span.start) || !boundary(span.end) {
        return Err("saved diagnostic label is outside authored UTF-8 boundaries".into());
    }
    Ok(())
}

pub(super) fn read_record(
    tx: &Transaction, id: NodeId, root: NodeId, max_bytes: usize, charge: impl FnOnce(usize) -> Result<()>,
) -> Result<(SavedBuildDiagnostic, usize)> {
    let node = read_verified_node_with(tx, id, |bytes| {
        charge(bytes)?;
        if bytes > effective_report_limit(max_bytes) + ARCHIVE_ENVELOPE {
            return Err(REPORT_BUDGET_ERROR.into());
        }
        Ok(())
    })?;
    let report = decode(&node, root)?;
    Ok((report, node.encoded_len()))
}

fn decode(node: &Node, root: NodeId) -> Result<SavedBuildDiagnostic> {
    if node.kind != Kind::SavedBuildDiagnostic || node.children != [root] || node.data.len() > MAX_REPORT_BYTES {
        return Err("invalid saved Build diagnostic record".into());
    }
    let report: SavedBuildDiagnostic = serde_json::from_slice(&node.data)?;
    if report.version != FORMAT || report.source_root != root { return Err("saved diagnostic does not match its authored source".into()); }
    Ok(report)
}


#[cfg(test)]
mod tests {
    use super::*;
    use wosy_protocol::BuildSourceSelection;

    #[test]
    fn archive_labels_observe_inclusive_utf8_boundaries() {
        let text = "%%start\n# WHY: café ☕\n%%end\n";
        let source = SourceIdentity::new("fixture".into(), "app".into(), "unit.w".into(), "test".into());
        let tree = wosy_syntax::parse(source, text.into(), &["WHY".into()]).root;
        let start = text.find('☕').unwrap() as u32;
        let end = start + '☕'.len_utf8() as u32;
        let extent = text.len() as u32;
        for (from, to) in [(start, end), (0, extent), (extent, extent)] {
            assert!(check_span(&tree, ByteSpan::new(from, to)).is_ok());
        }
        for (from, to) in [(start + 1, end), (start, end - 1), (end, start), (extent, extent + 1)] {
            assert!(check_span(&tree, ByteSpan::new(from, to)).is_err());
        }
    }

    #[test]
    #[ignore = "requires a disposable acknowledged source catalog"]
    fn saved_build_diagnostic_commit_requires_live_lease_and_conditional_name() {
        let path = |name: &str| std::path::PathBuf::from(std::env::var_os(name).unwrap());
        let fixture = path("WOSY_SAVED_DIAGNOSTIC_PROJECT").canonicalize().unwrap();
        let root = NodeId::parse_hex(&std::env::var("WOSY_SAVED_DIAGNOSTIC_ROOT").unwrap()).unwrap();
        let selected = BuildSourceSelection { root, project: fixture.to_str().unwrap().into(), target: "app".into() };
        tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap().block_on(async {
            let store = SourceStore::open(&path("WOSY_SAVED_DIAGNOSTIC_STORE"), 8 << 20).unwrap();
            let limits = BaseCstAdmissionLimits {
                records: 500, record_bytes: 1 << 20, graph_edges: 1000, expanded_record_bytes: 1 << 20,
                cst_occurrences: 5000, cst_depth: 64, authored_bytes: 10_000,
            };
            let open = || store.lease_build_source(selected.clone(), std::time::Duration::from_secs(30)).unwrap().unwrap();
            let lease = open();
            let archive = observe_archive(&store, &lease, "dev".into(), limits, MAX_REPORT_BYTES).unwrap();
            assert!(!archive.report.diagnostics.is_empty());
            let tx = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
            let (top, _) = checked_source_header_using(root, bounded_source_records(&tx, limits)).unwrap();
            let parts = source_preparation_parts(&top).unwrap();
            let scope = super::super::build_receipt::BuildInputs {
                config: parts.config, executable: parts.executable, dependencies: parts.dependencies,
            }.scope(archive.report.compiler_image);
            let receipts = |tx: &Transaction| {
                let mut remaining = 2;
                super::super::build_receipt::attached_rows(tx, scope, &mut remaining, |_, _| Ok(())).unwrap();
                2 - remaining
            };
            assert_eq!(receipts(&tx), 0);
            drop(tx);
            let prepared = store.rooted_prepared_project(root, "dev").await.unwrap().unwrap();
            let ordinary = prepared.build_analysis().await.unwrap();
            assert!(ordinary.receipt_written);
            assert_eq!(receipts(&store.tree.begin_with_mode(Mode::ReadOnly).unwrap()), 1);
            // The live and persisted query receipts now exist, but cannot
            // supply a fresh authored archive observation.
            assert_eq!(observe_archive(&store, &lease, "dev".into(), limits, MAX_REPORT_BYTES)
                .unwrap().report, archive.report);
            drop(prepared);
            drop(lease);
            let absent = |name| {
                let txn = store.tree.begin_with_mode(Mode::ReadOnly).unwrap();
                let owner = Owner::new(selected.project.clone());
                assert!(txn.get(&owner.pin_key(name).unwrap()).unwrap().is_none());
                assert!(txn.get(&key(NODES, archive.id)).unwrap().is_none());
            };

            let lease = open();
            let pending = plan_save(&store, &lease, "interrupted", &archive).unwrap();
            // A future dropped before commit admission does not publish
            // either the actual archive node or the name.
            drop(commit_save(pending, &lease, || Ok(())));
            absent("interrupted");
            let pending = plan_save(&store, &lease, "expired", &archive).unwrap();
            lease.release();
            let error = commit_save(pending, &lease, || Ok(())).await.unwrap_err();
            assert!(matches!(error.downcast_ref::<BuildRequestRejection>(), Some(BuildRequestRejection::Expired)));
            drop(lease);
            absent("expired");

            let lease = open();
            let pending = plan_save(&store, &lease, "concurrent", &archive).unwrap();
            store.pin_source(root, "concurrent").await.unwrap();
            assert!(!commit_save(pending, &lease, || Ok(())).await.unwrap());
            assert!(read(&store, &lease, "concurrent", limits, MAX_REPORT_BYTES).unwrap().is_none());
            assert!(plan_save(&store, &lease, "concurrent", &archive).is_err());
            assert!(store.tree.begin_with_mode(Mode::ReadOnly).unwrap().get(&key(NODES, archive.id)).unwrap().is_none());
            assert_eq!(store.source_pins(root).unwrap()[0].name, "concurrent");

            // The archive has genuine Unicode and labels. The same stored
            // rows are refused under independently tight read budgets.
            let no_config = BaseCstAdmissionLimits { records: 1, ..limits };
            let no_config_bytes = BaseCstAdmissionLimits { record_bytes: top.encoded_len(), ..limits };
            let no_file_records = BaseCstAdmissionLimits { records: 2, ..limits };
            let no_expansion = BaseCstAdmissionLimits { authored_bytes: 0, ..limits };
            let over = observe_archive(&store, &lease, "dev".into(), no_expansion, MAX_REPORT_BYTES).err().unwrap();
            assert!(over.to_string().contains("authored byte budget"));
            assert!(commit_save(plan_save(&store, &lease, "normal", &archive).unwrap(), &lease, || Ok(())).await.unwrap());
            for (bound, reason) in [
                (no_config, "record budget"), (no_config_bytes, "byte budget"),
                (no_file_records, "record budget"), (no_expansion, "authored byte budget"),
            ] {
                assert!(read(&store, &lease, "normal", bound, MAX_REPORT_BYTES).unwrap_err().to_string().contains(reason));
            }
            assert_eq!(read(&store, &lease, "normal", limits, MAX_REPORT_BYTES).unwrap().as_ref(), Some(&archive.report));
            drop(lease);
            store.close().await.unwrap();
        });
    }
}
