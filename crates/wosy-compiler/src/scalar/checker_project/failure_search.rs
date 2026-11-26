//! Run-local authored path budget. No values form part of structural identity.
use std::{collections::{HashMap, HashSet}, num::NonZeroUsize};
use wosy_syntax::{SourceIdentity, SourceSpan};
use num_bigint::BigInt;
use super::{OccurrenceOrdinal, PreparedCallable, ScalarType};

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

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum StartupProofRole { Bare, Ranked, Aggregate }

pub(super) struct StartupProofObservation {
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
    pub(super) proof_free_scalar_initializers: bool,
    pub(super) named_target: Option<PreparedCallable>,
    pub(super) named_cross_source: bool,
    pub(super) named_observations: Vec<StartupProofObservation>,
    pub(super) required_sites: HashMap<OccurrenceOrdinal, (PreparedCallable, StartupProofRole)>,
    pub(super) retain_after_cutoff: bool,
    pub(super) metered_frames: bool,
    pub(super) contextual_route: Option<(PreparedCallable, PreparedCallable)>,
    pub(super) contextual_observed: bool,
    pub(super) ranked_target: Option<PreparedCallable>,
    pub(super) ranked_observations: Vec<StartupProofObservation>,
    #[cfg(test)]
    pub(super) ranked_entries: Vec<BigInt>,
}

impl StartUpSearch {
    pub(super) fn new(root: &SourceIdentity, cap: NonZeroUsize) -> Self {
        Self { remaining: cap.get(), prefix: vec![PathStep::Startup(root.clone())], selected: HashSet::new(), work: 0, invalid: false, proof_free_scalar_initializers: false, named_target: None, named_cross_source: false, named_observations: Vec::new(), required_sites: HashMap::new(), retain_after_cutoff: false, metered_frames: false,
            contextual_route: None, contextual_observed: false, ranked_target: None, ranked_observations: Vec::new(),
            #[cfg(test)]
            ranked_entries: Vec::new(),
        }
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
        &mut self, declaration: SourceSpan, history: impl IntoIterator<Item = super::super::checker_flow::CallableHistoryStep>, directive: SourceSpan,
    ) -> Option<Admission> {
        use super::super::checker_flow::CallableHistoryStep;
        super::super::checker_flow::charge_call_work(&mut self.work, 2)?;
        let mut steps = vec![PathStep::CallableRoot(declaration)];
        for step in history {
            super::super::checker_flow::charge_call_work(&mut self.work, 1)?;
            let at = |range| SourceSpan::new(directive.source.clone(), range);
            steps.push(match step {
                CallableHistoryStep::Branch(span, accepted) => PathStep::Branch { condition: at(span), accepted },
                CallableHistoryStep::Call(span) => PathStep::Call(at(span)),
                CallableHistoryStep::Return(span) => PathStep::Return(at(span)),
            });
        }
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
    pub(super) fn enroll_proof_sites(&mut self, target: PreparedCallable, role: StartupProofRole, sites: &[OccurrenceOrdinal]) -> Option<()> {
        super::super::checker_flow::charge_call_work(&mut self.work, sites.len())?;
        for &site in sites {
            if self.required_sites.insert(site, (target, role)).is_some() { return None; }
        }
        Some(())
    }

    pub(super) fn charge_proof_frame(&mut self, body: &super::FunctionCallSites) -> Option<()> {
        if self.metered_frames {
            super::super::checker_flow::charge_call_work(&mut self.work, super::named_exact_frame_cost(body)?)?;
        }
        Some(())
    }

    pub(super) fn charge_formal_replay(&mut self, activations: usize, per_activation: usize) -> Option<()> {
        super::super::checker_flow::charge_call_work(&mut self.work, activations.checked_mul(per_activation)?)
    }

    pub(super) fn has_aggregate_site(&self, caller: &SourceIdentity, site: OccurrenceOrdinal) -> bool {
        self.owns_initialization(caller) && self.required_sites.get(&site).is_some_and(|(_, role)| *role == StartupProofRole::Aggregate)
    }

    pub(super) fn owns_initialization(&self, source: &SourceIdentity) -> bool {
        matches!(self.prefix.first(), Some(PathStep::Startup(root)) if root == source)
    }

    pub(super) fn pending_named_startup_observation(&self) -> bool {
        self.retain_after_cutoff && !self.required_sites.is_empty()
    }

    pub(in super::super) fn proof_only_continuation(&self) -> bool {
        !self.invalid && self.remaining == 0
            && ((self.contextual_route.is_some() && !self.contextual_observed)
                || self.pending_named_startup_observation())
    }

    pub(in super::super) fn cannot_continue(&self) -> bool {
        self.invalid || (self.remaining == 0 && !self.proof_only_continuation())
    }
    pub(super) fn invalid(&self) -> bool { self.invalid }

    #[cfg(test)]
    pub(super) fn call_paths(&self) -> Vec<Vec<SourceSpan>> {
        self.selected.iter().map(|path| path.steps.iter().filter_map(|step| match step {
            PathStep::Call(site) => Some(site.clone()),
            _ => None,
        }).collect()).collect()
    }

    #[cfg(test)]
    pub(super) fn contains_return(&self, site: &SourceSpan) -> bool {
        self.selected.iter().any(|path| path.steps.contains(&PathStep::Return(site.clone())))
    }

    #[cfg(test)]
    pub(super) fn contains_branch(&self, condition: SourceSpan, accepted: bool) -> bool {
        self.selected.iter().any(|path| path.steps.contains(&PathStep::Branch { condition: condition.clone(), accepted }))
    }
}
