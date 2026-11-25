//! Retain one selected source generation in a live client. No workspace
//! persistence, derived compiler cache, or transport is owned here.
use crate::source_preparation::{prepare_with_files, Files, PreparedSources};
use std::collections::HashSet;
use std::path::Path;
use wosy_protocol::{Delta, Node, NodeId};

pub struct RetainedSources {
    current: PreparedSources,
    files: Files,
}

impl RetainedSources {
    pub fn observe(project: &Path, target: Option<&str>) -> Result<Self, String> {
        let (mut current, files) = prepare_with_files(project, target, None)?;
        let closure = reachable(current.root, &current.delta, None)?;
        current.delta.nodes.retain(|id, _| closure.contains(id));
        Ok(Self { current, files })
    }

    pub fn refresh(&mut self, project: &Path, target: Option<&str>) -> Result<(), String> {
        let (mut next, files) = prepare_with_files(project, target, Some(&self.files))?;
        if self.current.root == next.root {
            self.current.parsed = next.parsed;
            self.current.reused = next.reused;
            return Ok(());
        }
        let closure = reachable(next.root, &next.delta, Some(&self.current.delta))?;
        // Do not invalidate the selected root until the new closure is known.
        // Move retained byte buffers; prior generations do not accumulate.
        self.current.delta.nodes.retain(|id, _| closure.contains(id) && !next.delta.nodes.contains_key(id));
        next.delta.nodes.retain(|id, _| closure.contains(id));
        next.delta.nodes.append(&mut self.current.delta.nodes);
        self.current = next;
        self.files = files;
        Ok(())
    }

    /// A complete source submission, including unchanged input nodes. The
    /// ordinary atomic publish accepts it whether this root is known or lost.
    /// Obtaining it does not read current workspace files or run the parser.
    pub fn selected(&self) -> &PreparedSources {
        &self.current
    }
}

fn reachable(root: NodeId, delta: &Delta, base: Option<&Delta>) -> Result<HashSet<NodeId>, String> {
    let mut closure = HashSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !closure.insert(id) { continue; }
        let bytes = delta.nodes.get(&id).or_else(|| base.and_then(|base| base.nodes.get(&id)))
            .ok_or("client source generation is not closed")?;
        pending.extend(Node::child_ids(bytes).ok_or("invalid client source generation node")?);
    }
    Ok(closure)
}
