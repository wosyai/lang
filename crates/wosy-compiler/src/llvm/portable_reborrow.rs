//! Exact allocation/loan closure for portable local projected reborrows.
use std::collections::{HashMap, HashSet};

use crate::scalar::{
    CfgPointId, CfgPointKind, ReferenceBindingId, ReferenceFlowPoint, ReferenceOrigin,
    ReferenceLoanId, ReferenceOriginId, ReferencePlaceId, ReferencePlaceProjection,
    ReleaseTarget, RuntimeArrayChildSite, ScalarReferenceCfg, ScalarReferenceMutability,
};
use crate::ScalarPlace;

type Owner = (u32, u32);

#[derive(Clone, Copy, Eq, PartialEq)]
struct Holder {
    owner: Owner,
    origin: ReferenceOriginId,
    child: bool,
}

pub(super) struct Census<'a> {
    exact: HashMap<CfgPointId, Holder>,
    holders: HashMap<&'a ReferenceBindingId, Holder>,
    origins: HashMap<usize, Holder>,
    scalar_returns: HashMap<CfgPointId, Holder>,
}

impl<'a> Census<'a> {
    pub(super) fn new(cfg: &'a ScalarReferenceCfg) -> Option<Self> {
        if cfg.runtime_array_child_sites.is_empty() { return None; }
        let mut census = Self { exact: HashMap::new(), holders: HashMap::new(), origins: HashMap::new(), scalar_returns: HashMap::new() };
        for site in &cfg.runtime_array_child_sites {
            let Some((install, parent_origin, child_origin, owner)) = cfg.runtime_array_portable_child(site) else { continue; };
            let owner = (owner.declaration_span.start, owner.declaration_span.end);
            let parent = Holder { owner, origin: parent_origin, child: false };
            let child = Holder { owner, origin: child_origin, child: true };
            let CfgPointKind::Bind { binding: child_binding } = &cfg.points[install.child_bind.0].kind else { continue; };
            let CfgPointKind::Bind { binding: parent_binding } = &cfg.points[install.parent_bind.0].kind else { continue; };
            for (point, holder) in [
                (install.parent_borrow, parent), (install.parent_bind, parent),
                (install.parent_read, parent), (install.child_borrow, child), (install.child_bind, child),
            ] {
                if census.exact.insert(point, holder).is_some_and(|old| old != holder) { return None; }
            }
            for (binding, holder) in [(parent_binding, parent), (child_binding, child)] {
                if census.holders.insert(binding, holder).is_some_and(|old| old != holder)
                    || census.origins.insert(holder.origin.0, holder).is_some_and(|old| old != holder)
                { return None; }
            }
        }
        (!census.exact.is_empty()).then_some(census)
    }

    pub(super) fn contains(&self, cfg: &ScalarReferenceCfg, site: &RuntimeArrayChildSite) -> bool {
        cfg.runtime_array_portable_child(site).is_some_and(|(install, _, _, _)|
            self.exact.contains_key(&install.child_borrow))
    }

    // Match both the original lexical pointer read and exact solved origin.
    // A compound projection, checked value read, nullable or mixed origin,
    // or non-unique pointer-use edge is not one of these consumers.
    fn child_use(
        &self, cfg: &ScalarReferenceCfg, point: &ReferenceFlowPoint, required_read: Option<CfgPointId>,
    ) -> Option<Holder> {
        let pointer = match &point.kind {
            CfgPointKind::ReadPlace { place: ScalarPlace::Dereference { .. }, pointer_point: Some(pointer),
                reads_origin: false, .. }
            | CfgPointKind::AssignThrough { place: ScalarPlace::Dereference { .. }, pointer_point: pointer, .. } => *pointer,
            _ => return None,
        };
        if required_read.is_some_and(|read| read != pointer) { return None; }
        let read = cfg.points.get(pointer.0)?;
        let CfgPointKind::Read { binding } = &read.kind else { return None; };
        let holder = *self.holders.get(binding)?;
        let ReferenceOrigin::Fresh { loan, .. } = cfg.facts.get(holder.origin.0)? else { return None; };
        if !holder.child || read.successors.as_slice() != [point.id]
            || !singleton(read, holder.origin) { return None; }
        match &point.kind {
            CfgPointKind::ReadPlace { place, candidates, .. }
                // Unlike AssignThrough, the solved non-reference ReadPlace
                // separately carries the pointer origin on the read itself.
                if singleton(point, holder.origin)
                    && matches!(candidates.as_slice(), [candidate]
                        if exact_child_pointee(candidate, place, loan, true)) => {}
            CfgPointKind::AssignThrough { place, candidates, .. }
                if loan.mode == ScalarReferenceMutability::Mutable
                    && matches!(candidates.as_slice(), [candidate] if candidate.origin == holder.origin
                        && candidate.loan == *loan
                        && exact_child_pointee(&candidate.place, place, loan, false)) => {}
            _ => return None,
        }
        Some(holder)
    }

    // The solver can echo pointee provenance at a scalar return even
    // though the ABI sends only the loaded bits. Seal only the exact single
    // terminal value path; never use this rule on a checked return or alias.
    fn scalar_return(&mut self, cfg: &ScalarReferenceCfg, point: &ReferenceFlowPoint) -> Option<Holder> {
        match &point.kind {
            CfgPointKind::ReturnOutput { output: 0, value_point, checked: false } => {
                let value = cfg.points.get(value_point.0)?;
                if !matches!(value.kind, CfgPointKind::ReadPlace { reads_origin: false, .. })
                    || value.scope != point.scope || value.successors.as_slice() != [point.id]
                { return None; }
                let holder = self.child_use(cfg, value, None)?;
                let [exit] = point.successors.as_slice() else { return None; };
                let exit = cfg.points.get(exit.0)?;
                let [terminal] = exit.successors.as_slice() else { return None; };
                let terminal = cfg.points.get(terminal.0)?;
                if !(point.id < exit.id && exit.id < terminal.id)
                    || point.scope != exit.scope || exit.scope != terminal.scope
                    || !matches!(exit.kind, CfgPointKind::ScopeExit)
                    || !matches!(terminal.kind, CfgPointKind::Return) || !terminal.successors.is_empty()
                    || !singleton(point, holder.origin) || !singleton(terminal, holder.origin)
                { return None; }
                self.scalar_returns.insert(terminal.id, holder);
                Some(holder)
            }
            CfgPointKind::Return => self.scalar_returns.remove(&point.id),
            _ => None,
        }
    }

    pub(super) fn observe(
        &mut self, cfg: &ScalarReferenceCfg, point: &ReferenceFlowPoint, excluded: &mut HashSet<Owner>,
    ) -> bool {
        let planned = self.exact.get(&point.id).copied();
        let allowed = planned.filter(|holder| singleton(point, holder.origin))
            .or_else(|| self.child_use(cfg, point, None))
            .or_else(|| {
                let CfgPointKind::Read { .. } = point.kind else { return None; };
                let [consumer] = point.successors.as_slice() else { return None; };
                self.child_use(cfg, cfg.points.get(consumer.0)?, Some(point.id))
            }).or_else(|| self.scalar_return(cfg, point));
        if let Some(holder) = planned {
            if allowed != Some(holder) { excluded.insert(holder.owner); }
        }
        if let CfgPointKind::Read { binding } = &point.kind {
            if let Some(holder) = self.holders.get(binding) {
                if allowed != Some(*holder) { excluded.insert(holder.owner); }
            }
        }
        // Holder storage is part of the proof, even when the new or moved
        // value carries no tracked origin (e.g. an assignment of null).
        let mut poison_holder = |binding: &ReferenceBindingId| {
            if let Some(holder) = self.holders.get(binding) { excluded.insert(holder.owner); }
        };
        match &point.kind {
            CfgPointKind::Assign { target, .. } => poison_holder(&target.binding),
            CfgPointKind::Move { place, owner, .. } | CfgPointKind::DeferredMove { place, owner, .. } => {
                poison_holder(&place.binding);
                poison_holder(owner);
            }
            CfgPointKind::AssignThrough { candidates, .. }
            | CfgPointKind::MoveThrough { candidates, .. }
            | CfgPointKind::DeferredMoveThrough { candidates, .. }
            | CfgPointKind::Release { target: ReleaseTarget::Checked { candidates, .. } }
            | CfgPointKind::Invalidate { candidates, .. } => {
                for candidate in candidates { poison_holder(&candidate.place.binding); }
            }
            CfgPointKind::Release { target: ReleaseTarget::Raw { binding } } => poison_holder(binding),
            _ => {}
        }
        if let Some(origins) = &point.possible_origins {
            for holder in origins.iter().filter_map(|origin| self.origins.get(&origin.0)) {
                if allowed != Some(*holder) { excluded.insert(holder.owner); }
            }
        }
        allowed.is_some()
    }
}

fn singleton(point: &ReferenceFlowPoint, origin: ReferenceOriginId) -> bool {
    point.possible_origins.as_ref().is_some_and(|origins| origins.len() == 1 && origins.contains(&origin))
}


// The solver preserves the complete issued child's owner/projection and
// substitutes the authored dereference. ReadPlace additionally appends the
// exact child loan; AssignThrough records that loan on its candidate itself.
fn exact_child_pointee(
    candidate: &ReferencePlaceId, authored: &ScalarPlace, loan: &ReferenceLoanId, read: bool,
) -> bool {
    let issued = &loan.origin_place;
    if candidate.place != *authored || candidate.binding != issued.binding
        || candidate.source != issued.source || candidate.function_span != issued.function_span
        || candidate.declaration_span != issued.declaration_span || candidate.block_span != issued.block_span
    {
        return false;
    }
    if read {
        matches!(candidate.projections.split_last(), Some((ReferencePlaceProjection::Dereference(child), prefix))
            if child.as_ref() == loan && prefix == issued.projections)
    } else {
        candidate.projections == issued.projections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scalar::{ReferenceBindingKind, ReferenceScopeId};
    use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

    #[test]
    fn runtime_element_reborrows_poison_registered_holder_on_empty_rhs_assignment() {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let span = ByteSpan::new(3, 8);
        let holder_binding = ReferenceBindingId {
            source: source.clone(), function_span: span, declaration_span: span,
            block_span: span, kind: ReferenceBindingKind::Declared,
        };
        let point = ReferenceFlowPoint {
            id: CfgPointId(0),
            kind: CfgPointKind::Assign {
                target: ReferencePlaceId {
                    source: source.clone(), function_span: span, declaration_span: span,
                    block_span: span, binding: holder_binding.clone(), projections: Vec::new(),
                    place: ScalarPlace::Name { name: "child".to_owned(), span },
                },
                rhs_point: CfgPointId(1), previous_origins: Default::default(), previous_loans: Vec::new(),
            },
            source_span: SourceSpan::new(source, span),
            scope: ReferenceScopeId(0),
            possible_origins: Some(Default::default()),
            successors: Vec::new(),
        };
        let cfg = ScalarReferenceCfg::unsealed_for_test(Vec::new(), Vec::new());
        let mut census = Census { exact: HashMap::new(), holders: HashMap::new(), origins: HashMap::new(), scalar_returns: HashMap::new() };
        let mut excluded = HashSet::new();
        assert!(!census.observe(&cfg, &point, &mut excluded));
        assert!(excluded.is_empty());
        let holder = Holder { owner: (1, 2), origin: ReferenceOriginId(0), child: true };
        census.holders.insert(&holder_binding, holder);
        census.origins.insert(holder.origin.0, holder);
        assert!(!census.observe(&cfg, &point, &mut excluded));
        assert_eq!(excluded, HashSet::from([holder.owner]));
    }
}
