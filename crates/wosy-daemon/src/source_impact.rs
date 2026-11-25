//! A rooted source dependency comparison, not a semantic invalidation verdict.
use super::{key, read_index_charged, read_node, source_top, Edge, Index, Result, SourceStore, EDGES_IN};
use std::collections::{BTreeSet, HashMap, HashSet};
use surrealkv::{LSMIterator, Transaction};
use wosy_protocol::{source_preparation_parts, FileKey, NodeId, SourceConfig};

#[derive(Clone, Copy)]
pub struct SourceImpactLimits {
    /// File entries across both rooted indexes.
    pub root_files: usize,
    /// Ordered forward import occurrences and reverse rows physically visited.
    /// Not a memory or elapsed-time limit.
    pub adjacency_rows: usize,
}

pub struct SourceFileChange {
    pub file: FileKey,
    pub before: Option<NodeId>,
    pub after: Option<NodeId>,
    pub executable_changed: bool,
    pub annotation_changed: bool,
    pub source_owner_changed: bool,
    pub imports_changed: bool,
}

pub struct SourceImpact {
    pub configuration_changed: bool,
    pub executable_tree_changed: bool,
    pub annotation_tree_changed: bool,
    pub files: Vec<SourceFileChange>,
    /// Direct executable/owner/import changes and transitive importing files, scoped
    /// to the selected old and new roots. Config changes may require additional
    /// analysis; these are neither Build/Check results nor reuse permission.
    pub before_candidates: Vec<FileKey>,
    pub after_candidates: Vec<FileKey>,
    pub forward_occurrences: usize,
    pub selected_reverse_rows: usize,
}

impl SourceStore {
    /// Compare two admitted source preparations from one workspace and target.
    /// Both remain registered as active readers for this one consistent cut.
    pub fn source_impact(
        &self, before: NodeId, after: NodeId, limits: SourceImpactLimits,
    ) -> Result<Option<SourceImpact>> {
        let Some((tx, _reads)) = self.begin_source_reads([before, after])? else { return Ok(None); };
        let (Some(old), Some(new)) = (source_top(&tx, before)?, source_top(&tx, after)?) else {
            return Ok(None);
        };
        let a = source_preparation_parts(&old).ok_or("unsupported old source preparation")?;
        let b = source_preparation_parts(&new).ok_or("unsupported new source preparation")?;
        let configuration_changed = a.config != b.config;
        if configuration_changed && !same_scope(&tx, a.config, b.config)? {
            return Err("source impact roots must select the same workspace and target".into());
        }
        let count = a.files.len().checked_add(b.files.len()).ok_or("source impact file budget exceeded")?;
        if count > limits.root_files { return Err("source impact file budget exceeded".into()); }

        let mut remaining = limits.adjacency_rows;
        let mut charge = |count| charge_rows(&mut remaining, count);
        let old = read_index_charged(&tx, old, &mut charge)?;
        let new = read_index_charged(&tx, new, &mut charge)?;
        let forward_occurrences = limits.adjacency_rows - remaining;
        let old_paths = module_paths(&old);
        let new_paths = module_paths(&new);
        let paths: BTreeSet<_> = old.files.keys().chain(new.files.keys()).collect();
        let (mut files, mut old_seeds, mut new_seeds) = (Vec::new(), Vec::new(), Vec::new());
        for file in paths {
            let previous = old.files.get(file);
            let next = new.files.get(file);
            let before = previous.map(|entry| entry.file);
            let after = next.map(|entry| entry.file);
            let executable_changed = previous.map(|entry| entry.executable) != next.map(|entry| entry.executable);
            let annotation_changed = previous.map(|entry| entry.annotation) != next.map(|entry| entry.annotation);
            let source_owner_changed = previous.zip(next).is_some_and(|(a, b)| a.metadata.source != b.metadata.source);
            // A resolved destination is its authored source identity, not
            // the content-dependent module hash nor just the relative path.
            let prior_imports = old.module_edges.get(file).into_iter().flatten()
                .map(|id| &old.files[old_paths[id]].metadata.source);
            let next_imports = new.module_edges.get(file).into_iter().flatten()
                .map(|id| &new.files[new_paths[id]].metadata.source);
            let imports_changed = prior_imports.ne(next_imports);
            if before == after && !imports_changed { continue; }
            if executable_changed || source_owner_changed || imports_changed {
                old_seeds.extend(old.modules.get(file).copied());
                new_seeds.extend(new.modules.get(file).copied());
            }
            files.push(SourceFileChange {
                file: file.clone(), before, after, executable_changed, annotation_changed,
                source_owner_changed, imports_changed,
            });
        }
        let (before_candidates, before_rows) = reverse_imports(&tx, &old_paths, old_seeds, &mut remaining)?;
        let (after_candidates, after_rows) = reverse_imports(&tx, &new_paths, new_seeds, &mut remaining)?;
        Ok(Some(SourceImpact {
            configuration_changed, executable_tree_changed: old.executable != new.executable,
            annotation_tree_changed: old.annotation != new.annotation, files, before_candidates, after_candidates,
            forward_occurrences, selected_reverse_rows: before_rows + after_rows,
        }))
    }
}

fn charge_rows(remaining: &mut usize, count: usize) -> Result<()> {
    *remaining = remaining.checked_sub(count).ok_or("source impact adjacency budget exceeded")?;
    Ok(())
}

fn same_scope(tx: &Transaction, old: NodeId, new: NodeId) -> Result<bool> {
    let old: SourceConfig = serde_json::from_slice(&read_node(tx, old)?.data)?;
    let new: SourceConfig = serde_json::from_slice(&read_node(tx, new)?.data)?;
    Ok(old.target == new.target
        && old.namespaces.get(&old.entry.package).ok_or("missing old source owner")?.project
            == new.namespaces.get(&new.entry.package).ok_or("missing new source owner")?.project)
}

fn module_paths(index: &Index) -> HashMap<NodeId, &FileKey> {
    index.modules.iter().map(|(path, id)| (*id, path)).collect()
}

fn reverse_imports(
    tx: &Transaction, selected: &HashMap<NodeId, &FileKey>, mut pending: Vec<NodeId>, remaining: &mut usize,
) -> Result<(Vec<FileKey>, usize)> {
    let mut visited = HashSet::new();
    let mut paths = BTreeSet::new();
    let mut selected_rows = 0;
    while let Some(child) = pending.pop() {
        if !visited.insert(child) { continue; }
        paths.insert(selected[&child].clone());
        // (to, Imports, from); other roots can share a child. Only parents
        // admitted by this selected root's typed index may enter the walk.
        let mut lower = [0; 34];
        lower[..33].copy_from_slice(&key(EDGES_IN, child));
        lower[33] = Edge::Imports as u8;
        let mut upper = lower;
        upper[33] += 1;
        let mut rows = tx.range(lower.as_slice(), upper.as_slice())?;
        rows.seek_first()?;
        while rows.valid() {
            charge_rows(remaining, 1)?;
            let row = rows.key().user_key();
            let parent = NodeId(row.get(34..).ok_or("invalid incoming source edge")?.try_into()?);
            if selected.contains_key(&parent) {
                selected_rows += 1;
                if !visited.contains(&parent) { pending.push(parent); }
            }
            rows.next()?;
        }
    }
    Ok((paths.into_iter().collect(), selected_rows))
}
