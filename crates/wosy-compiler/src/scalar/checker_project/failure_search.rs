//! Run-local authored path budget. No values form part of structural identity.
use std::{rc::Rc, collections::{HashMap, HashSet}, num::NonZeroUsize};
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
    MatchArm(SourceSpan),
    Branch { condition: SourceSpan, accepted: bool },
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct FailurePath {
    steps: Vec<PathStep>,
    directive: SourceSpan,
}

pub(super) enum Admission { Duplicate, Selected, SelectedAndStop }

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum StartupProofRole { Bare, Ranked, Aggregate, Cited }

pub(super) struct StartupProofObservation {
    pub(super) target: PreparedCallable,
    pub(super) site: SourceSpan,
    pub(super) directive_position: usize,
    pub(super) parameter: ScalarType,
    pub(super) argument: BigInt,
    pub(super) returned: BigInt,
}

pub(super) struct WideRankObservation {
    pub(super) target: PreparedCallable,
    pub(super) site: SourceSpan,
    pub(super) outer: Option<SourceSpan>,
    pub(super) arguments: Vec<BigInt>,
    pub(super) returned: BigInt,
}

pub(super) struct PointeeRoute {
    pub(super) startup_site: OccurrenceOrdinal,
    pub(super) caller: PreparedCallable,
    pub(super) callee: PreparedCallable,
    pub(super) pending_calls: HashSet<OccurrenceOrdinal>,
}

/// Original typed direct contextual edges and the number of activations
/// required by the admitted straight-line startup. Distinct caller bodies
/// may share occurrence ordinals; the prepared parent is part of the key.
pub(super) struct ContextualStartupRoster {
    pub(super) root_site: OccurrenceOrdinal,
    pub(super) entry: PreparedCallable,
    pub(super) pending: HashMap<(PreparedCallable, OccurrenceOrdinal), (PreparedCallable, usize)>,
}

impl ContextualStartupRoster {
    pub(super) fn expects(&self, parent: PreparedCallable, site: OccurrenceOrdinal, target: PreparedCallable) -> bool {
        self.pending.get(&(parent, site)).is_some_and(|&(expected, count)| expected == target && count != 0)
    }

    pub(super) fn observe(&mut self, parent: PreparedCallable, site: OccurrenceOrdinal, target: PreparedCallable) -> Option<()> {
        let key = (parent, site);
        let (expected, count) = self.pending.get_mut(&key)?;
        if *expected != target { return None; }
        *count = count.checked_sub(1)?;
        if *count == 0 { self.pending.remove(&key); }
        Some(())
    }
}

pub(super) struct MixedContextStartup {
    pub(super) root_site: OccurrenceOrdinal,
    pub(super) entry_site: OccurrenceOrdinal,
    pub(super) bridge: PreparedCallable,
    pub(super) bridge_site: OccurrenceOrdinal,
    pub(super) finished: bool,
    pub(super) refuted_after_named: Option<usize>,
    pub(super) dependent_requires: HashSet<OccurrenceOrdinal>,
}

pub(in super::super) type SharedStartupTrace = std::rc::Rc<std::cell::RefCell<StartUpSearch>>;

pub(in super::super) struct StartUpSearch {
    remaining: usize,
    prefix: Vec<PathStep>,
    selected: HashSet<FailurePath>,
    #[cfg(test)]
    selected_in_order: Vec<Vec<PathStep>>,
    work: usize,
    invalid: bool,
    pub(super) proof_free_scalar_initializers: bool,
    pub(super) has_bare_startup: bool,
    pub(super) named_cross_source: bool,
    pub(super) named_observations: Vec<StartupProofObservation>,
    pub(super) required_sites: HashMap<OccurrenceOrdinal, (PreparedCallable, StartupProofRole)>,
    pub(super) retain_after_cutoff: bool,
    finish_dormant_startup: bool,
    pub(super) metered_frames: bool,
    pub(super) cited_routes: HashMap<PreparedCallable, (OccurrenceOrdinal, PreparedCallable)>,
    pub(super) pointee_route: Option<PointeeRoute>,
    pub(super) contextual_roster: Option<ContextualStartupRoster>,
    pub(super) contextual_route: Option<(PreparedCallable, PreparedCallable)>,
    pub(super) contextual_observed: bool,
    pub(super) mixed_context: Option<MixedContextStartup>,
    pub(super) ranked_target: Option<PreparedCallable>,
    pub(super) ranked_observations: Vec<StartupProofObservation>,
    pub(super) wide_rank_recipe: Option<Rc<super::ranked::CappedTailRecipe>>,
    pub(super) wide_rank_observations: Vec<WideRankObservation>,
    pub(super) pending_wide_forward: Option<(SourceSpan, SourceSpan, Vec<BigInt>)>,
    #[cfg(test)]
    pub(super) ranked_entries: Vec<BigInt>,
}

impl StartUpSearch {
    pub(super) fn new(root: &SourceIdentity, cap: NonZeroUsize) -> Self {
        Self::with_budget(root, cap.get(), false)
    }

    fn with_budget(root: &SourceIdentity, remaining: usize, finish_dormant_startup: bool) -> Self {
        Self { remaining, prefix: vec![PathStep::Startup(root.clone())], selected: HashSet::new(),
            #[cfg(test)]
            selected_in_order: Vec::new(),
            work: 0, invalid: false, proof_free_scalar_initializers: false, has_bare_startup: false, named_cross_source: false, named_observations: Vec::new(), required_sites: HashMap::new(), retain_after_cutoff: false, finish_dormant_startup, metered_frames: false, cited_routes: HashMap::new(),
            pointee_route: None, contextual_roster: None, contextual_route: None, contextual_observed: false, mixed_context: None, ranked_target: None, ranked_observations: Vec::new(), wide_rank_recipe: None, wide_rank_observations: Vec::new(), pending_wide_forward: None,
            #[cfg(test)]
            ranked_entries: Vec::new(),
        }
    }

    /// A separately prepared proof-bearing package whose command budget
    /// was already spent. It has no authored admission from its first item.
    pub(super) fn formal_only(root: &SourceIdentity) -> Self {
        Self::with_budget(root, 0, true)
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
        &mut self, declaration: SourceSpan, history: impl IntoIterator<Item = super::super::checker_flow::CallableHistoryStep>,
        directive: SourceSpan, modules: &[super::ScalarModule],
    ) -> Option<Admission> {
        use super::super::checker_flow::CallableHistoryStep;
        super::super::checker_flow::charge_call_work(&mut self.work, 2)?;
        let root = declaration.source.clone();
        if directive.source.project != root.project || directive.source.package != root.package { return None; }
        let mut steps = vec![PathStep::CallableRoot(declaration)];
        for step in history {
            super::super::checker_flow::charge_call_work(&mut self.work, 1)?;
            let at = |range| SourceSpan::new(root.clone(), range);
            let provider_at = |owner, range| -> Option<SourceSpan> {
                let module: &super::ScalarModule = modules.get(owner)?;
                let source = &module.source;
                if source.project != root.project || source.package != root.package
                    || source == &root { return None; }
                Some(SourceSpan::new(source.clone(), range))
            };
            steps.push(match step {
                CallableHistoryStep::Branch(span, accepted) => PathStep::Branch { condition: at(span), accepted },
                CallableHistoryStep::ProviderBranch(owner, span, accepted) =>
                    PathStep::Branch { condition: provider_at(owner, span)?, accepted },
                CallableHistoryStep::MatchArm(span) => PathStep::MatchArm(at(span)),
                CallableHistoryStep::ProviderMatchArm(owner, span) => PathStep::MatchArm(provider_at(owner, span)?),
                CallableHistoryStep::Call(span) => PathStep::Call(at(span)),
                CallableHistoryStep::Return(span) => PathStep::Return(at(span)),
                CallableHistoryStep::ProviderCall(owner, span) => PathStep::Call(provider_at(owner, span)?),
                CallableHistoryStep::ProviderReturn(owner, span) => PathStep::Return(provider_at(owner, span)?),
            });
        }
        self.select(FailurePath { steps, directive })
    }

    fn select(&mut self, path: FailurePath) -> Option<Admission> {
        if self.remaining == 0 { return None; }
        #[cfg(test)]
        let observed = path.steps.clone();
        if !self.selected.insert(path) { return Some(Admission::Duplicate); }
        #[cfg(test)]
        self.selected_in_order.push(observed);
        self.remaining -= 1;
        Some(if self.remaining == 0 { Admission::SelectedAndStop } else { Admission::Selected })
    }

    pub(super) fn remaining(&self) -> usize { self.remaining }
    pub(super) fn invalidate(&mut self) { self.invalid = true; }
    pub(super) fn enroll_proof_sites(&mut self, target: PreparedCallable, role: StartupProofRole, sites: &[OccurrenceOrdinal]) -> Option<()> {
        super::super::checker_flow::charge_call_work(&mut self.work, sites.len())?;
        // At least one actual observation means the existing required-site
        // continuation, not the dormant-only lane, owns the stopping point.
        if !sites.is_empty() {
            self.finish_dormant_startup = false;
            self.has_bare_startup |= role == StartupProofRole::Bare;
        }
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

    pub(super) fn allows_qualified_scalar_proof(&self, caller: &SourceIdentity, site: OccurrenceOrdinal, target: (usize, usize)) -> bool {
        self.named_cross_source && self.owns_initialization(caller)
            && matches!(self.required_sites.get(&site),
                Some((prepared, StartupProofRole::Bare | StartupProofRole::Ranked)) if *prepared == target)
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

    pub(in super::super) fn admits_typed_ordinary_startup(&self) -> bool { self.proof_free_scalar_initializers }

    pub(in super::super) fn proof_only_continuation(&self) -> bool {
        !self.invalid && self.remaining == 0
            && (self.finish_dormant_startup || (self.contextual_route.is_some() && !self.contextual_observed)
                || self.contextual_roster.as_ref().is_some_and(|roster| !roster.pending.is_empty())
                || self.mixed_context.as_ref().is_some_and(|route| !route.finished)
                || self.pending_named_startup_observation()
                || self.pointee_route.as_ref().is_some_and(|route| !route.pending_calls.is_empty()))
    }

    pub(in super::super) fn cannot_continue(&self) -> bool {
        self.invalid || (self.remaining == 0 && !self.proof_only_continuation())
    }
    pub(super) fn invalid(&self) -> bool { self.invalid }

    #[cfg(test)]
    pub(super) fn current_path(&self) -> &[PathStep] { &self.prefix }

    #[cfg(test)]
    pub(super) fn selected_steps(&self) -> impl ExactSizeIterator<Item = &[PathStep]> {
        self.selected_in_order.iter().map(Vec::as_slice)
    }

    #[cfg(test)]
    pub(super) fn call_paths(&self) -> Vec<Vec<SourceSpan>> {
        self.selected.iter().map(|path| path.steps.iter().filter_map(|step| match step {
            PathStep::Call(site) => Some(site.clone()),
            _ => None,
        }).collect()).collect()
    }

    #[cfg(test)]
    pub(super) fn contains_match_arm(&self, site: &SourceSpan) -> bool {
        self.selected.iter().any(|path| path.steps.contains(&PathStep::MatchArm(site.clone())))
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
