//! Canonical identities for observed compile-time results. Computing an
//! identity does not authenticate execution or admit it to a final snapshot.

use crate::NodeId;
use serde::Serialize;

/// Signed and unsigned integers are kept distinct even when their bits
/// describe the same number. Widths are supplied by the resolved type.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComptimeScalarValue {
    Bool(bool),
    Signed { width: u16, value: i64 },
    Unsigned { width: u16, value: u64 },
}

/// Type identity comes from the compiler's resolved type, not stdout syntax.
/// Child order is compiler field/index order, including for single-field types.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ComptimeValue {
    Scalar(ComptimeScalarValue),
    Aggregate { resolved_type: NodeId, children: Vec<ComptimeValue> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComptimeWatchIdentity {
    /// Resolved package/project-relative watch identity, not just basename.
    pub input: NodeId,
    pub content: NodeId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ComptimeInputs {
    pub fragment_ir: NodeId,
    pub function: NodeId,
    pub specialization: NodeId,
    pub concrete_values: Vec<ComptimeValue>,
    pub target: String,
    pub profile: String,
    pub compiler_image: NodeId,
    pub builder: NodeId,
    pub runner: NodeId,
    pub toolchain: NodeId,
    pub params: NodeId,
    pub permissions: NodeId,
    pub watches: Vec<ComptimeWatchIdentity>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComptimeKey { identity: NodeId, scope: NodeId }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComptimeResult {
    key: ComptimeKey,
    identity: NodeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComptimeResultRoot(NodeId);

fn identify(domain: &str, value: &impl Serialize) -> Result<NodeId, serde_json::Error> {
    let bytes = serde_json::to_vec(value)?;
    let mut hash = blake3::Hasher::new_derive_key(domain);
    hash.update(&bytes);
    Ok(NodeId(*hash.finalize().as_bytes()))
}

impl ComptimeInputs {
    pub fn key(mut self) -> Result<ComptimeKey, ComptimeIdentityError> {
        self.watches.sort_unstable_by_key(|watch| watch.input);
        self.watches.dedup();
        if self.watches.windows(2).any(|pair| pair[0].input == pair[1].input) {
            return Err(ComptimeIdentityError::DuplicateWatch);
        }
        Ok(ComptimeKey {
            identity: identify("Wosy comptime inputs v1", &self)?,
            scope: identify("Wosy comptime target and profile v1", &(&self.target, &self.profile))?,
        })
    }
}

impl ComptimeKey {
    pub fn identity(self) -> NodeId { self.identity }

    pub fn identify_result(self, resolved_type: NodeId, value: &ComptimeValue) -> Result<ComptimeResult, ComptimeIdentityError> {
        let identity = identify("Wosy comptime result v1", &(self.identity, resolved_type, value))?;
        Ok(ComptimeResult { key: self, identity })
    }
}

impl ComptimeResult {
    pub fn identity(self) -> NodeId { self.identity }
}

impl ComptimeResultRoot {
    /// Frozen set of actual evaluator observations for one prepared source
    /// revision. A conflicting result for one exact specialization is not
    /// silently selected by evaluation order.
    pub fn identify(
        prepared_source: NodeId, target: &str, profile: &str,
        results: impl IntoIterator<Item = ComptimeResult>,
    ) -> Result<Self, ComptimeIdentityError> {
        let members = checked_result_members(target, profile, results)?;
        Ok(Self(identify("Wosy comptime result root v1", &(prepared_source, target, profile, members))?))
    }

    /// Return the active root and the comparable member identity from
    /// the same single validated set. The latter is not a result root.
    pub fn identify_with_members(
        prepared_source: NodeId, target: &str, profile: &str,
        results: impl IntoIterator<Item = ComptimeResult>,
    ) -> Result<(Self, NodeId), ComptimeIdentityError> {
        let members = checked_result_members(target, profile, results)?;
        let root = Self(identify("Wosy comptime result root v1", &(prepared_source, target, profile, &members))?);
        let comparable = identify("Wosy comptime result members v1", &(target, profile, members))?;
        Ok((root, comparable))
    }

    pub fn identity(self) -> NodeId { self.0 }
}

fn checked_result_members(
    target: &str, profile: &str, results: impl IntoIterator<Item = ComptimeResult>,
) -> Result<Vec<(NodeId, NodeId)>, ComptimeIdentityError> {
    let scope = identify("Wosy comptime target and profile v1", &(target, profile))?;
    let mut members = Vec::new();
    for result in results {
        if result.key.scope != scope {
            return Err(ComptimeIdentityError::ForeignTargetOrProfile);
        }
        members.push((result.key.identity, result.identity));
    }
    members.sort_unstable();
    members.dedup();
    if members.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(ComptimeIdentityError::ConflictingResult);
    }
    Ok(members)
}

#[derive(Debug)]
pub enum ComptimeIdentityError {
    Encoding(serde_json::Error),
    DuplicateWatch,
    ConflictingResult,
    ForeignTargetOrProfile,
}

impl std::fmt::Display for ComptimeIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encoding(error) => error.fmt(formatter),
            Self::DuplicateWatch => formatter.write_str("one comptime watch identity has conflicting content"),
            Self::ConflictingResult => formatter.write_str("one comptime specialization has conflicting values"),
            Self::ForeignTargetOrProfile => formatter.write_str("comptime result belongs to a different target or profile"),
        }
    }
}

impl std::error::Error for ComptimeIdentityError {}

impl From<serde_json::Error> for ComptimeIdentityError {
    fn from(error: serde_json::Error) -> Self { Self::Encoding(error) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(i: u8) -> NodeId { NodeId([i; 32]) }

    fn inputs() -> ComptimeInputs {
        ComptimeInputs {
            fragment_ir: id(1), function: id(2), specialization: id(3),
            concrete_values: vec![ComptimeValue::Scalar(ComptimeScalarValue::Bool(true))],
            target: "app".into(), profile: "release".into(), compiler_image: id(4),
            builder: id(5), runner: id(6), toolchain: id(7), params: id(8), permissions: id(9),
            watches: vec![
                ComptimeWatchIdentity { input: id(10), content: id(11) },
                ComptimeWatchIdentity { input: id(12), content: id(13) },
            ],
        }
    }

    #[test]
    fn exact_watch_and_tool_inputs_are_in_the_key() {
        let first = inputs().key().unwrap();
        let mut reordered = inputs();
        reordered.watches.reverse();
        reordered.watches.push(reordered.watches[0].clone());
        assert_eq!(first, reordered.key().unwrap());
        let mut changed = inputs();
        changed.watches[0].content = id(14);
        assert_ne!(first, changed.key().unwrap());
        let mut changed = inputs();
        changed.runner = id(14);
        assert_ne!(first, changed.key().unwrap());
        let mut conflict = inputs();
        conflict.watches.push(ComptimeWatchIdentity { input: id(10), content: id(14) });
        assert!(matches!(conflict.key(), Err(ComptimeIdentityError::DuplicateWatch)));
    }

    #[test]
    fn result_identity_preserves_aggregate_type_and_value() {
        let key = inputs().key().unwrap();
        let single = |value| ComptimeValue::Aggregate {
            resolved_type: id(15),
            children: vec![ComptimeValue::Scalar(ComptimeScalarValue::Signed { width: 32, value })],
        };
        let first = key.identify_result(id(15), &single(42)).unwrap();
        let changed = key.identify_result(id(15), &single(43)).unwrap();
        let retyped = key.identify_result(id(16), &single(42)).unwrap();
        assert_ne!(first, retyped);
        assert_ne!(first, changed);
        let root = |source, members| ComptimeResultRoot::identify(source, "app", "release", members);
        assert_ne!(root(id(18), vec![first]).unwrap(), root(id(19), vec![first]).unwrap());
        assert_eq!(root(id(18), vec![first]).unwrap(), root(id(18), vec![first, first]).unwrap());
        assert_ne!(root(id(18), vec![first]).unwrap(), root(id(18), vec![changed]).unwrap());
        assert!(matches!(root(id(18), vec![first, changed]), Err(ComptimeIdentityError::ConflictingResult)));
        assert!(matches!(ComptimeResultRoot::identify(id(18), "other", "release", [first]),
            Err(ComptimeIdentityError::ForeignTargetOrProfile)));
    }
}
