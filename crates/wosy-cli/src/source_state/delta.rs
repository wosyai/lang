//! The read side of the exact selected/predecessor SQLite generation boundary.
use super::{
    load_usage, read_selection, AcknowledgedSourceGeneration, ClientSourceLimits, Closure, Files, Result, SourceAttempt,
    SqliteSourceState, StoredNodes,
};
use rusqlite::{params, Connection, TransactionBehavior};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wosy_project::{packages::{match_installed_git_base_files, InstalledGitBase}, PackageRelativePath};
use wosy_protocol::{
    partition_cst_files, BaseCstProposal, BaseCstProposalBinding, BaseDeclarationConnectionId,
    BaseDeclarationProjection,
    BuildSessionCommandId, BuildSessionReply, BuildSessionResponse, BuildSourceSelection, CstAddition,
    CstDelta, NodeId, SourceConfig, SourceDeltaCommit, SourceDeltaPublication, SourceIndexAddition,
    BASE_CST_DELTA_VERSION, BASE_DECLARATION_PROJECTION_VERSION,
};

pub struct ProjectedGitBaseFile {
    pub revision: NodeId,
    pub authored: NodeId,
    pub executable: NodeId,
}

/// Evidence about the reachable files of one directly selected Git package in
/// a pending source attempt. It grants no authority to a daemon, does not
/// acknowledge source, and is not a complete package representation.
pub struct PendingGitBaseProjection {
    pub project: PathBuf,
    pub target: String,
    pub package: String,
    pub attempt: SourceAttempt,
    pub parser_policy: NodeId,
    pub installed: InstalledGitBase,
    pub files: BTreeMap<PackageRelativePath, ProjectedGitBaseFile>,
}

pub enum PendingBasePublication {
    Applied(SourceDeltaCommit),
    PredecessorMissing,
    StaleAttempt,
}

pub(super) fn pending_base_matches(state: &super::SourceSelection, binding: &BaseCstProposalBinding) -> bool {
    state.selected == Some(binding.selected.root)
        && state.pending == Some(SourceAttempt::new(binding.proposed, binding.attempt))
}

impl SqliteSourceState {
    /// Construct a sparse proposal for the exact still-pending base source
    /// attempt, without observing live source, parsing, or acknowledging it.
    /// Reads both bounded source closures from one local snapshot, keeping
    /// base identities and novel encoded records, not two copies of the CST.
    /// A cold client needs the separate complete source identity handshake.
    pub fn pending_base_cst_proposal(
        &mut self, target: &str, attempt: SourceAttempt,
    ) -> Result<(BaseCstProposalBinding, BaseCstProposal)> {
        self.proposal_if_current(target, attempt)?.ok_or("base CST proposal is not the current source attempt".into())
    }

    fn proposal_if_current(
        &mut self, target: &str, attempt: SourceAttempt,
    ) -> Result<Option<(BaseCstProposalBinding, BaseCstProposal)>> {
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let selection = read_selection(&tx, target)?;
        if selection.pending != Some(attempt) {
            return Ok(None);
        }
        let base = selection.selected.ok_or("base CST proposal requires a selected predecessor")?;
        let root = attempt.root();
        let (known, _, previous) =
            trace_and_index(&tx, &self.project, target, base, self.limits, |_, _| ())?;
        let mut novel = BTreeMap::new();
        let (_, _, proposed) =
            trace_and_index(&tx, &self.project, target, root, self.limits, |id, bytes| {
                if !known.contains(id) { novel.insert(id, bytes); }
            })?;
        let partitions = partition_cst_files(&previous, &proposed, |id| novel.get(&id).map(Vec::as_slice))?;
        let files = partitions.into_iter().map(|partition| {
            let added = partition.added.into_iter().map(|id| {
                let bytes = novel.remove(&id).expect("partition only contains once-assigned novel records");
                CstAddition::from_canonical(id, bytes)
            }).collect::<std::result::Result<_, _>>()?;
            Ok(CstDelta {
                file: partition.file, old_file_root: partition.old_file_root,
                new_file_root: partition.new_file_root, added,
            })
        }).collect::<std::result::Result<_, &'static str>>()?;
        let source_index = novel.into_iter().map(|(id, bytes)| SourceIndexAddition::from_canonical(id, bytes))
            .collect::<std::result::Result<_, _>>()?;
        let project = self.project.to_str().ok_or("workspace path is not UTF-8")?;
        let binding = BaseCstProposalBinding {
            selected: BuildSourceSelection { root: base, project: project.into(), target: target.into() },
            proposed: root, attempt: attempt.generation(),
        };
        tx.commit()?;
        Ok(Some((binding, BaseCstProposal {
            version: BASE_CST_DELTA_VERSION, base, root, attempt: attempt.generation(), files, source_index,
        })))
    }

    /// Publish one still-pending immutable base-source update via the
    /// trusted local adapter. The adapter may retain a committed cache root
    /// even if our attempt advances. Only an exact local compare-and-set
    /// can select it, and no SQLite transaction spans daemon work.
    pub async fn publish_pending_base<F, Reply>(
        &mut self, target: &str, attempt: SourceAttempt, publish: F,
    ) -> Result<PendingBasePublication>
    where
        F: FnOnce(BaseCstProposalBinding, BaseCstProposal) -> Reply,
        Reply: Future<Output = Result<SourceDeltaPublication>>,
    {
        let Some((binding, proposal)) = self.proposal_if_current(target, attempt)? else {
            return Ok(PendingBasePublication::StaleAttempt);
        };
        if !self.pending_base_is_current(&binding)? { return Ok(PendingBasePublication::StaleAttempt); }
        let response = publish(binding.clone(), proposal).await?;
        self.finish_pending_base_publication(&binding, response)
    }

    /// One local prepared source operation with an explicitly selected
    /// trusted adapter. Both phases receive the same bounded canonical
    /// proposal, constructed once. The client retains its share across both
    /// calls and ends the SQLite read before invoking either callback.
    ///
    /// If the binding has advanced when preparation returns, do not start
    /// publication. If it advances during an admitted commit, only the daemon root may
    /// remain; the same SQLite compare-and-set as direct publication decides
    /// selection. This is not a compile-time result or a Build/Check route.
    pub async fn prepare_then_publish_pending_base<T, Prepare, Preparation, Publish, Publication>(
        &mut self, target: &str, attempt: SourceAttempt, prepare: Prepare, publish: Publish,
    ) -> Result<PendingBasePublication>
    where
        Prepare: FnOnce(BaseCstProposalBinding, Arc<BaseCstProposal>) -> Preparation,
        Preparation: Future<Output = Result<T>>,
        Publish: FnOnce(BaseCstProposalBinding, Arc<BaseCstProposal>, T) -> Publication,
        Publication: Future<Output = Result<SourceDeltaPublication>>,
    {
        let Some((binding, proposal)) = self.proposal_if_current(target, attempt)? else {
            return Ok(PendingBasePublication::StaleAttempt);
        };
        let proposal = Arc::new(proposal);
        let prepared = prepare(binding.clone(), Arc::clone(&proposal)).await;
        if !self.pending_base_is_current(&binding)? { return Ok(PendingBasePublication::StaleAttempt); }
        let response = publish(binding.clone(), Arc::clone(&proposal), prepared?).await?;
        self.finish_pending_base_publication(&binding, response)
    }

    pub(super) fn pending_base_is_current(&self, binding: &BaseCstProposalBinding) -> Result<bool> {
        Ok(pending_base_matches(&read_selection(&self.db, &binding.selected.target)?, binding))
    }

    fn finish_pending_base_publication(
        &self, binding: &BaseCstProposalBinding, response: SourceDeltaPublication,
    ) -> Result<PendingBasePublication> {
        let receipt = match response {
            SourceDeltaPublication::PredecessorMissing => return Ok(if self.pending_base_is_current(binding)? {
                PendingBasePublication::PredecessorMissing
            } else {
                PendingBasePublication::StaleAttempt
            }),
            SourceDeltaPublication::Committed(receipt) if receipt.binding == *binding => receipt,
            SourceDeltaPublication::Committed(_) => return Err("committed base source receipt does not match the pending attempt".into()),
        };
        let applied = self.db.execute(
            "UPDATE source_targets SET selected = ?2, selected_generation = ?3, pending = NULL
             WHERE target = ?1 AND pending = ?2 AND generation = ?3 AND selected = ?4",
            params![binding.selected.target, binding.proposed.0, binding.attempt, binding.selected.root.0],
        )? == 1;
        Ok(if applied { PendingBasePublication::Applied(receipt) } else { PendingBasePublication::StaleAttempt })
    }

    /// Project one directly installed pristine selection onto the exact
    /// still-pending source attempt (including a first/cold attempt). The
    /// bounded SQLite read and checkout observation are separate; neither
    /// acquires a writer lock across the other. The last SQLite read is a
    /// freshness cut, not a guarantee about future workspace or DB writes.
    pub fn pending_git_base_projection(
        &mut self, target: &str, attempt: SourceAttempt, package: &str,
    ) -> Result<Option<PendingGitBaseProjection>> {
        let (selection, config, files) = {
            let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let selection = read_selection(&tx, target)?;
            if selection.pending != Some(attempt) {
                return Err("Git base projection is not the current source attempt".into());
            }
            let (_, config, files) = trace_and_index(&tx, &self.project, target, attempt.root(), self.limits, |_, _| ())?;
            tx.commit()?;
            (selection, config, files)
        };
        let Some((installed, files)) = matching_git_base(&self.project, package, &config, files)? else { return Ok(None) };
        if read_selection(&self.db, target)? != selection {
            return Err("Git base projection source attempt advanced during observation".into());
        }
        Ok(Some(PendingGitBaseProjection {
            project: self.project.clone(), target: target.to_owned(), package: package.to_owned(),
            attempt, parser_policy: config.parser.id(), installed, files,
        }))
    }

    /// Consume only the receipt from the still-addressed local connection
    /// and original command/source attempt,
    /// while that same predecessor and pending attempt are still selected in
    /// one SQLite read. This is a local freshness cut, not a publication ack
    /// or proof of continuing freshness beyond this call.
    pub fn consume_base_declaration_projection<D>(
        &self, captured: &BaseCstProposalBinding, connection: BaseDeclarationConnectionId,
        command: BuildSessionCommandId, received: BuildSessionResponse<D>,
    ) -> Result<Option<BaseDeclarationProjection>> {
        let BuildSessionReply::BaseDeclarationProjection { projection } = received.reply else {
            return Ok(None);
        };
        if received.id != command || projection.connection != connection
            || projection.version != BASE_DECLARATION_PROJECTION_VERSION
            || projection.base != captured.selected.root || projection.proposed != captured.proposed
            || projection.attempt != captured.attempt || captured.attempt <= 0
            || Path::new(&captured.selected.project) != self.project
        {
            return Ok(None);
        }
        // A single SQL statement observes target, base, digest and generation
        // together. No transaction is held across a daemon request.
        let state = read_selection(&self.db, &captured.selected.target)?;
        let attempt = SourceAttempt::new(captured.proposed, captured.attempt);
        Ok((state.selected == Some(captured.selected.root) && state.pending == Some(attempt)).then_some(projection))
    }
}

impl AcknowledgedSourceGeneration {
    /// Offer an exact observed installed base only for the reachable package
    /// files of this acknowledged revision. The local adapter still opens
    /// and checks the daemon lease. No Git program or live compiler source
    /// is consulted; the anchored installation observation reads files.
    pub async fn open_installed_git_session<T, Open, Opening>(
        &self, local: &mut SqliteSourceState, package: &str, open: Open,
    ) -> Result<Option<T>>
    where
        Open: FnOnce(BuildSourceSelection, InstalledGitBase) -> Opening,
        Opening: Future<Output = Result<T>>,
    {
        let selected = self.selection();
        if local.project.to_str() != Some(selected.project.as_str()) { return Ok(None); }
        let (config, files) = {
            let tx = local.db.transaction_with_behavior(TransactionBehavior::Deferred)?;
            if !self.matches(read_selection(&tx, &selected.target)?) { return Ok(None); }
            let (_, config, files) = trace_and_index(
                &tx, &local.project, &selected.target, selected.root, local.limits, |_, _| (),
            )?;
            tx.commit()?;
            (config, files)
        };
        let Some((installation, _)) = matching_git_base(&local.project, package, &config, files)? else { return Ok(None) };
        if !self.is_current(local)? { return Ok(None); }
        let opened = open(selected.clone(), installation).await;
        // Never return a live session for a capture that advanced while
        // opening. This is a freshness cut, not a lease over SQLite.
        if self.is_current(local)? { opened.map(Some) } else { Ok(None) }
    }
}

fn matching_git_base(
    project: &Path, package: &str, config: &SourceConfig, files: Files,
) -> Result<Option<(InstalledGitBase, BTreeMap<PackageRelativePath, ProjectedGitBaseFile>)>> {
    let Some(namespace) = config.namespaces.get(package) else { return Ok(None) };
    let Some(git) = &namespace.git else { return Ok(None) };
    let files: BTreeMap<_, _> = files.into_iter().filter_map(|(key, file)| {
        (key.package == package).then_some((key.path, ProjectedGitBaseFile {
            revision: file.metadata.revision, authored: file.authored, executable: file.executable,
        }))
    }).collect();
    if files.is_empty() { return Ok(None); }
    let Some(installed) = match_installed_git_base_files(project, package,
        files.iter().map(|(path, file)| (path, &file.revision.0)),
    )? else { return Ok(None) };
    if installed.locked() != &git.locked || installed.patches() != git.patches
        || installed.checkout_path() != Path::new(&namespace.project)
    {
        return Err("staged Git base selection no longer matches the installed package".into());
    }
    Ok(Some((installed, files)))
}

fn trace_and_index(
    tx: &Connection, project: &Path, target: &str, root: NodeId, limits: ClientSourceLimits,
    keep: impl FnMut(NodeId, Vec<u8>),
) -> Result<(Closure, SourceConfig, Files)> {
    if let Some(usage) = load_usage(tx, root)? { usage.admit(limits)?; }
    let mut read = StoredNodes::new(tx)?;
    // A usage receipt can refuse early. Verify the actual closure before
    // retaining the file index even when a migrated root has no receipt.
    let closure = read.trace(root, limits, keep)?;
    let (config, entries) = SqliteSourceState::header(project, &mut read, target, root)?;
    let files = read.file_index(&config, entries)?;
    Ok((closure, config, files))
}
