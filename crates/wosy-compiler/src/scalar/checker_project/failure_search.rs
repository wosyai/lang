//! Run-local authored path budget. No values form part of structural identity.
use std::{collections::HashSet, num::NonZeroUsize};
use wosy_syntax::{SourceIdentity, SourceSpan};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum PathStep {
    Startup(SourceIdentity),
    Branch { condition: SourceSpan, accepted: bool },
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct FailurePath {
    steps: Vec<PathStep>,
    directive: SourceSpan,
}

pub(super) enum Admission { Duplicate, Selected, SelectedAndStop }

pub(super) struct StartUpSearch {
    remaining: usize,
    prefix: Vec<PathStep>,
    selected: HashSet<FailurePath>,
    work: usize,
}

impl StartUpSearch {
    pub(super) fn new(root: &SourceIdentity, cap: NonZeroUsize) -> Self {
        Self { remaining: cap.get(), prefix: vec![PathStep::Startup(root.clone())], selected: HashSet::new(), work: 0 }
    }

    pub(super) fn branch(&mut self, condition: SourceSpan, accepted: bool) -> Option<()> {
        super::super::checker_flow::charge_call_work(&mut self.work, 1)?;
        self.prefix.push(PathStep::Branch { condition, accepted });
        Some(())
    }

    pub(super) fn admit(&mut self, directive: SourceSpan) -> Option<Admission> {
        if self.remaining == 0 { return None; }
        super::super::checker_flow::charge_call_work(&mut self.work, self.prefix.len().checked_add(1)?)?;
        let path = FailurePath { steps: self.prefix.clone(), directive };
        if !self.selected.insert(path) { return Some(Admission::Duplicate); }
        self.remaining -= 1;
        Some(if self.remaining == 0 { Admission::SelectedAndStop } else { Admission::Selected })
    }

    pub(super) fn remaining(&self) -> usize { self.remaining }

    #[cfg(test)]
    pub(super) fn contains_branch(&self, condition: SourceSpan, accepted: bool) -> bool {
        self.selected.iter().any(|path| path.steps.contains(&PathStep::Branch { condition: condition.clone(), accepted }))
    }
}
