//! One in-run cache for pure qualified provider obligations. Demand never
//! changes symbolic root order or emits diagnostics from another root.
use super::*;
use super::super::checker_flow::charge_call_work;

#[derive(Default)]
pub(super) struct Cache<'proof> {
    verified: SymbolicSummaryIndex<'proof>,
    verdicts: HashMap<PreparedCallable, Verdict>,
    work: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Evidence {
    Certified,
    Refuted,
    Blocked(PreparedCallable),
    Unavailable,
}

struct Verdict {
    active: bool,
    scheduled: bool,
    evidence: Evidence,
    failure: Option<SymbolicProofDiagnostic>,
}

impl Cache<'_> {
    fn refuter(&self, target: PreparedCallable) -> Result<PreparedCallable, ProjectCheck> {
        let verdict = self.verdicts.get(&target).ok_or(ProjectCheck::Unsupported)?;
        let refuter = match verdict.evidence {
            Evidence::Refuted => target,
            Evidence::Blocked(refuter) => refuter,
            Evidence::Certified | Evidence::Unavailable => return Err(ProjectCheck::Unsupported),
        };
        let receipt = self.verdicts.get(&refuter).ok_or(ProjectCheck::Unsupported)?;
        if receipt.evidence != Evidence::Refuted || !(receipt.scheduled || receipt.failure.is_some()) {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(refuter)
    }

    pub(super) fn complete(&self) -> Result<(), ProjectCheck> {
        for (&target, verdict) in &self.verdicts {
            if !verdict.scheduled || verdict.failure.is_some() { return Err(ProjectCheck::Unsupported); }
            match verdict.evidence {
                Evidence::Certified if self.verified.lookup(target).is_none() => return Err(ProjectCheck::Unsupported),
                Evidence::Blocked(_) => { self.refuter(target)?; }
                _ => {}
            }
        }
        Ok(())
    }
}

enum Evaluation<'proof> {
    Checked { active: bool, reusable: bool, entry_result: Option<VerifiedSymbolicResult<'proof>> },
    Blocked(PreparedCallable),
}

struct Job<'proof, 'run, 'case> {
    target: PreparedCallable,
    case: &'case MainFlowProofCase<'proof>,
    issuer: Option<&'case SymbolicSummaryIssuer<'proof>>,
    body: &'run FunctionCallSites,
    function: &'run ScalarFunction,
    owner: &'run ScalarModule,
    origins: Vec<RankedScalarOrigin<'run>>,
}

impl<'proof, 'run, 'case> Job<'proof, 'run, 'case> {
    fn discover(
        watched: &'case MainFlowProofObserver<'proof>, target: PreparedCallable,
        runtime: SymbolicProofRuntime<'run>,
    ) -> Option<Self> {
        let group = if watched.first.0 == target { &watched.first.1 } else { watched.other.get(&target)? };
        let MainFlowProofCases::Bare { first, rest } = &group.cases else { return None; };
        let owner = runtime.project.modules.get(target.0)?;
        if group.seen || !rest.is_empty() || first.proof_source != &owner.source || first.owner != &owner.source
            || runtime.visits?.get(target.0) != Some(&InitializationVisit::Completed)
        {
            return None;
        }
        let ScalarItem::Function(function) = owner.items.get(target.1)? else { return None; };
        if !function.generic_parameters.is_empty() || !function.overload_arms.is_empty() { return None; }
        let body = runtime.calls.function_body(target.0, target.1, None)?;
        let origins = if body.call_occurrences.is_empty() {
            if !group.qualified_closed_leaf || !body.checker_pure_calling_frame()
                || !body.structure.checker_direct_targets.is_empty() || !valid_arbitrary_proof_body(body)
            {
                return None;
            }
            Vec::new()
        } else {
            ranked_scalar_return_origins(body)?
        };
        if origins.iter().any(|(callee, _, _, _)| callee.0 == target.0)
            || (first.bound.literal().is_none() && !(origins.is_empty()
                && matches!(first.bound, MainFlowProofBound::Entry(0) | MainFlowProofBound::EntryAffine { slot: 0, .. })
                && first.relation == IntegerRelation::from_operator(&BinaryOperator::Equal)
                && body.boundary.results.first().is_some_and(|out| supports_unary_entry_proof(body, &out.ty))))
        {
            return None;
        }
        Some(Self { target, case: first, issuer: group.qualified_issuer.as_ref(), body, function, owner, origins })
    }

    fn export_issuer(&self) -> Option<&'case SymbolicSummaryIssuer<'proof>> {
        // Export only an exact unary forwarding definition, including a
        // bounded ordered sequence whose dependencies are all verified.
        // All declared entry restrictions survive.
        let issuer = self.issuer.filter(|issuer| std::ptr::eq(issuer.body, self.body))?;
        let [parameter] = self.body.boundary.parameters.as_slice() else { return None; };
        let case = self.case;
        if !is_integer_type(&parameter.ty)
            || case.guard.always_rejects || case.guard.domain.is_some() || case.guard.arity != 1
            || case.guard.boolean.iter().any(Option::is_some)
            || case.relation != IntegerRelation::from_operator(&BinaryOperator::Equal)
            || !case.filter.iter().all(|guard| matches!(guard, CheckerPredicate::Integer(_) | CheckerPredicate::Boolean(true)))
            || !self.body.callable_escapes.is_empty() || !self.body.structure.module_storage_uses.is_empty()
            || self.body.contextual_formal().is_some()
        {
            return None;
        }
        Some(issuer)
    }

    fn summary(&self, entry_result: Option<VerifiedSymbolicResult<'proof>>) -> Option<VerifiedSymbolicSummary<'proof>> {
        let issuer = self.export_issuer()?;
        let case = self.case;
        Some(VerifiedSymbolicSummary {
            target: self.target, canonical_target: issuer.canonical_target, layout: issuer.layout, body: issuer.body,
            result: match entry_result {
                Some(result) => result,
                None => VerifiedSymbolicResult::Constant { bound: case.bound.literal()?, shared: std::cell::OnceCell::new() },
            },
            guards: VerifiedSymbolicGuards::Derived(case.guard.comparisons.clone()),
        })
    }
}

fn checked_edge(
    caller: &ScalarModule, body: &FunctionCallSites, (target, _, origin, occurrence): RankedScalarOrigin<'_>,
    canonical: &ResolvedCallTarget, provider_body: &FunctionCallSites, runtime: SymbolicProofRuntime<'_>,
) -> Result<(), ProjectCheck> {
    let owner = runtime.project.modules.get(target.0).ok_or(ProjectCheck::MissingRoot)?;
    let signature = body.checked_call_signatures.get(&origin.call).ok_or(ProjectCheck::MissingRootOccurrences)?;
    let [output] = provider_body.boundary.results.as_slice() else { return Err(ProjectCheck::Unsupported); };
    let site = body.sites.get(occurrence.site).ok_or(ProjectCheck::MissingRootOccurrences)?;
    if &caller.source == &owner.source
        || !completed_namespace_owner(&caller.source, &owner.source, target.0, runtime.visits)
        || !std::ptr::eq(runtime.calls.function_body(target.0, target.1, None).ok_or(ProjectCheck::Unsupported)?, provider_body)
        || output.ordinal != 0
        || !production_bound_reuse::matching_call(
            canonical, owner.target_layout, &provider_body.boundary.parameters, &output.ty,
            caller.target_layout, body, origin, &site.target, signature,
        )
    {
        return Err(ProjectCheck::Unsupported);
    }
    Ok(())
}

impl<'proof, 'case> SelectedSymbolicProof<'proof, 'case> {
    fn qualify_edge(
        &self, caller: &ScalarModule, body: &FunctionCallSites, origin: RankedScalarOrigin<'_>,
        runtime: SymbolicProofRuntime<'_>, ranked: &HashMap<PreparedCallable, RankedOrderedSymbolic<'_, 'proof>>,
    ) -> Result<(), ProjectCheck> {
        let (target, _, argument, _) = origin;
        if let Some(provider) = ranked.get(&target) {
            let owner = runtime.project.modules.get(target.0).ok_or(ProjectCheck::MissingRoot)?;
            if !provider.allow_qualified_demand || provider.plan.proof_source != &owner.source
                || !matches!(argument.arguments, super::super::reference::CheckerIntegerCallArguments::Wide(_))
            {
                return Err(ProjectCheck::Unsupported);
            }
            return checked_edge(caller, body, origin, provider.plan.canonical_target, provider.plan.typed.body, runtime);
        }
        if argument.arguments.as_slice().len() != 1 { return Err(ProjectCheck::Unsupported); }
        if let Some(summary) = self.qualified.verified.lookup(target) {
            return checked_edge(caller, body, origin, summary.canonical_target, summary.body, runtime);
        }
        let provider = Job::discover(self.watched, target, runtime).ok_or(ProjectCheck::Unsupported)?;
        let issuer = provider.export_issuer().ok_or(ProjectCheck::Unsupported)?;
        checked_edge(caller, body, origin, issuer.canonical_target, issuer.body, runtime)
    }

    fn ensure_qualified_provider(
        &mut self, first: PreparedCallable, requesting: PreparedCallable, runtime: SymbolicProofRuntime<'_>,
        ranked: &mut HashMap<PreparedCallable, RankedOrderedSymbolic<'_, 'proof>>,
    ) -> Result<(), ProjectCheck> {
        let watched = self.watched;
        let mut stack: Vec<(Job<'_, '_, '_>, usize)> = Vec::new();
        let mut next = Some(first);
        loop {
            if let Some(current) = next.take() {
                charge_call_work(&mut self.qualified.work, 1).ok_or(ProjectCheck::Unsupported)?;
                if current == requesting || stack.iter().any(|(job, _)| job.target == current) {
                    return Err(ProjectCheck::Unsupported);
                }
                if self.qualified.verified.lookup(current).is_none() && !self.qualified.verdicts.contains_key(&current) {
                    if let Some(provider) = ranked.get_mut(&current) {
                        let source = &runtime.project.modules.get(current.0).ok_or(ProjectCheck::MissingRoot)?.source;
                        provider.verify_qualified_demand()?;
                        let summary = ranked_symbolic_equality(provider.plan, source).ok_or(ProjectCheck::Unsupported)?;
                        self.qualified.verified.insert(summary);
                    } else {
                        if stack.len() == MAX_SYMBOLIC_SUMMARY_LINEAGE { return Err(ProjectCheck::Unsupported); }
                        let job = Job::discover(watched, current, runtime).ok_or(ProjectCheck::Unsupported)?;
                        job.export_issuer().ok_or(ProjectCheck::Unsupported)?;
                        stack.push((job, 0));
                    }
                }
            }
            let Some((job, index)) = stack.last_mut() else { break; };
            if let Some(&edge) = job.origins.get(*index) {
                self.qualify_edge(job.owner, job.body, edge, runtime, ranked)?;
                *index += 1;
                next = Some(edge.0);
            } else {
                let (job, _) = stack.pop().ok_or(ProjectCheck::Unsupported)?;
                self.verify_qualified_job(job, runtime)?;
            }
        }
        if self.qualified.verified.lookup(first).is_none() {
            self.qualified.refuter(first)?;
        }
        Ok(())
    }

    fn verify_qualified_job(
        &mut self, job: Job<'proof, '_, 'case>, runtime: SymbolicProofRuntime<'_>,
    ) -> Result<(), ProjectCheck> {
        let index = std::mem::take(&mut self.qualified.verified);
        let previous_failure = self.failure.take();
        let evaluated = (|| {
            if job.case.bound.literal().is_none() {
                if !job.origins.is_empty() || job.export_issuer().is_none() {
                    return Err(ProjectCheck::Unsupported);
                }
                let checked = Self::prove_unary_entry_relationship(
                    job.target, job.body, &job.owner.source, runtime, job.case, &mut self.qualified.work,
                )?;
                let Some((entry_result, failure)) = checked else {
                    return Ok(Evaluation::Checked { active: false, reusable: false, entry_result: None });
                };
                if let Some(failure) = failure {
                    self.record(job.case, job.function, &job.owner.source, 0, failure, None)?;
                    return Ok(Evaluation::Checked { active: true, reusable: false, entry_result: None });
                }
                return Ok(Evaluation::Checked { active: true, reusable: true, entry_result: Some(entry_result) });
            }
            let mut imports = Vec::with_capacity(job.origins.len());
            for &(callee, position, origin, occurrence) in &job.origins {
                let Some(summary) = index.lookup(callee) else {
                    return Ok(Evaluation::Blocked(self.qualified.refuter(callee)?));
                };
                let import = summary.import_local(job.target, job.body, position, origin, occurrence, true, Some(job.owner.target_layout))?
                    .ok_or(ProjectCheck::Unsupported)?;
                imports.push(Some(import));
            }
            let mut scratch = OrderedFailures::default();
            let (active, reusable) = self.check_ranked_scalar_import(
                job.case, job.body, job.function, &job.owner.source, &mut scratch, &mut imports,
            )?;
            if !scratch.records.is_empty() { return Err(ProjectCheck::Unsupported); }
            Ok(Evaluation::Checked { active: active.unwrap_or(false), reusable, entry_result: None })
        })();
        let failure = self.failure.take();
        self.failure = previous_failure;
        self.qualified.verified = index;
        let (active, evidence) = match evaluated? {
            Evaluation::Blocked(refuter) if failure.is_none() => (false, Evidence::Blocked(refuter)),
            Evaluation::Blocked(_) => return Err(ProjectCheck::Unsupported),
            Evaluation::Checked { active, .. } if failure.is_some() => (active, Evidence::Refuted),
            Evaluation::Checked { active, reusable, entry_result } => {
                let certified = reusable && job.summary(entry_result).is_some_and(|summary| {
                    if job.origins.is_empty() {
                        self.qualified.verified.insert(summary);
                        true
                    } else {
                        self.qualified.verified.insert_dependency_links(summary, job.origins.iter().map(|provider| provider.0), true).is_some()
                    }
                });
                (active, if certified { Evidence::Certified } else { Evidence::Unavailable })
            }
        };
        if self.qualified.verdicts.insert(job.target, Verdict { active, scheduled: false, evidence, failure }).is_some() {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(())
    }

    pub(super) fn try_collect_in_catalog(
        &mut self, target: PreparedCallable, calls: &FunctionCallSites, function: &ScalarFunction,
        source: &SourceIdentity, effects: &mut OrderedFailures, runtime: SymbolicProofRuntime<'_>,
        ranked: &mut HashMap<PreparedCallable, RankedOrderedSymbolic<'_, 'proof>>,
    ) -> Result<Option<bool>, ProjectCheck> {
        if self.watched.qualified_catalog || !ranked.is_empty() {
            if let Some(active) = self.try_collect_qualified_provider(target, calls, source, effects, runtime, ranked)? {
                return Ok(Some(active));
            }
            if let Some(active) = self.try_collect_qualified_requirement(target, calls, source, effects, runtime, ranked)? {
                return Ok(Some(active));
            }
        }
        self.try_collect_legacy(target, calls, function, source, effects, runtime)
    }

    pub(super) fn try_collect_qualified_requirement(
        &mut self, target: PreparedCallable, calls: &FunctionCallSites, source: &SourceIdentity,
        effects: &mut OrderedFailures, runtime: SymbolicProofRuntime<'_>,
        ranked: &mut HashMap<PreparedCallable, RankedOrderedSymbolic<'_, 'proof>>,
    ) -> Result<Option<bool>, ProjectCheck> {
        let watched = self.watched;
        if target == watched.first.0 || watched.other.contains_key(&target)
            || !calls.checker_returns_unit() || !calls.checker_named_route_frame()
            || !calls.callable_escapes.is_empty() || !calls.structure.module_storage_uses.is_empty()
            || calls.contextual_formal().is_some()
        {
            return Ok(None);
        }
        let body = &calls.structure;
        let Some(root) = body.blocks.get(body.root.0) else { return Ok(None); };
        let [binding, evaluation] = root.items.as_slice() else { return Ok(None); };
        let (BlockItemKind::Binding(region), BlockItemKind::Evaluation { expression }) = (&binding.kind, &evaluation.kind) else {
            return Ok(None);
        };
        let [receiver] = region.receivers() else { return Ok(None); };
        let Some(origin) = body.checker_integer_call_locals.get(&(body.root, binding.position)) else { return Ok(None); };
        let Some(&callee) = body.checker_direct_targets.get(&origin.call) else { return Ok(None); };
        let [occurrence] = calls.call_occurrences.as_slice() else { return Ok(None); };
        let Some(directive) = body.checker_directives.get(expression) else { return Ok(None); };
        let Some(CheckerPredicate::Integer(predicate)) = &directive.predicate else { return Ok(None); };
        if callee.0 == target.0 || occurrence.ordinal != origin.call || body.checker_directives.len() != 1
            || directive.operation != CheckerOperation::Require || predicate.binding != origin.binding
            || !is_integer_type(&receiver.ty) || predicate.ty != receiver.ty || receiver.contextual_storage.is_some()
            || Job::discover(watched, callee, runtime).and_then(|job| job.export_issuer()).is_none()
        {
            return Ok(None);
        }
        let owner = runtime.project.modules.get(target.0).ok_or(ProjectCheck::MissingRoot)?;
        if owner.source != *source { return Err(ProjectCheck::Unsupported); }
        let edge = (callee, binding.position, origin, occurrence);
        self.qualify_edge(owner, calls, edge, runtime, ranked)?;
        self.ensure_qualified_provider(callee, target, runtime, ranked)?;
        let Some(summary) = self.qualified.verified.lookup(callee) else {
            // This exact requirement is gated by a refuted supplying proof.
            // The actual owner's diagnostic retains its original root order.
            self.qualified.refuter(callee)?;
            return Ok(Some(false));
        };
        let import = summary.import_local(target, calls, binding.position, origin, occurrence, false, Some(owner.target_layout))?
            .ok_or(ProjectCheck::Unsupported)?;
        let active = super::super::checker_flow::collect_ordered_imported_root_locals(
            calls, &mut [Some(import)], &mut |failure| effects.push_callable(calls, source, failure),
        ).ok_or(ProjectCheck::Unsupported)?;
        Ok(Some(active))
    }

    pub(super) fn try_collect_qualified_provider(
        &mut self, target: PreparedCallable, calls: &FunctionCallSites, source: &SourceIdentity,
        effects: &OrderedFailures, runtime: SymbolicProofRuntime<'_>,
        ranked: &mut HashMap<PreparedCallable, RankedOrderedSymbolic<'_, 'proof>>,
    ) -> Result<Option<bool>, ProjectCheck> {
        let watched = self.watched;
        let Some(job) = Job::discover(watched, target, runtime) else { return Ok(None); };
        if !std::ptr::eq(job.body, calls) || job.owner.source != *source { return Err(ProjectCheck::Unsupported); }
        if job.origins.is_empty() && job.export_issuer().is_none() { return Ok(None); }
        // Do not take over unrelated existing narrow qualified routes.
        // The old ranked route still owns wide imports; new unary
        // qualification needs an explicitly enrolled pure owner proof.
        if job.origins.iter().any(|(callee, _, origin, _)| {
            !matches!(origin.arguments, super::super::reference::CheckerIntegerCallArguments::Wide(_))
                && Job::discover(watched, *callee, runtime).and_then(|plan| plan.export_issuer()).is_none()
        }) {
            return Ok(None);
        }
        if !self.qualified.verdicts.contains_key(&target) {
            for &origin in &job.origins {
                self.qualify_edge(job.owner, job.body, origin, runtime, ranked)?;
                self.ensure_qualified_provider(origin.0, target, runtime, ranked)?;
            }
            self.verify_qualified_job(job, runtime)?;
        }
        let verdict = self.qualified.verdicts.get_mut(&target).ok_or(ProjectCheck::Unsupported)?;
        if std::mem::replace(&mut verdict.scheduled, true) { return Err(ProjectCheck::Unsupported); }
        let active = verdict.active;
        if let Some(mut failure) = verdict.failure.take() {
            if verdict.evidence != Evidence::Refuted { return Err(ProjectCheck::Unsupported); }
            failure.directive_position = effects.records.len();
            self.publish_failure(failure);
        } else if verdict.evidence == Evidence::Refuted {
            return Err(ProjectCheck::Unsupported);
        }
        Ok(Some(active))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_entry_dependent_summary_preserves_the_admitted_input_axis() {
        let app = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "main".into());
        let owner = SourceIdentity::new("project".into(), "package".into(), "src/lib.w".into(), "lib".into());
        let main = r#"%%start
            provider = namespace package "src/lib.w";
            u8(u8) bounded = fn(input) { u8 actual = provider.advance(input); actual };
            proof output_range for bounded(input) -> (out) where (input <= 7) ensures (out <= 8) {}
            unit() lower = fn() {
                u8 seen = provider.advance(2);
                core.require(seen == 3, { .stdout = ""; .stderr = "lower actual"; .code = 65; .files = []; });
            };
            unit() upper = fn() {
                u8 seen = provider.advance(7);
                core.require(seen == 8, { .stdout = ""; .stderr = "upper actual"; .code = 66; .files = []; });
            };
        %%end"#;
        let library = r#"%%start
            u8(u8) advance = fn(input) { u8 next = input + 1; next };
            proof increment for advance(input) -> (out) where (input <= 7) ensures (out == core.old(input) + 1) {}
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let run = |main: &str, library: &str, scope| {
                let parse = |id: &SourceIdentity, text: &str| {
                    crate::parse_proof_source(id.clone(), text.to_owned(), &[]).ok().unwrap().1
                };
                check_loaded_scalar_ordered_local_proof_contexts(
                    Vec::new(),
                    [(parse(&app, main), vec![ScalarNamespaceBinding {
                        binding: "provider".into(), target: owner.clone(), span: ByteSpan::new(0, 0),
                    }]), (parse(&owner, library), Vec::new())],
                    vec![app.clone(), owner.clone()], ScalarPreparationInputs { target_layout }, scope, Vec::new(),
                )
            };
            let scope = ScalarCheckerScope::CurrentPackage;
            let valid = run(main, library, scope).unwrap();
            assert!(valid.active && valid.directives.is_empty() && valid.symbolic_diagnostic.is_none());
            let contrary = main.replace("out <= 8", "out <= 7");
            let failure = run(&contrary, library, scope).unwrap().symbolic_diagnostic.unwrap();
            assert_eq!(failure.proof.source, app);
            assert_eq!(failure.returned, BigInt::from(8));
            let seven = BigInt::from(7);
            assert!(failure.witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
            let actual = main.replace("seen == 3", "seen == 4").replace("seen == 8","seen == 9");
            let failure = run(&actual, library, scope).unwrap();
            assert!(failure.symbolic_diagnostic.is_none());
            assert_eq!(failure.directives.len(), 2);
            for (record, (code, expected)) in failure.directives.iter().zip([(65, 3), (66, 8)]) {
                let expected = BigInt::from(expected);
                assert_eq!(record.code(), &BigInt::from(code));
                assert!(record.witness.intervals_at(0, 0).unwrap().eq([(&expected, &expected)]));
            }
            let bad_provider = library.replace("core.old(input) + 1", "core.old(input) + 2");
            let failure = run(&actual, &bad_provider, scope).unwrap();
            assert!(failure.directives.is_empty() && failure.additional_symbolic_diagnostics.is_empty());
            assert_eq!(failure.symbolic_diagnostic.unwrap().proof.source, owner);
            for unavailable in [
                library.replace("input <= 7", "input <= 6"),
                library.replace("input <= 7", "input <= 255"),
            ] {
                assert!(matches!(run(main, &unavailable, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
            let idle = run(&actual, &bad_provider, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!idle.active && idle.directives.is_empty() && idle.symbolic_diagnostic.is_none());
        }
    }

    #[test]
    fn qualified_dependency_diamond_reaches_ordinary_requirement() {
        let [app, hub, left, right, leaf] = ["main", "hub", "left", "right", "leaf"].map(|name| {
            SourceIdentity::new("project".into(), "package".into(), format!("src/{name}.w"), name.into())
        });
        let main = r#"%%start
            route = namespace package "src/hub.w";
            unit() ordinary = fn() {
                u8 observed = route.merge(2);
                core.require(observed == 7, { .stdout = ""; .stderr = "diamond result"; .code = 64; .files = []; });
            };
        %%end"#;
        let merge = r#"%%start
            first = namespace package "src/left.w";
            second = namespace package "src/right.w";
            u8(u8) merge = fn(input) {
                u8 before = first.relay(input);
                u8 after = second.relay(before);
                after
            };
            proof merge_result for merge(input) -> (out) where (input <= 3) ensures (out == 7) {}
        %%end"#;
        let relay = r#"%%start
            bottom = namespace package "src/leaf.w";
            u8(u8) relay = fn(input) { u8 returned = bottom.descend(input, 2, input, 4); returned };
            proof guarded for relay(input) -> (out) where (input <= 7) ensures (out == 7) {}
        %%end"#;
        let rank = r#"%%start
            u8(u8,u8,u8,u8) descend = fn(page, slot, n, quota) {
                if (n == 0) { 7 } else { descend(page, slot, n - 1, quota) }
            };
            proof induction for descend(page, slot, n, quota) -> (out)
                where (page <= 7 && slot <= 3 && n <= 7 && quota == 4)
                ensures (out == 7) decreases (n) {
                if (n == 0) { core.show(out == 7); }
                else { core.use(induction(page, slot, n - 1, quota)); }
            }
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let run = |main: &str, lhs: &str, rhs: &str, leaf_text: &str, scope| {
                let parse = |source: &SourceIdentity, text: &str| {
                    crate::parse_proof_source(source.clone(), text.to_owned(), &[]).ok().unwrap().1
                };
                let edge = |binding: &str, target: &SourceIdentity| ScalarNamespaceBinding {
                    binding: binding.to_owned(), target: target.clone(), span: ByteSpan::new(0, 0),
                };
                check_loaded_scalar_ordered_local_proof_contexts(
                    Vec::new(),
                    [
                        (parse(&app, main), vec![edge("route", &hub)]),
                        (parse(&hub, merge), vec![edge("first", &left), edge("second", &right)]),
                        (parse(&left, lhs), vec![edge("bottom", &leaf)]),
                        (parse(&right, rhs), vec![edge("bottom", &leaf)]),
                        (parse(&leaf, leaf_text), Vec::new()),
                    ],
                    // Consumer roots deliberately precede suppliers, while
                    // sibling diagnostic order is opposite to use order.
                    vec![app.clone(), hub.clone(), right.clone(), left.clone(), leaf.clone()],
                    ScalarPreparationInputs { target_layout }, scope, Vec::new(),
                )
            };
            let scope = ScalarCheckerScope::CurrentPackage;
            let receipt = run(main, relay, relay, rank, scope).unwrap();
            assert!(receipt.active && receipt.directives.is_empty() && receipt.symbolic_diagnostic.is_none());
            let contrary = main.replace("observed == 7", "observed == 8");
            let failure = run(&contrary, relay, relay, rank, scope).unwrap();
            assert_eq!(failure.directives.len(), 1);
            assert_eq!(failure.directives[0].code(), &BigInt::from(64));
            let seven = BigInt::from(7);
            assert!(failure.directives[0].witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
            assert!(failure.symbolic_diagnostic.is_none());
            let refuted = relay.replace("ensures (out == 7)", "ensures (out == 8)");
            let failure = run(&contrary, &refuted, &refuted, rank, scope).unwrap();
            assert!(failure.directives.is_empty());
            assert_eq!(failure.symbolic_diagnostic.unwrap().proof.source, right);
            assert_eq!(failure.additional_symbolic_diagnostics.len(), 1);
            assert_eq!(failure.additional_symbolic_diagnostics[0].proof.source, left);
            let uncovered = relay.replace("input <= 7", "input <= 6");
            assert!(matches!(run(main, relay, &uncovered, rank, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let idle = run(&contrary, &refuted, &refuted, rank, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!idle.active && idle.directives.is_empty() && idle.symbolic_diagnostic.is_none());

            let closed = r#"%%start
                u8(u8) fixed = fn(input) { if (input <= 7) { 7 } else { 9 } };
                proof bounded for fixed(input) -> (out) where (input <= 7) ensures (out == 7) {}
            %%end"#;
            let forwards = relay.replace("descend(input, 2, input, 4)", "fixed(input)");
            let ordinary = run(main, &forwards, &forwards, closed, scope).unwrap();
            assert!(ordinary.active && ordinary.directives.is_empty() && ordinary.symbolic_diagnostic.is_none());
            let failure = run(&contrary, &forwards, &forwards, closed, scope).unwrap();
            assert_eq!(failure.directives.len(), 1);
            assert_eq!(failure.directives[0].code(), &BigInt::from(64));
            assert!(failure.directives[0].witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
            assert!(failure.symbolic_diagnostic.is_none());
            let false_forward = forwards.replace("ensures (out == 7)", "ensures (out == 8)");
            let siblings = run(&contrary, &false_forward, &false_forward, closed, scope).unwrap();
            assert!(siblings.directives.is_empty());
            assert_eq!(siblings.symbolic_diagnostic.unwrap().proof.source, right);
            assert_eq!(siblings.additional_symbolic_diagnostics.len(), 1);
            assert_eq!(siblings.additional_symbolic_diagnostics[0].proof.source, left);
            let false_leaf = closed.replace("ensures (out == 7)", "ensures (out == 8)");
            let failed = run(&contrary, &forwards, &forwards, &false_leaf, scope).unwrap();
            assert!(failed.directives.is_empty() && failed.additional_symbolic_diagnostics.is_empty());
            let leaf_failure = failed.symbolic_diagnostic.unwrap();
            assert_eq!(leaf_failure.proof.source, leaf);
            assert_eq!(leaf_failure.returned, seven);
            let narrowed_leaf = closed.replace("where (input <= 7)", "where (input <= 6)");
            assert!(matches!(run(main, &forwards, &forwards, &narrowed_leaf, scope), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let dormant = run(&contrary, &forwards, &forwards, &false_leaf, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!dormant.active && dormant.directives.is_empty() && dormant.symbolic_diagnostic.is_none());
        }
    }

    #[test]
    fn replacing_one_qualified_ancestor_invalidates_the_transitive_lineage() {
        let identities = ["a", "b", "c", "other", "d", "e"].map(|name| {
            SourceIdentity::new("project".into(), "package".into(), format!("src/{name}.w"), name.into())
        });
        let declarations = identities.iter().map(|source| {
            let parsed = crate::parse_source(source.clone(), "%%start u8() fixed = fn { 7 }; %%end".to_owned(), &[]);
            (crate::derive_scalar_declarations_from_cst(parsed.result.canonical_cst()), Vec::new())
        }).collect::<Vec<_>>();
        let layout = ScalarTargetLayout::NATIVE64;
        let prepared = prepare_checker_project(declarations, identities.to_vec(), ScalarPreparationInputs { target_layout: layout }).unwrap();
        let (project, calls) = prepared.parts();
        let keys = project.modules.iter().map(|module| {
            let ScalarItem::Function(function) = &module.items[0] else { panic!("prepared fixed callable"); };
            ResolvedCallTarget::ModuleCallable {
                source: module.source.clone(), declaration_span: function.name_span,
                concrete: ConcreteCallSelection::Declaration,
            }
        }).collect::<Vec<_>>();
        let value = BigInt::from(7);
        let make = |module| VerifiedSymbolicSummary {
            target: (module, 0), canonical_target: &keys[module], layout,
            body: calls.function_body(module, 0, None).unwrap(),
            result: VerifiedSymbolicResult::Constant { bound: &value, shared: std::cell::OnceCell::new() },
            guards: if module == 0 || module == 3 { VerifiedSymbolicGuards::Bare(None) }
                else { VerifiedSymbolicGuards::Derived(Vec::new()) },
        };
        let mut index = SymbolicSummaryIndex::default();
        index.insert(make(0));
        index.insert(make(3));
        index.insert_linked(make(1), (0, 0), true).unwrap();
        index.insert_linked(make(2), (0, 0), true).unwrap();
        index.insert_dependency_links(make(4), [(1, 0), (2, 0)], true).unwrap();
        assert_eq!(index.derived.get(&(4, 0)).unwrap().as_slice(), &[((0, 0), 1), ((1, 0), 1), ((2, 0), 1)]);
        index.insert_linked(make(5), (4, 0), true).unwrap();
        index.insert(make(1));
        assert!(index.lookup((0, 0)).is_some() && index.lookup((2, 0)).is_some());
        assert!(index.lookup((4, 0)).is_none() && index.lookup((5, 0)).is_none());
        index.insert(make(0));
        assert!(index.lookup((2, 0)).is_none());
        assert!(index.lookup((3, 0)).is_some());
        assert!(index.insert_linked(make(1), (0, 0), true).is_none());
    }

    #[test]
    fn qualified_provider_demand_preserves_original_root_and_guard() {
        let app = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "app".into());
        let mid = SourceIdentity::new("project".into(), "package".into(), "src/relay.w".into(), "relay".into());
        let lib = SourceIdentity::new("project".into(), "package".into(), "src/lib.w".into(), "lib".into());
        let main = r#"%%start
            bridge = namespace package "src/relay.w";
            u8(u8) client = fn(input) { u8 observed = bridge.relay(input); observed };
            u8(u8) second = fn(input) { u8 observed = bridge.relay(input); observed };
            proof first_client for client(input) -> (out) where (input <= 2) ensures (out == 7) {}
            proof later_client for second(input) -> (out) where (input <= 1) ensures (out == 7) {}
        %%end"#;
        let relay = r#"%%start
            leaf = namespace package "src/lib.w";
            u8(u8) relay = fn(input) { u8 forwarded = leaf.descend(input, 2, input, 4); forwarded };
            proof exported for relay(input) -> (out) where (input <= 3) ensures (out == 7) {}
        %%end"#;
        let library = r#"%%start
            u8(u8,u8,u8,u8) descend = fn(page, slot, left, quota) {
                if (left == 0) { 7 } else { descend(page, slot, left - 1, quota) }
            };
            proof rank for descend(page, slot, left, quota) -> (out)
                where (page <= 3 && slot <= 3 && left <= 3 && quota == 4)
                ensures (out == 7) decreases (left) {
                if (left == 0) { core.show(out == 7); }
                else { core.use(rank(page, slot, left - 1, quota)); }
            }
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let run = |main: &str, relay: &str, library: &str, scope| {
                let parse = |identity: &SourceIdentity, code: &str| crate::parse_proof_source(identity.clone(), code.to_owned(), &[])
                    .ok().unwrap().1;
                let import = |binding: &str, target: &SourceIdentity| ScalarNamespaceBinding {
                    binding: binding.to_owned(), target: target.clone(), span: ByteSpan::new(0, 0),
                };
                check_loaded_scalar_ordered_local_proof_contexts(Vec::new(), [
                    (parse(&app, main), vec![import("bridge", &mid)]),
                    (parse(&mid, relay), vec![import("leaf", &lib)]),
                    (parse(&lib, library), Vec::new()),
                ], vec![app.clone(), mid.clone(), lib.clone()], ScalarPreparationInputs { target_layout }, scope, Vec::new())
            };
            let package = ScalarCheckerScope::CurrentPackage;
            let run_chain = |count: usize| {
                let mut owners = vec![mid.clone()];
                owners.extend((1..count).map(|index| {
                    SourceIdentity::new("project".into(), "package".into(), format!("src/relay{index}.w"), index.to_string())
                }));
                let parser = |source: &SourceIdentity, text: &str| {
                    crate::parse_proof_source(source.clone(), text.to_owned(), &[]).ok().unwrap().1
                };
                let edge = |binding: &str, target: &SourceIdentity| ScalarNamespaceBinding {
                    binding: binding.to_owned(), target: target.clone(), span: ByteSpan::new(0, 0),
                };
                let mut catalogs = vec![(parser(&app, main), vec![edge("bridge", &mid)])];
                for (index, owner) in owners.iter().enumerate() {
                    let (text, child) = if let Some(next) = owners.get(index + 1) {
                        (relay.replace("src/lib.w", &format!("src/relay{}.w", index + 1))
                            .replace("leaf.descend(input, 2, input, 4)", "leaf.relay(input)"), next)
                    } else {
                        (relay.to_owned(), &lib)
                    };
                    catalogs.push((parser(owner, &text), vec![edge("leaf", child)]));
                }
                catalogs.push((parser(&lib, library), Vec::new()));
                let order = std::iter::once(app.clone()).chain(owners).chain(std::iter::once(lib.clone())).collect();
                check_loaded_scalar_ordered_local_proof_contexts(
                    Vec::new(), catalogs, order, ScalarPreparationInputs { target_layout }, package, Vec::new(),
                )
            };
            let capped = run_chain(MAX_SYMBOLIC_SUMMARY_LINEAGE).unwrap();
            assert!(capped.active && capped.symbolic_diagnostic.is_none() && capped.directives.is_empty());
            assert!(matches!(run_chain(MAX_SYMBOLIC_SUMMARY_LINEAGE + 1), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let covered = run(main, relay, library, package).unwrap();
            assert!(covered.active && covered.directives.is_empty() && covered.symbolic_diagnostic.is_none());
            // No theorem is authored for this Unit root. A real ordered
            // requirement observes the exact result of its qualified call.
            let add_ordinary = |expected, argument| main.replace("%%end", &format!(r#"
                unit() ordinary = fn() {{
                    u8 seen = bridge.relay({argument});
                    core.require(seen == {expected}, {{ .stdout = ""; .stderr = "ordinary call result"; .code = 63; .files = []; }});
                }};
            %%end"#));
            let yes = run(&add_ordinary(7, 2), relay, library, package).unwrap();
            assert!(yes.active && yes.directives.is_empty() && yes.symbolic_diagnostic.is_none());
            let no = run(&add_ordinary(8, 2), relay, library, package).unwrap();
            assert_eq!(no.directives.len(), 1);
            let requirement = &no.directives[0];
            assert_eq!(requirement.code(), &BigInt::from(63));
            let seven = BigInt::from(7);
            assert!(requirement.witness.intervals_at(0, 0).unwrap().eq([(&seven, &seven)]));
            assert!(no.symbolic_diagnostic.is_none());
            assert!(matches!(run(&add_ordinary(7, 4), relay, library, package), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            let dormant = run(&add_ordinary(8, 2), relay, library, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!dormant.active && dormant.directives.is_empty() && dormant.symbolic_diagnostic.is_none());

            let bad_clients = main.replace("ensures (out == 7)", "ensures (out == 8)");
            let refuted = run(&bad_clients, relay, library, package).unwrap();
            let failures = refuted.symbolic_diagnostic.iter().chain(&refuted.additional_symbolic_diagnostics);
            assert!(failures.map(|failure| {
                assert_eq!(failure.returned, BigInt::from(7));
                assert_eq!(failure.proof.source, app);
                &bad_clients[failure.proof.range.start as usize..failure.proof.range.end as usize]
            }).zip(["proof first_client", "proof later_client"]).all(|(actual, expected)| actual.starts_with(expected)));
            assert_eq!(refuted.additional_symbolic_diagnostics.len(), 1);
            let bad_relay = relay.replace("ensures (out == 7)", "ensures (out == 8)");
            let blocked = run(&add_ordinary(8, 2), &bad_relay, library, package).unwrap();
            assert!(blocked.directives.is_empty() && blocked.additional_symbolic_diagnostics.is_empty());
            assert_eq!(blocked.symbolic_diagnostic.unwrap().proof.source, mid);
            let refuted = run(&bad_clients, &bad_relay, library, package).unwrap();
            assert!(refuted.active && refuted.additional_symbolic_diagnostics.is_empty() && refuted.directives.is_empty());
            let failure = refuted.symbolic_diagnostic.unwrap();
            assert_eq!(failure.proof.source, mid);
            assert_eq!(failure.returned, BigInt::from(7));
            let with_earlier_effect = bad_clients.replace("%%end", r#"
                core.require(false, { .stdout = ""; .stderr = "root output"; .code = 61; .files = []; });
            %%end"#);
            let ordered = run(&with_earlier_effect, &bad_relay, library, package).unwrap();
            assert_eq!(ordered.directives.len(), 1);
            assert_eq!(ordered.directives[0].code(), &BigInt::from(61));
            let provider = ordered.symbolic_diagnostic.unwrap();
            assert_eq!(provider.proof.source, mid);
            assert_eq!(provider.directive_position, 1);
            let idle = run(&bad_clients, &bad_relay, library, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!idle.active && idle.symbolic_diagnostic.is_none());
            for malformed in [
                relay.replace("where (input <= 3)", "where (input <= 4)"),
                relay.replace("input, 2, input, 4", "input, 2, input, 5"),
                relay.replace("forwarded };", "forwarded + 0 };"),
            ] {
                assert!(matches!(run(main, &malformed, library, package), Err(ScalarOrderedCheckerOutcome::Unsupported)));
            }
        }
    }
}
