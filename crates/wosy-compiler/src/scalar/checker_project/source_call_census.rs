//! Inventory of prepared source uses, not proof of actual activation or artifact roots.
use super::*;
use crate::scalar::control::ModuleInitializationDetail;
use crate::scalar::reference::FunctionCallSites;
use wosy_syntax::SourceSpan;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SourceOwner {
    Initialization(usize),
    Function {
        declaration: PreparedCallable,
        // The row ordinal is relative to this specific prepared overload arm.
        arms: Vec<usize>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SourceUseKind {
    Direct,
    Overload,
    Generic,
    CallableBinding,
    AllocationLength,
    Unassociated,
}

#[derive(Clone, Debug)]
pub(super) struct SourceUse {
    pub(super) owner: SourceOwner,
    pub(super) kind: SourceUseKind,
    pub(super) site: Option<OccurrenceOrdinal>,
    pub(super) span: ByteSpan,
    pub(super) signature: Option<CheckedCallSignature>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ValueUse {
    pub(super) owner: SourceOwner,
    pub(super) site: OccurrenceOrdinal,
    pub(super) span: ByteSpan,
}

#[derive(Default)]
pub(super) struct SourceUses {
    pub(super) sites: Vec<SourceUse>,
    // A value use is not itself an execution, and must never be silently
    // reclassified as a call with the same span.
    pub(super) escapes: Vec<ValueUse>,
}

pub(super) struct SourceCallCensus {
    pub(super) uses: HashMap<PreparedCallable, SourceUses>,
    pub(super) foreign_or_indirect: usize,
}

impl SourceCallCensus {
    /// Scan only prepared semantic tables; never treat absence of a use as
    /// an assertion about the selected artifact or the host.
    pub(super) fn build(
        project: &ScalarProject,
        calls: &ProjectCallSiteResults,
        selected: impl IntoIterator<Item = PreparedCallable>,
    ) -> Option<Self> {
        let mut index = HashMap::new();
        let mut uses = HashMap::new();
        for target @ (module, item) in selected {
            let module = project.modules.get(module)?;
            let ScalarItem::Function(function) = module.items.get(item)? else { return None; };
            let source = (&module.source, function.name_span);
            if index.insert(source, target).is_some() { return None; }
            uses.insert(target, SourceUses::default());
        }
        let mut out = Self { uses, foreign_or_indirect: 0 };
        // Even a loaded module outside the selected package may contain a
        // typed use or callable escape; it must not vanish from the census.
        for (module, source) in project.modules.iter().enumerate() {
            let region = calls.initialization_region(module)?;
            if region.source != source.source { return None; }
            let ModuleInitializationDetail::Occurrences(initializers) = &region.detail else { return None; };
            let owner = SourceOwner::Initialization(module);
            out.record_escapes(&index, &owner, &initializers.structure.prepared_callable_value_uses,
                &initializers.occurrences)?;
            let mut canonical = HashSet::new();
            for &ordinal in &initializers.calls {
                if !canonical.insert(ordinal) || !initializers.call_targets.contains_key(&ordinal) {
                    return None;
                }
            }
            let mut sites: Vec<_> = initializers.call_targets.iter().collect();
            sites.sort_unstable_by_key(|(ordinal, _)| ordinal.0);
            for (&site, target) in sites {
                let (declaration, kind) = match target {
                    ModuleInitializationCallTarget::Direct { declaration, concrete } =>
                        (declaration, Self::selection(concrete)),
                    ModuleInitializationCallTarget::Generic { declaration, .. } =>
                        (declaration, SourceUseKind::Generic),
                    ModuleInitializationCallTarget::CallableBinding(declaration) => {
                        out.note_foreign_or_indirect()?;
                        (declaration, SourceUseKind::CallableBinding)
                    }
                    ModuleInitializationCallTarget::Core(_) => continue,
                    ModuleInitializationCallTarget::Extern(_) | ModuleInitializationCallTarget::CallableField { .. } => {
                        out.note_foreign_or_indirect()?;
                        continue;
                    }
                };
                if let Some(target) = index.get(&(&declaration.source, declaration.range)) {
                    let kind = if canonical.contains(&site) { kind } else { SourceUseKind::Unassociated };
                    let span = initializers.occurrences.get(site.0)?.span;
                    out.record(*target, &owner, kind, Some(site), span, initializers.checked_call_signatures.get(&site));
                }
            }
            for (item, declaration) in source.items.iter().enumerate() {
                if !matches!(declaration, ScalarItem::Function(_)) { continue; }
                let body = calls.function_body(module, item, None)?;
                out.scan_function(&index, (module, item), body)?;
            }
        }
        Some(out)
    }

    #[cfg(test)]
    pub(super) fn test_with_unassociated_first_call(
        project: &ScalarProject, calls: &ProjectCallSiteResults, caller: PreparedCallable, callee: PreparedCallable,
    ) -> Option<Vec<SourceUse>> {
        let mut body = calls.function_body(caller.0,caller.1,None)?.clone();
        body.call_occurrences.remove(0);
        let owner = project.modules.get(callee.0)?;
        let ScalarItem::Function(function) = owner.items.get(callee.1)? else { return None; };
        let index = HashMap::from([((&owner.source,function.name_span),callee)]);
        let mut census = Self { uses:HashMap::from([(callee,SourceUses::default())]), foreign_or_indirect:0 };
        census.scan_function(&index,caller,&body)?;
        Some(census.uses.remove(&callee)?.sites)
    }

    fn note_foreign_or_indirect(&mut self) -> Option<()> {
        self.foreign_or_indirect = self.foreign_or_indirect.checked_add(1)?;
        Some(())
    }

    fn selection(concrete: &ConcreteCallSelection) -> SourceUseKind {
        match concrete {
            ConcreteCallSelection::Declaration => SourceUseKind::Direct,
            ConcreteCallSelection::Overload(_) => SourceUseKind::Overload,
        }
    }

    fn record_escapes(
        &mut self, index: &HashMap<(&SourceIdentity, ByteSpan), PreparedCallable>, owner: &SourceOwner,
        escapes: &HashMap<OccurrenceOrdinal, SourceSpan>,
        occurrences: &[crate::scalar::reference::OccurrenceMetadata],
    ) -> Option<()> {
        let mut rows = escapes.iter().collect::<Vec<_>>();
        rows.sort_unstable_by_key(|(site, _)| site.0);
        for (&site, escape) in rows {
            if let Some(target) = index.get(&(&escape.source, escape.range)) {
                let span = occurrences.get(site.0)?.span;
                self.uses.get_mut(target)?.escapes.push(ValueUse { owner: owner.clone(), site, span });
            }
        }
        Some(())
    }

    fn record(&mut self, target: PreparedCallable, owner: &SourceOwner, kind: SourceUseKind,
        site: Option<OccurrenceOrdinal>, span: ByteSpan, signature: Option<&CheckedCallSignature>,
    ) {
        self.uses.get_mut(&target).expect("registered declaration").sites.push(SourceUse {
            owner: owner.clone(), kind, site, span, signature: signature.cloned(),
        });
    }

    fn select_target(
        &mut self, index: &HashMap<(&SourceIdentity, ByteSpan), PreparedCallable>, target: &ResolvedCallTarget,
    ) -> Option<Option<(PreparedCallable, SourceUseKind)>> {
        match target {
            ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } =>
                Some(index.get(&(source, *declaration_span))
                    .map(|&target| (target, Self::selection(concrete)))),
            ResolvedCallTarget::Core(_) => Some(None),
            ResolvedCallTarget::ExternCallable { .. } | ResolvedCallTarget::LocalCallable(_)
                | ResolvedCallTarget::CallableField { .. } => {
                self.note_foreign_or_indirect()?;
                Some(None)
            }
        }
    }

    fn scan_function(
        &mut self,
        index: &HashMap<(&SourceIdentity, ByteSpan), PreparedCallable>,
        declaration: PreparedCallable,
        root: &FunctionCallSites,
    ) -> Option<()> {
        let mut todo = vec![(root, Vec::new())];
        while let Some((body, arms)) = todo.pop() {
            for (arm, child) in body.arms.iter().enumerate().rev() {
                let mut path = arms.clone();
                path.push(arm);
                todo.push((child, path));
            }
            let owner = SourceOwner::Function { declaration, arms };
            self.record_escapes(index, &owner, &body.structure.prepared_callable_value_uses, &body.occurrences)?;
            let mut associated = vec![false; body.sites.len()];
            for occurrence in &body.call_occurrences {
                let ordinal = occurrence.ordinal;
                let call = body.sites.get(occurrence.site)?;
                // A prepared occurrence can refer to a shared resolution,
                // without making the occurrences themselves interchangeable.
                associated[occurrence.site] = true;
                if let Some((target, kind)) = self.select_target(index, &call.target)? {
                    let span = body.occurrences.get(ordinal.0)?.span;
                    self.record(target, &owner, kind, Some(ordinal), span, body.checked_call_signatures.get(&ordinal));
                }
            }
            for (slot, call) in body.sites.iter().enumerate() {
                if !associated[slot] {
                    if let Some((target, _)) = self.select_target(index, &call.target)? {
                        // No executable occurrence was authenticated. Retain
                        // the resolved source range and no checked signature.
                        self.record(target, &owner, SourceUseKind::Unassociated,
                            None, call.source_span.range, None);
                    }
                }
            }
            let mut lengths: Vec<_> = body.allocation_length_call_targets.iter().collect();
            lengths.sort_unstable_by_key(|(ordinal, _)| ordinal.0);
            for (&ordinal, call) in lengths {
                if let Some((target, _)) = self.select_target(index, call)? {
                    let span = body.occurrences.get(ordinal.0)?.span;
                    self.record(target, &owner, SourceUseKind::AllocationLength, Some(ordinal), span,
                        body.checked_call_signatures.get(&ordinal));
                }
            }
        }
        Some(())
    }
}

/// Reverse edges for ordinary authored root planning. This is deliberately
/// not an artifact entry roster: a missing edge does not certify nonuse,
/// purity or external reachability. Only the separate package startup and
/// symbolic execution may establish those facts.
pub(super) struct PreparedAuthoredIncoming {
    parents: HashMap<PreparedCallable, Vec<PreparedCallable>>,
    unresolved: HashSet<PreparedCallable>,
}

impl PreparedAuthoredIncoming {
    pub(super) fn build(project: &ScalarProject, calls: &ProjectCallSiteResults,
        package: &SourceIdentity) -> Option<Self> {
        let mut index = HashMap::new();
        for (module, owner) in project.modules.iter().enumerate() {
            if owner.source.project != package.project || owner.source.package != package.package { continue; }
            for (item, declaration) in owner.items.iter().enumerate() {
                if let ScalarItem::Function(function) = declaration {
                    if index.insert((&owner.source,function.name_span),(module,item)).is_some() { return None; }
                }
            }
        }
        let mut out = Self { parents:HashMap::new(),unresolved:HashSet::new() };
        let find = |resolution: &ResolvedCallTarget| match resolution {
            ResolvedCallTarget::ModuleCallable { source,declaration_span,concrete } =>
                index.get(&(source,*declaration_span)).copied().map(|target|(target,matches!(concrete,ConcreteCallSelection::Declaration))),
            _=>None,
        };
        for (module,owner) in project.modules.iter().enumerate() {
            let region=calls.initialization_region(module)?;
            if region.source != owner.source { return None; }
            let ModuleInitializationDetail::Occurrences(initializers)=&region.detail else { return None; };
            let canonical:HashSet<_>=initializers.calls.iter().copied().collect();
            for (&site,edge) in &initializers.call_targets {
                let (decl,direct)=match edge {
                    ModuleInitializationCallTarget::Direct { declaration,concrete } =>
                        (declaration,matches!(concrete,ConcreteCallSelection::Declaration)),
                    ModuleInitializationCallTarget::Generic { declaration,.. }
                    | ModuleInitializationCallTarget::CallableBinding(declaration) => (declaration,false),
                    _=>continue,
                };
                let Some(&target)=index.get(&(&decl.source,decl.range)) else { continue; };
                if !direct || !canonical.contains(&site)
                    || initializers.structure.checker_direct_targets.get(&site)!=Some(&target)
                    || !initializers.checked_call_signatures.contains_key(&site)
                {
                    out.unresolved.insert(target);
                }
            }
            for value in initializers.structure.prepared_callable_value_uses.values() {
                if let Some(target)=index.get(&(&value.source,value.range)) { out.unresolved.insert(*target); }
            }
            let same_package=owner.source.project==package.project && owner.source.package==package.package;
            for (item,declaration) in owner.items.iter().enumerate() {
                if !matches!(declaration,ScalarItem::Function(_)) { continue; }
                let parent=(module,item);
                let mut body_stack=vec![calls.function_body(module,item,None)?];
                while let Some(body)=body_stack.pop() {
                    body_stack.extend(&body.arms);
                    for value in body.structure.prepared_callable_value_uses.values() {
                        if let Some(target)=index.get(&(&value.source,value.range)) { out.unresolved.insert(*target); }
                    }
                    let mut associated=vec![false;body.sites.len()];
                    for occurrence in &body.call_occurrences {
                        *associated.get_mut(occurrence.site)?=true;
                        let resolution=&body.sites.get(occurrence.site)?.target;
                        let Some((target,direct))=find(resolution) else { continue; };
                        let site=occurrence.ordinal;
                        let typed=body.checked_call_signatures.get(&site).is_some_and(|signature|
                            authenticate_prepared_direct_edge(project,calls,module,body,site,signature)==Some(target));
                        if !same_package || !typed || !direct {
                            out.unresolved.insert(target);
                        } else {
                            // All prepared occurrence sites are considered, including
                            // qualified scalar calls. At most one parent edge is kept
                            // per function, even if several arms invoke it.
                            let before=out.parents.entry(target).or_default();
                            if before.last()!=Some(&parent) { before.push(parent); }
                        }
                    }
                    for (known,site) in associated.into_iter().zip(&body.sites) {
                        if !known {
                            if let Some((target,_))=find(&site.target) { out.unresolved.insert(target); }
                        }
                    }
                    for call in body.allocation_length_call_targets.values() {
                        if let Some((target,_))=find(call) { out.unresolved.insert(target); }
                    }
                }
            }
        }
        Some(out)
    }

    pub(super) fn parents(&self,target:PreparedCallable)->Option<&[PreparedCallable]> {
        if self.unresolved.contains(&target) { return None; }
        Some(self.parents.get(&target).map(Vec::as_slice).unwrap_or(&[]))
    }
}
