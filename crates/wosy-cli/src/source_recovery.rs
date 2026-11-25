//! Retain one selected source generation in a live client. No workspace
//! persistence, derived compiler cache, or transport is owned here.
use crate::source_preparation::{prepare_with_files, Files, PreparedSources};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use wosy_protocol::{Delta, Node, NodeId, SourceUpdate};

/// Bounds one selected generation's canonical payload, not the parser, file
/// index, candidate staging, allocator overhead, or total process RSS.
#[derive(Clone, Copy)]
pub struct ClientSourceLimits {
    pub nodes: usize,
    pub encoded_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientSourceUsage {
    pub nodes: usize,
    pub encoded_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceLimit {
    Nodes,
    EncodedBytes,
}

#[derive(Debug)]
pub enum ClientSourceError {
    Observe(String),
    InvalidGeneration(&'static str),
    LimitExceeded { field: SourceLimit, limit: usize },
}

impl fmt::Display for ClientSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Observe(message) => f.write_str(message),
            Self::InvalidGeneration(message) => f.write_str(message),
            Self::LimitExceeded { field, limit } => {
                write!(f, "retained source exceeds {field:?} limit {limit}")
            }
        }
    }
}

impl std::error::Error for ClientSourceError {}

pub struct RetainedSources {
    current: PreparedSources,
    files: Files,
    usage: ClientSourceUsage,
    limits: ClientSourceLimits,
}

/// A staged selection is private until committed. Dropping it releases the
/// candidate without changing the client's selected root, bytes or file index.
pub struct StagedSourceRefresh<'a> {
    owner: &'a mut RetainedSources,
    data: NextSelection,
}

enum NextSelection {
    Same { parsed: usize, reused: usize },
    Replace { source: PreparedSources, files: Files, closure: Closure },
}

impl StagedSourceRefresh<'_> {
    pub(crate) fn observation_counts(&self) -> (usize, usize) {
        match &self.data {
            NextSelection::Same { parsed, reused } => (*parsed, *reused),
            NextSelection::Replace { source, .. } => (source.parsed, source.reused),
        }
    }

    pub fn root(&self) -> NodeId {
        match &self.data {
            NextSelection::Same { .. } => self.owner.current.root,
            NextSelection::Replace { source, .. } => source.root,
        }
    }

    pub fn usage(&self) -> ClientSourceUsage {
        match &self.data {
            NextSelection::Same { .. } => self.owner.usage,
            NextSelection::Replace { closure, .. } => closure.usage(),
        }
    }

    /// Borrow the exact added nodes and the immutable recovery base without
    /// publishing or replacing this client's selected snapshot.
    pub fn submission(&self) -> SourceUpdate<'_> {
        let added = match &self.data {
            NextSelection::Same { .. } => None,
            NextSelection::Replace { source, .. } => Some(&source.delta),
        };
        SourceUpdate {
            base: self.owner.current.root, root: self.root(), added,
            base_payload: &self.owner.current.delta,
        }
    }

    pub fn commit(self) {
        let Self { owner, data } = self;
        match data {
            NextSelection::Same { parsed, reused } => {
                owner.current.parsed = parsed;
                owner.current.reused = reused;
            }
            NextSelection::Replace { mut source, files, closure } => {
                owner.current.delta.nodes.retain(|id, _| closure.nodes.contains(id));
                source.delta.nodes.append(&mut owner.current.delta.nodes);
                owner.current = source;
                owner.files = files;
                owner.usage = closure.usage();
            }
        }
    }
}

impl RetainedSources {
    pub(crate) fn from_persisted(
        current: PreparedSources, files: Files, usage: ClientSourceUsage, limits: ClientSourceLimits,
    ) -> Self {
        Self { current, files, usage, limits }
    }

    pub fn observe(
        project: &Path, target: Option<&str>, limits: ClientSourceLimits,
    ) -> Result<Self, ClientSourceError> {
        let (mut current, files) = prepare_with_files(project, target, None).map_err(ClientSourceError::Observe)?;
        let closure = reachable(current.root, &current.delta, None, limits)?;
        current.delta.nodes.retain(|id, _| closure.nodes.contains(id));
        Ok(Self { current, files, usage: closure.usage(), limits })
    }

    pub fn refresh(&mut self, project: &Path, target: Option<&str>) -> Result<(), ClientSourceError> {
        self.stage_refresh(project, target)?.commit();
        Ok(())
    }

    pub fn stage_refresh(
        &mut self, project: &Path, target: Option<&str>,
    ) -> Result<StagedSourceRefresh<'_>, ClientSourceError> {
        let (mut source, files) = prepare_with_files(project, target, Some(&self.files)).map_err(ClientSourceError::Observe)?;
        let data = if self.current.root == source.root {
            NextSelection::Same { parsed: source.parsed, reused: source.reused }
        } else {
            let closure = reachable(source.root, &source.delta, Some(&self.current.delta), self.limits)?;
            source.delta.nodes.retain(|id, _| {
                closure.nodes.contains(id) && !self.current.delta.nodes.contains_key(id)
            });
            NextSelection::Replace { source, files, closure }
        };
        Ok(StagedSourceRefresh { owner: self, data })
    }

    /// A complete source submission, including unchanged input nodes. The
    /// ordinary atomic publish accepts it whether this root is known or lost.
    /// Obtaining it does not read current workspace files or run the parser.
    pub fn selected(&self) -> &PreparedSources {
        &self.current
    }

    pub fn usage(&self) -> ClientSourceUsage {
        self.usage
    }
}

pub(crate) struct Closure {
    nodes: HashSet<NodeId>,
    encoded_bytes: usize,
}

impl Closure {
    pub(crate) fn usage(&self) -> ClientSourceUsage {
        ClientSourceUsage { nodes: self.nodes.len(), encoded_bytes: self.encoded_bytes }
    }
}

fn reachable(
    root: NodeId, delta: &Delta, base: Option<&Delta>, limits: ClientSourceLimits,
) -> Result<Closure, ClientSourceError> {
    trace_reachable(root, limits, |id| {
        delta.nodes.get(&id).or_else(|| base.and_then(|base| base.nodes.get(&id)))
            .ok_or(ClientSourceError::InvalidGeneration("client source generation is not closed"))
    }, |_, _| ())
}

pub(crate) fn trace_reachable<B, E>(
    root: NodeId, limits: ClientSourceLimits,
    mut fetch: impl FnMut(NodeId) -> Result<B, E>,
    mut keep: impl FnMut(NodeId, B),
) -> Result<Closure, E>
where
    B: AsRef<[u8]>,
    E: From<ClientSourceError>,
{
    use ClientSourceError::{InvalidGeneration, LimitExceeded};
    let nodes_over = || LimitExceeded { field: SourceLimit::Nodes, limit: limits.nodes };
    let bytes_over = || LimitExceeded { field: SourceLimit::EncodedBytes, limit: limits.encoded_bytes };
    if limits.nodes == 0 { return Err(nodes_over().into()); }
    let mut closure = Closure { nodes: HashSet::from([root]), encoded_bytes: 0 };
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        let bytes = fetch(id)?;
        closure.encoded_bytes = closure.encoded_bytes.checked_add(bytes.as_ref().len())
            .filter(|&total| total <= limits.encoded_bytes).ok_or_else(bytes_over)?;
        for child in Node::child_ids(bytes.as_ref()).ok_or(InvalidGeneration("invalid client source generation node"))? {
            if closure.nodes.insert(child) {
                if closure.nodes.len() > limits.nodes { return Err(nodes_over().into()); }
                pending.push(child);
            }
        }
        keep(id, bytes);
    }
    Ok(closure)
}
