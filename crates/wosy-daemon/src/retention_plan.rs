//! Read-only reclamation census for explicit, fixture-owned source revisions.
use super::{
    authority_key, base_declarations, pins, project_declarations, published_source, read_node,
    source_top, supported_top, Result, SourceStore, FILE_AUTHORITY, SNAPSHOT_ROOTS,
};
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use surrealkv::{LSMIterator, Mode, Transaction};
use wosy_protocol::{Kind, NodeId};

const PROTECTED: u8 = 1;
const CANDIDATE: u8 = 2;
const SHARED: u8 = PROTECTED | CANDIDATE;

pub struct SourceRetentionPlan {
    /// Exact logical protection inputs observed in a single MVCC transaction.
    /// This is not an authorization or token for a subsequent sweep.
    pub protection_epoch: NodeId,
    pub nominated_roots: usize,
    pub protected_roots: usize,
    pub candidate_roots: usize,
    pub nodes_visited: usize,
    pub shared_nodes: usize,
    pub candidate_only_nodes: usize,
    /// Encoded node values only, not disk bytes or a compaction prediction.
    pub candidate_only_node_bytes: usize,
    /// Optional payload identities referenced only by candidate closures.
    /// Actual files and sizes are not part of this MVCC census.
    pub candidate_only_cas_references: usize,
}

struct Mark {
    classes: u8,
    encoded_bytes: usize,
}

#[derive(Default)]
struct Marks {
    nodes: HashMap<NodeId, Mark>,
    cas: HashMap<NodeId, u8>,
}

fn walk(
    tx: &Transaction, roots: impl IntoIterator<Item = NodeId>, class: u8,
    limit: usize, marks: &mut Marks,
) -> Result<()> {
    let mut todo: Vec<_> = roots.into_iter().collect();
    while let Some(id) = todo.pop() {
        let at_limit = marks.nodes.len() >= limit;
        let entry = marks.nodes.entry(id);
        match &entry {
            Entry::Occupied(existing) if existing.get().classes & class != 0 => continue,
            Entry::Vacant(_) if at_limit => return Err("source retention plan exceeded node budget".into()),
            _ => {}
        }
        let node = read_node(tx, id)?;
        entry.or_insert_with(|| Mark { classes: 0, encoded_bytes: node.encoded_len() }).classes |= class;

        match node.kind {
            // Other source schemas may have different auxiliary edges.
            Kind::SourcePreparation if !supported_top(&node) => {
                return Err("unsupported published root in retention census".into());
            }
            Kind::File => {
                let authority = tx.get(&authority_key(FILE_AUTHORITY, id))?
                    .ok_or("missing namespace authority for source retention")?;
                todo.push(decode_id(authority.as_ref())?);
                if let Some(body) = tx.get(&base_declarations::query_key(node.children[1]))? {
                    todo.push(decode_id(body.as_ref())?);
                }
            }
            Kind::Module => {
                if let Some(context) = tx.get(&project_declarations::projection_key(id))? {
                    todo.push(decode_id(context.as_ref())?);
                }
            }
            Kind::BaseDeclarations => {
                if let Some(blob) = base_declarations::cas_digest(&node.data)? {
                    *marks.cas.entry(blob).or_default() |= class;
                }
            }
            _ => {}
        }
        todo.extend(node.children);
    }
    Ok(())
}

fn decode_id(bytes: &[u8]) -> Result<NodeId> {
    Ok(NodeId(bytes.try_into()?))
}

fn rows(
    tx: &Transaction, tag: u8, mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    let mut iter = tx.range([tag].as_slice(), [tag + 1].as_slice())?;
    iter.seek_first()?;
    while iter.valid() {
        visit(iter.key().user_key(), iter.value()?.as_ref())?;
        iter.next()?;
    }
    Ok(())
}

fn framed_hash(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_be_bytes()).update(bytes);
}

impl SourceStore {
    /// Conservative, direct fixture inspection. All other published roots are
    /// protected by default. The supplied keep list adds fixture read roots;
    /// this is not a substitute for production leases or a retention policy.
    pub fn plan_source_retention(
        &self, scope: NodeId, nominated: &[NodeId], keep: &[NodeId], limit: usize,
    ) -> Result<SourceRetentionPlan> {
        let tx = self.tree.begin_with_mode(Mode::ReadOnly)?;
        let scope_top = published_source(&tx, scope)?.ok_or("unknown source retention scope")?;
        let owner = pins::workspace_of_config(&tx, scope_top.children[0])?;
        if nominated.len().max(keep.len()) > limit {
            return Err("source retention selection exceeded root budget".into());
        }
        let nominated: BTreeSet<_> = nominated.iter().copied().collect();
        let keep: BTreeSet<_> = keep.iter().copied().chain([scope]).collect();

        let mut epoch = blake3::Hasher::new_derive_key("wosy source protection census v1");
        epoch.update(&scope.0);
        for selection in [&nominated, &keep] {
            epoch.update(&(selection.len() as u64).to_be_bytes());
            for id in selection { epoch.update(&id.0); }
        }

        let mut published = BTreeSet::new();
        rows(&tx, SNAPSHOT_ROOTS, |catalog_key, value| {
            let root = decode_id(&catalog_key[1..])?;
            if value != root.0 { return Err("incomplete published source marker".into()); }
            if published.len() >= limit { return Err("source retention plan exceeded root budget".into()); }
            published.insert(root);
            framed_hash(&mut epoch, catalog_key);
            framed_hash(&mut epoch, value);
            Ok(())
        })?;
        if !keep.is_subset(&published) || !nominated.is_subset(&published) {
            return Err("retention selection names an unpublished root".into());
        }

        let mut pinned = BTreeSet::new();
        let mut pin_count = 0;
        rows(&tx, pins::PINS, |pin_key, value| {
            if pin_count >= limit { return Err("source retention plan exceeded pin budget".into()); }
            pin_count += 1;
            let (root, _) = pins::decode_pin_value(value)?;
            if !published.contains(&root) { return Err("durable pin is outside published roots".into()); }
            pinned.insert(root);
            framed_hash(&mut epoch, pin_key);
            framed_hash(&mut epoch, value);
            Ok(())
        })?;

        let mut candidates = BTreeSet::new();
        for &root in &nominated {
            let top = source_top(&tx, root)?.ok_or("unsupported retention candidate")?;
            if pins::workspace_of_config(&tx, top.children[0])? != owner {
                return Err("retention candidate belongs to another workspace".into());
            }
            if !keep.contains(&root) && !pinned.contains(&root) {
                candidates.insert(root);
            }
        }

        let mut marks = Marks::default();
        walk(&tx, published.difference(&candidates).copied(), PROTECTED, limit, &mut marks)?;
        walk(&tx, candidates.iter().copied(), CANDIDATE, limit, &mut marks)?;

        let mut report = SourceRetentionPlan {
            protection_epoch: NodeId(*epoch.finalize().as_bytes()),
            nominated_roots: nominated.len(),
            protected_roots: published.len() - candidates.len(),
            candidate_roots: candidates.len(),
            nodes_visited: marks.nodes.len(),
            shared_nodes: 0,
            candidate_only_nodes: 0,
            candidate_only_node_bytes: 0,
            candidate_only_cas_references: marks.cas.values().filter(|&&class| class == CANDIDATE).count(),
        };
        for mark in marks.nodes.values() {
            match mark.classes {
                SHARED => report.shared_nodes += 1,
                CANDIDATE => {
                    report.candidate_only_nodes += 1;
                    report.candidate_only_node_bytes += mark.encoded_bytes;
                }
                _ => {}
            }
        }
        Ok(report)
    }
}
