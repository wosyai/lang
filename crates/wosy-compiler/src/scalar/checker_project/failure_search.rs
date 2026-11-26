//! Run-local authored path budget. No values form part of structural identity.
use std::{collections::HashSet, num::NonZeroUsize};
use wosy_syntax::{SourceIdentity, SourceSpan};
use num_bigint::BigInt;
use super::{PreparedCallable, ScalarType};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in super::super) enum PathStep {
    Startup(SourceIdentity),
    CallableRoot(SourceSpan),
    Namespace { binding: SourceSpan, target: SourceIdentity },
    Call(SourceSpan),
    Return(SourceSpan),
    Output(SourceSpan),
    Branch { condition: SourceSpan, accepted: bool },
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct FailurePath {
    steps: Vec<PathStep>,
    directive: SourceSpan,
}

pub(super) enum Admission { Duplicate, Selected, SelectedAndStop }

pub(super) struct StartupNamedObservation {
    pub(super) site: SourceSpan,
    pub(super) directive_position: usize,
    pub(super) parameter: ScalarType,
    pub(super) argument: BigInt,
    pub(super) returned: BigInt,
}

pub(in super::super) struct StartUpSearch {
    remaining: usize,
    prefix: Vec<PathStep>,
    selected: HashSet<FailurePath>,
    work: usize,
    invalid: bool,
    pub(super) named_target: Option<PreparedCallable>,
    pub(super) named_cross_source: bool,
    pub(super) named_observation: Option<StartupNamedObservation>,
}

impl StartUpSearch {
    pub(super) fn new(root: &SourceIdentity, cap: NonZeroUsize) -> Self {
        Self { remaining: cap.get(), prefix: vec![PathStep::Startup(root.clone())], selected: HashSet::new(), work: 0, invalid: false, named_target: None, named_cross_source: false, named_observation: None }
    }

    pub(in super::super) fn branch(&mut self, condition: SourceSpan, accepted: bool) -> Option<()> {
        self.append(PathStep::Branch { condition, accepted })
    }

    pub(in super::super) fn append(&mut self, step: PathStep) -> Option<()> {
        super::super::checker_flow::charge_call_work(&mut self.work, 1)?;
        self.prefix.push(step);
        Some(())
    }

    pub(super) fn admit(&mut self, directive: SourceSpan) -> Option<Admission> {
        super::super::checker_flow::charge_call_work(&mut self.work, self.prefix.len().checked_add(1)?)?;
        self.select(FailurePath { steps: self.prefix.clone(), directive })
    }

    pub(super) fn admit_callable(
        &mut self, declaration: SourceSpan, history: Vec<(wosy_syntax::ByteSpan, bool)>, directive: SourceSpan,
    ) -> Option<Admission> {
        super::super::checker_flow::charge_call_work(&mut self.work, history.len().checked_add(2)?)?;
        let mut steps = Vec::with_capacity(history.len().checked_add(1)?);
        steps.push(PathStep::CallableRoot(declaration));
        steps.extend(history.into_iter().map(|(range, accepted)| PathStep::Branch {
            condition: SourceSpan::new(directive.source.clone(), range), accepted,
        }));
        self.select(FailurePath { steps, directive })
    }

    fn select(&mut self, path: FailurePath) -> Option<Admission> {
        if self.remaining == 0 { return None; }
        if !self.selected.insert(path) { return Some(Admission::Duplicate); }
        self.remaining -= 1;
        Some(if self.remaining == 0 { Admission::SelectedAndStop } else { Admission::Selected })
    }

    pub(super) fn remaining(&self) -> usize { self.remaining }
    pub(super) fn invalidate(&mut self) { self.invalid = true; }
    pub(in super::super) fn cannot_continue(&self) -> bool { self.invalid || self.remaining == 0 }
    pub(super) fn invalid(&self) -> bool { self.invalid }

    #[cfg(test)]
    pub(super) fn call_paths(&self) -> Vec<Vec<SourceSpan>> {
        self.selected.iter().map(|path| path.steps.iter().filter_map(|step| match step {
            PathStep::Call(site) => Some(site.clone()),
            _ => None,
        }).collect()).collect()
    }

    #[cfg(test)]
    pub(super) fn contains_branch(&self, condition: SourceSpan, accepted: bool) -> bool {
        self.selected.iter().any(|path| path.steps.contains(&PathStep::Branch { condition: condition.clone(), accepted }))
    }
}
