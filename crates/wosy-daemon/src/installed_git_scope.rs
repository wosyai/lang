//! Bind a verified installed file subset to the exact selected source lease.
use super::{read_verified_node_with, source_proposal::take, BaseCstAdmissionLimits, Result};
use std::{collections::BTreeSet, path::Path};
use surrealkv::Transaction;
use wosy_project::packages::InstalledGitBase;
use wosy_protocol::{source_preparation_parts, FileEntry, Node, SourceConfig};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledGitScopeRejection {
    Ownership,
    Selection,
    Files,
}

impl std::fmt::Display for InstalledGitScopeRejection {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::Ownership => "installed Git witness belongs to a different workspace",
            Self::Selection => "rooted source does not select the witnessed installed Git base",
            Self::Files => "installed Git witness differs from the exact selected file subset",
        })
    }
}
impl std::error::Error for InstalledGitScopeRejection {}

pub(super) fn verify(
    tx: &Transaction, top: &Node, config: &SourceConfig, base: &InstalledGitBase,
    limits: BaseCstAdmissionLimits,
) -> Result<()> {
    let namespace = config.namespaces.get(base.package()).ok_or(InstalledGitScopeRejection::Selection)?;
    let git = namespace.git.as_ref().ok_or(InstalledGitScopeRejection::Selection)?;
    if &git.locked != base.locked() || git.patches != base.patches()
        || Path::new(&namespace.project) != base.checkout_path()
    {
        return Err(InstalledGitScopeRejection::Selection.into());
    }
    let expected = base.visited_files();
    if expected.is_empty() { return Err(InstalledGitScopeRejection::Files.into()); }
    let files = source_preparation_parts(top).ok_or("missing selected file index")?.files;
    if files.len() > limits.records { return Err("installed Git source index exceeds the record budget".into()); }
    let mut remaining = limits.record_bytes;
    let parser = config.parser.id();
    let mut matched = BTreeSet::new();
    for &id in files {
        let node = read_verified_node_with(tx, id, |bytes| {
            take(&mut remaining, bytes, "installed Git source index exceeds the byte budget")
        })?;
        let file = FileEntry::from_node(id, node)?;
        let source = file.metadata.source;
        if source.package != base.package() { continue; }
        if source.project != namespace.project || file.metadata.policy != parser
            || expected.get(&source.path) != Some(&file.metadata.revision.0)
            || !matched.insert(source.path)
        {
            return Err(InstalledGitScopeRejection::Files.into());
        }
    }
    if matched.len() != expected.len() { return Err(InstalledGitScopeRejection::Files.into()); }
    Ok(())
}
