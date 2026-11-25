//! The closed authored Unit effect route for sealed entry cells. A reached
//! call needs its own exact receipt for every independently lent formal.
use super::*;
use super::super::{ScalarPlace, ScalarReferenceMutability};
use super::super::checker_flow::LocalPointeeTransition as LocalTransition;
use wosy_syntax::SourceSpan;

pub(super) struct Plan {
    target: PreparedCallable,
    proof: SourceSpan,
    claims: Vec<PointeeClaim>,
    references: std::cell::OnceCell<super::super::reference::ProjectReferenceResults>,
    applied: std::cell::Cell<Option<(usize, usize)>>,
}

struct PointeeClaim {
    slot: usize,
    goal: SourceSpan,
    amount: BigInt,
}

pub(super) fn is_unit_target(selected: &SelectedModuleProof<'_>) -> bool {
    matches!(selected.body.boundary.results.as_slice(),
        [result] if result.ty == ScalarType::Unit)
}

impl Plan {
    pub(super) fn prepare(source: &SourceIdentity, selected: SelectedModuleProof<'_>) -> Result<Self, ProjectCheck> {
        let declaration = selected.node;
        let parameters = &selected.body.boundary.parameters;
        let limit = match parameters.len() { 1 => 48, 2 => 96, _ => return Err(ProjectCheck::Unsupported) };
        if source != &selected.owner.source || proof_target_tokens(declaration)?.0.is_some()
            || declaration.descendants().take(limit + 1).count() > limit
            || declaration.children().any(|node| matches!(node.kind(),
                SyntaxKind::ProofWhere | SyntaxKind::ProofForall | SyntaxKind::ProofDecreases))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let shape = derive_block(&child(declaration, SyntaxKind::Block)?);
        if !shape.items.is_empty() || shape.unsafe_context || !shape.final_output_values.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        if !selected.body.boundary.root_outputs.is_empty()
            || !proof_input_names(&child(declaration, SyntaxKind::ProofOutputs)?)?.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        let names = proof_input_names(&child(declaration, SyntaxKind::ProofTarget)?)?;
        if names.len() != parameters.len() || (names.len() == 2 && names[0] == names[1]) {
            return Err(ProjectCheck::InvalidProof);
        }
        let point = |expression: &ScalarExpression| {
            let ScalarExpression::Dereference { place: ScalarPlace::Dereference { pointer, .. }, .. } = expression
                else { return None; };
            let ScalarExpression::Name { name, .. } = pointer.as_ref() else { return None; };
            names.iter().position(|formal| name == formal)
        };
        let goal = child(&child(declaration, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
        let prepared = derive_expression(&goal);
        let goals = match (&prepared, parameters.len()) {
            (_, 1) => vec![&prepared],
            (ScalarExpression::Binary { operator: BinaryOperator::And, left, right, .. }, 2) =>
                vec![left.as_ref(), right.as_ref()],
            _ => return Err(ProjectCheck::Unsupported),
        };
        let mut claims = Vec::with_capacity(goals.len());
        for term in goals {
            let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } = term
                else { return Err(ProjectCheck::Unsupported); };
            let ScalarExpression::Binary { operator: BinaryOperator::WrappingAdd, left: old, right: amount, .. } = right.as_ref()
                else { return Err(ProjectCheck::Unsupported); };
            let ScalarExpression::Call { receiver: Some(core), name, arguments, type_arguments, .. } = old.as_ref()
                else { return Err(ProjectCheck::Unsupported); };
            let [snapshot] = arguments.as_slice() else { return Err(ProjectCheck::Unsupported); };
            let slot = point(left).ok_or(ProjectCheck::Unsupported)?;
            if point(snapshot) != Some(slot) || core != "core" || name != "old" || !type_arguments.is_empty()
                || claims.iter().any(|claim: &PointeeClaim| claim.slot == slot)
            {
                return Err(ProjectCheck::Unsupported);
            }
            let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner } = &parameters[slot].ty
                else { return Err(ProjectCheck::Unsupported); };
            if parameters[slot].ordinal != slot || !is_integer_type(inner) { return Err(ProjectCheck::Unsupported); }
            let ScalarExpression::Integer { value: amount, .. } = amount.as_ref() else {
                return Err(ProjectCheck::Unsupported);
            };
            if !integer_literal_fits_type(amount, inner) { return Err(ProjectCheck::InvalidProof); }
            let range = if parameters.len() == 1 { wosy_syntax::byte_span(&goal) }
                else { super::super::typecheck::expression_span(term) };
            claims.push(PointeeClaim { slot, goal: SourceSpan::new(source.clone(), range), amount: amount.clone() });
        }
        Ok(Self { target: (selected.function.0, selected.function.1),
            proof: SourceSpan::new(source.clone(), wosy_syntax::byte_span(declaration)), claims,
            references: std::cell::OnceCell::new(), applied: std::cell::Cell::new(None) })
    }

    fn references(
        &self, source: &SourceIdentity, runtime: SymbolicProofRuntime<'_>,
    ) -> Result<&super::super::reference::ProjectReferenceResults, ProjectCheck> {
        if let Some(known) = self.references.get() { return Ok(known); }
        let [module] = runtime.project.modules.as_slice() else { return Err(ProjectCheck::Unsupported); };
        if &self.proof.source != source || module.items.len() > 32 || !module.structs.is_empty() || !module.enums.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        // Bound the one opt-in project reference pass before requesting it.
        // There is no pass on a dormant proof or any other proof shape.
        let mut remaining = 512usize;
        for (item, declaration) in module.items.iter().enumerate() {
            if matches!(declaration, ScalarItem::Function(_)) {
                let sites = runtime.calls.function_body(self.target.0, item, None).ok_or(ProjectCheck::Unsupported)?;
                if !sites.arms.is_empty() { return Err(ProjectCheck::Unsupported); }
                remaining = remaining.checked_sub(sites.occurrences.len()).ok_or(ProjectCheck::Unsupported)?;
            }
        }
        Ok(self.references.get_or_init(|| super::super::reference::analyze_project_references_with_call_points(
            &runtime.project.modules, runtime.calls, true,
        )))
    }

    pub(super) fn run(
        &self, prepared: &PreparedCheckerProject, scope: ScalarCheckerScope,
    ) -> Result<MainFlowProofReceipt, ProjectCheck> {
        let run = run_prepared_ordered_check(prepared, scope, 1, OrderedReachPolicy::Closed, self)?;
        Ok(MainFlowProofReceipt {
            active: run.active, directives: run.directives, diagnostics: Vec::new(), contextual_diagnostic: None,
            aggregate_diagnostics: Vec::new(), symbolic_diagnostic: None,
            additional_symbolic_diagnostics: Vec::new(), entry_evidence: run.entry_evidence,
            pointer_effect_diagnostic: run.symbolic,
        })
    }
}

impl ActualIntegerObserver for &Plan {
    const ENABLED: bool = false;
    const EXACT_POINTEE_UPDATE: bool = true;
    const ORDINARY_INTEGER_LOCALS: bool = true;

    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}

    fn exact_pointee_update(
        &self, target: PreparedCallable, body: &FunctionCallSites, occurrence: OccurrenceOrdinal,
        source: &SourceIdentity, runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<Vec<LocalTransition>>, ProjectCheck> {
        if target != self.target { return Ok(None); }
        if &self.proof.source != source
            || !matches!(runtime.visits, Some([InitializationVisit::Started]))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let references = self.references(source, runtime)?;
        let module = runtime.project.modules.get(target.0).ok_or(ProjectCheck::Unsupported)?;
        let caller = module.items.iter().enumerate().find_map(|(item, decl)| {
            matches!(decl, ScalarItem::Function(_)).then_some(())?;
            std::ptr::eq(body, runtime.calls.function_body(target.0, item, None)?).then_some(item)
        }).ok_or(ProjectCheck::Unsupported)?;
        let mut amounts = self.claims.iter().map(|claim| (claim.slot, &claim.amount)).collect::<Vec<_>>();
        amounts.sort_unstable_by_key(|(slot, _)| *slot);
        let amounts = amounts.into_iter().map(|(_, amount)| amount).collect::<Vec<_>>();
        let receipts = references.pointer_caller_effects(
            &runtime.project.modules, runtime.calls, target.0, caller, target.1, &amounts, Some(occurrence),
        ).ok_or(ProjectCheck::Unsupported)?;
        let call = body.occurrences.get(occurrence.0).ok_or(ProjectCheck::Unsupported)?.span;
        // The receipt is immutable, but only the next real occurrence on
        // this exact caller frame may consume it. A second activation starts
        // with position zero and cannot borrow the previous epoch.
        let expected = match self.applied.get() {
            None => 0,
            Some((owner, next)) if owner == caller => next,
            _ => return Err(ProjectCheck::Unsupported),
        };
        if receipts.len() != self.claims.len() || receipts.iter().any(|receipt| {
            receipt.position() != expected || &receipt.call().source != source || receipt.call().range != call
        }) {
            return Err(ProjectCheck::Unsupported);
        }
        self.applied.set(Some((caller, expected.checked_add(1).ok_or(ProjectCheck::Unsupported)?)));
        Ok(Some(receipts.into_iter().map(|receipt| LocalTransition {
            binding: receipt.place().binding.clone(), ty: receipt.width().clone(),
            entry: receipt.entry().clone(), exit: receipt.exit().clone(),
        }).collect()))
    }
}

impl OrderedRunEvidence for &Plan {
    type Symbolic<'a> = Root<'a> where Self: 'a;
    type SymbolicReport = Option<PointerEffectProofDiagnostic>;
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if reached.contains(&self.target) != self.applied.get().is_some() { return Err(ProjectCheck::Unsupported); }
        Ok(self.applied.get().is_some())
    }
    fn symbolic(&self) -> Root<'_> { Root { plan: self, diagnostic: None } }
    fn finish_symbolic<'run>(root: Root<'run>) -> Self::SymbolicReport where Self: 'run { root.diagnostic }
}

pub(super) struct Root<'a> {
    plan: &'a Plan,
    diagnostic: Option<PointerEffectProofDiagnostic>,
}

impl SymbolicRootProof for Root<'_> {
    const ENABLED: bool = true;

    fn seed(&self, roots: &mut HashMap<usize, HashSet<usize>>, reached: &HashSet<PreparedCallable>) {
        let (module, item) = self.plan.target;
        if !reached.contains(&self.plan.target) { roots.entry(module).or_default().insert(item); }
    }

    fn try_collect(
        &mut self, target: PreparedCallable, body: &FunctionCallSites, _: &ScalarFunction,
        source: &SourceIdentity, _: &mut OrderedFailures, runtime: SymbolicProofRuntime<'_>,
    ) -> Result<Option<bool>, ProjectCheck> {
        if target != self.plan.target { return Ok(None); }
        if runtime.visits != Some(&[InitializationVisit::Completed])
            || !std::ptr::eq(body, runtime.calls.function_body(target.0, target.1, None).ok_or(ProjectCheck::Unsupported)?)
        {
            return Err(ProjectCheck::Unsupported);
        }
        let validated = self.plan.references(source, runtime)?;
        let updates = validated.pointer_entry_wrapping_effects(&runtime.project.modules, runtime.calls, target.0, target.1)
            .ok_or(ProjectCheck::Unsupported)?;
        if updates.len() != self.plan.claims.len() { return Err(ProjectCheck::Unsupported); }
        for claim in &self.plan.claims {
            let update = updates.get(claim.slot).ok_or(ProjectCheck::Unsupported)?;
            if &update.formal().source != source || !integer_literal_fits_type(&claim.amount, update.width()) {
                return Err(ProjectCheck::InvalidProof);
            }
            if update.offset() != &claim.amount {
                // The per-place offset gives a genuine counterexample at
                // entry zero for that place; report the first authored claim.
                self.diagnostic = Some(PointerEffectProofDiagnostic {
                    proof: self.plan.proof.clone(), goal: claim.goal.clone(),
                    read: update.read().clone(), write: update.write().clone(),
                    entry: BigInt::from(0), expected: claim.amount.clone(), observed: update.offset().clone(),
                });
                break;
            }
        }
        Ok(Some(true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_actual_pointer_update_changes_only_the_borrowed_local_before_require() {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r8".into());
        let program = r#"%%start
            unit(*!u32) increment = fn(p) { *p = *p +@ 1; };
            proof increment_effect for increment(p) -> ()
                ensures (*p == core.old(*p) +@ 1) {}
            unit() start = fn {
                u32 local = 4294967295;
                increment(&!local);
                core.require(local == 0, { .stdout = ""; .stderr = "wrong exit"; .code = 1; .files = []; });
            };
            start();
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let run = |text: &str, scope| {
                let (_, proof) = crate::parse_proof_source(source.clone(), text.to_owned(), &[]).ok().unwrap();
                check_loaded_scalar_ordered_local_proof_contexts(Vec::new(), vec![(proof, Vec::new())],
                    vec![source.clone()], ScalarPreparationInputs { target_layout }, scope, Vec::new())
            };
            for scope in [ScalarCheckerScope::CurrentPackage, ScalarCheckerScope::MainFlowOnly] {
                let approved = run(program, scope).unwrap();
                assert!(approved.active && approved.directives.is_empty() && approved.pointer_effect_diagnostic.is_none());
                // u32::MAX + 1 wraps to 0; the second write uses that
                // current value, while core.old is still u32::MAX.
                let ordered = program.replace("*p = *p +@ 1;", "*p = *p +@ 1; *p = *p +@ 2;")
                    .replace("core.old(*p) +@ 1", "core.old(*p) +@ 3")
                    .replace("core.require(local == 0", "core.require(local == 2");
                let approved = run(&ordered, scope).unwrap();
                assert!(approved.active && approved.directives.is_empty() && approved.pointer_effect_diagnostic.is_none());
                let stale = ordered.replace("core.require(local == 2", "core.require(local == 0");
                let witnessed = run(&stale, scope).unwrap();
                let [failed] = witnessed.directives.as_slice() else { panic!("ordered caller requirement"); };
                assert_eq!(&stale[failed.condition.start as usize..failed.condition.end as usize], "local == 0");
                let two = BigInt::from(2);
                assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&two, &two)]));

                let repeated = ordered.replace("increment(&!local);", "increment(&!local); increment(&!local);")
                    .replace("local == 2", "local == 5");
                let approved = run(&repeated, scope).unwrap();
                assert!(approved.active && approved.directives.is_empty() && approved.pointer_effect_diagnostic.is_none());
                let first_exit = repeated.replace("local == 5", "local == 2");
                let refuted = run(&first_exit, scope).unwrap();
                let [failed] = refuted.directives.as_slice() else { panic!("second exact call exit"); };
                let five = BigInt::from(5);
                assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&five, &five)]));
                let overwritten = repeated.replace("increment(&!local); increment", "increment(&!local); local = 9; increment")
                    .replace("local == 5", "local == 12");
                let approved = run(&overwritten, scope).unwrap();
                assert!(approved.active && approved.directives.is_empty() && approved.pointer_effect_diagnostic.is_none());
                let stale = overwritten.replace("local == 12", "local == 5");
                let refuted = run(&stale, scope).unwrap();
                let [failed] = refuted.directives.as_slice() else { panic!("the overwrite kills the preceding cell value"); };
                let twelve = BigInt::from(12);
                assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&twelve, &twelve)]));
                for excluded in [
                    repeated.replace("increment(&!local); increment", "increment(&!local); local = local +@ 1; increment"),
                    repeated.replace("            start();", "            start(); start();"),
                ] {
                    assert!(run(&excluded, scope).is_err(), "{excluded}");
                }

                let separate = overwritten
                    .replace("u32 local = 4294967295;", "u32 local = 4294967295; u32 another = 10;")
                    .replace("local = 9;", "increment(&!another); local = 9;")
                    .replace("            };", r#"                core.require(another == 13, { .stdout = ""; .stderr = "other exit"; .code = 1; .files = []; });
            };"#);
                let approved = run(&separate, scope).unwrap();
                assert!(approved.active && approved.directives.is_empty() && approved.pointer_effect_diagnostic.is_none());
                for (from, bad) in [("local == 12", "local == 5"), ("another == 13", "another == 10")] {
                    let stale = separate.replace(from, bad);
                    let report = run(&stale, scope).unwrap();
                    let [failed] = report.directives.as_slice() else { panic!("each cell needs its own update"); };
                    assert_eq!(&stale[failed.condition.start as usize..failed.condition.end as usize], bad);
                    let twelve = BigInt::from(12);
                    let thirteen = BigInt::from(13);
                    assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&twelve, &twelve)]));
                    assert!(failed.witness.intervals_at(0, 1).unwrap().eq([(&thirteen, &thirteen)]));
                }
                let wrong_width = separate.replace("u32 another = 10;", "u16 another = 10;");
                assert!(run(&wrong_width, scope).is_err());
                let unrelated_write = separate.replace("increment(&!another); local = 9;",
                    "another = 9; increment(&!another); local = 9;");
                assert!(run(&unrelated_write, scope).is_err());

                let old = program.replace("core.require(local == 0", "core.require(local == 4294967295");
                let witnessed = run(&old, scope).unwrap();
                assert!(witnessed.pointer_effect_diagnostic.is_none());
                let [failed] = witnessed.directives.as_slice() else { panic!("exact caller requirement"); };
                assert_eq!(failed.source, source);
                assert_eq!(&old[failed.condition.start as usize..failed.condition.end as usize], "local == 4294967295");
                let zero = BigInt::from(0);
                assert!(failed.witness.intervals_at(0, 0).unwrap().eq([(&zero, &zero)]));

                for unsupported in [
                    program.replace("core.old(*p) +@ 1", "core.old(*p) +@ 2"),
                    program.replace("increment(&!local);", "*!u32 alias = &!local; increment(alias);"),
                    program.replace("increment(&!local);", "local = 2; increment(&!local);"),
                    program.replace("            start();", "            start(); start();"),
                ] {
                    assert!(run(&unsupported, scope).is_err(), "{unsupported}");
                }
            }
        }
    }

    #[test]
    fn authored_unit_pointer_old_seals_distinct_formal_epochs() {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "dual-formal".into());
        let program = r#"%%start
            unit(*!u8, *!u32) adjust = fn(p, q) {
                *p = *p +@ 255;
                *q = *q +@ 2;
                *p = *p +@ 2;
            };
            proof snapshots for adjust(first, second) -> ()
                ensures ((*second == core.old(*second) +@ 2) && (*first == core.old(*first) +@ 1)) {}
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let run = |text: &str, scope| {
                let (_, proof) = crate::parse_proof_source(source.clone(), text.to_owned(), &[]).ok().unwrap();
                check_loaded_scalar_ordered_local_proof_contexts(Vec::new(), vec![(proof, Vec::new())],
                    vec![source.clone()], ScalarPreparationInputs { target_layout }, scope, Vec::new())
            };
            let package = ScalarCheckerScope::CurrentPackage;
            let pass = run(program, package).unwrap();
            assert!(pass.active && pass.directives.is_empty() && pass.pointer_effect_diagnostic.is_none());
            for (previous, wrong, read) in [
                ("core.old(*second) +@ 2", "core.old(*second) +@ 4", "*q"),
                ("core.old(*first) +@ 1", "core.old(*first) +@ 3", "*p"),
            ] {
                let failed = program.replace(previous, wrong);
                let receipt = run(&failed, package).unwrap().pointer_effect_diagnostic.unwrap();
                let snippet = |span: &SourceSpan| &failed[span.range.start as usize..span.range.end as usize];
                assert!(snippet(&receipt.goal).ends_with(wrong));
                assert_eq!(snippet(&receipt.read), read);
                assert_eq!(snippet(&receipt.write), read);
                assert_eq!(receipt.entry, BigInt::from(0));
                let dormant = run(&failed, ScalarCheckerScope::MainFlowOnly).unwrap();
                assert!(!dormant.active && dormant.pointer_effect_diagnostic.is_none());
            }
            let reached = program.replace("%%end", r#"
                unit() start = fn {
                    u8 a = 255;
                    u32 b = 42;
                    adjust(&!a, &!b);
                    core.require(a == 0, { .stdout = ""; .stderr = "first"; .code = 1; .files = []; });
                    core.require(b == 44, { .stdout = ""; .stderr = "second"; .code = 1; .files = []; });
                };
                start();
            %%end"#);
            for scope in [ScalarCheckerScope::MainFlowOnly, package] {
                let pass = run(&reached, scope).unwrap();
                assert!(pass.active && pass.directives.is_empty() && pass.pointer_effect_diagnostic.is_none());
                for (expected, stale) in [("a == 0", "a == 255"), ("b == 44", "b == 42")] {
                    let wrong = reached.replace(expected, stale);
                    let failure = run(&wrong, scope).unwrap();
                    let [failed] = failure.directives.as_slice() else { panic!("exact per-argument requirement"); };
                    assert_eq!(&wrong[failed.condition.start as usize..failed.condition.end as usize], stale);
                }
                for refused in [
                    reached.replace("*!u32", "*!u8").replace("adjust(&!a, &!b)", "adjust(&!a, &!a)"),
                    reached.replace("adjust(&!a, &!b)", "adjust(&!b, &!a)"),
                    reached.replace("core.old(*second) +@ 2", "core.old(*second) +@ 4"),
                    reached.replace("start();", "start(); start();"),
                ] {
                    assert!(run(&refused, scope).is_err(), "{refused}");
                }
            }
            for unsupported in [
                program.replace("*first == core.old(*first)", "*first == core.old(*second)"),
                program.replace("*first == core.old(*first) +@ 1", "*second == core.old(*second) +@ 2"),
                program.replace("*!u32", "*!u8").replace("*p = *p +@ 2", "*p = *q +@ 2"),
                program.replace("*q = *q +@ 2;", ""),
            ] {
                assert!(run(&unsupported, package).is_err(), "{unsupported}");
            }
        }
    }

    #[test]
    fn authored_unit_pointer_old_observes_one_sealed_entry_epoch() {
        let source = SourceIdentity::new("project".into(), "package".into(), "src/main.w".into(), "r1".into());
        let program = r#"%%start
            unit(*!u32) increment = fn(p) { *p = *p +@ 1; };
            proof increment_effect for increment(p) -> ()
                ensures (*p == core.old(*p) +@ 1) {}
        %%end"#;
        for target_layout in [ScalarTargetLayout::NATIVE64, ScalarTargetLayout::WASM32] {
            let run = |text: &str, scope| {
                let (_, proof) = crate::parse_proof_source(source.clone(), text.to_owned(), &[]).ok().unwrap();
                check_loaded_scalar_ordered_local_proof_contexts(Vec::new(), vec![(proof, Vec::new())],
                    vec![source.clone()], ScalarPreparationInputs { target_layout }, scope, Vec::new())
            };
            let package = ScalarCheckerScope::CurrentPackage;
            let approved = run(program, package).unwrap();
            assert!(approved.active && approved.directives.is_empty() && approved.pointer_effect_diagnostic.is_none());
            let alias = program.replace("increment(p) ->", "increment(arg) ->")
                .replace("ensures (*p == core.old(*p)", "ensures (*arg == core.old(*arg)");
            assert!(run(&alias, package).unwrap().active);
            let wrong = program.replace("core.old(*p) +@ 1", "core.old(*p) +@ 2");
            let failure = run(&wrong, package).unwrap().pointer_effect_diagnostic.unwrap();
            assert_eq!(failure.entry, BigInt::from(0));
            assert_eq!(failure.observed, BigInt::from(1));
            assert_eq!(failure.expected, BigInt::from(2));
            let slice = |span: &SourceSpan| &wrong[span.range.start as usize..span.range.end as usize];
            assert_eq!(slice(&failure.goal), "*p == core.old(*p) +@ 2");
            assert_eq!(slice(&failure.read), "*p");
            assert_eq!(slice(&failure.write), "*p");
            assert_ne!(failure.read, failure.write);
            assert_eq!(failure.read.source, source);
            let wrong_final = program.replace("*p = *p +@ 1;", "*p = *p +@ 1; *p = *p +@ 2;")
                .replace("core.old(*p) +@ 1", "core.old(*p) +@ 2");
            let final_failure = run(&wrong_final, package).unwrap().pointer_effect_diagnostic.unwrap();
            assert_eq!(final_failure.entry, BigInt::from(0));
            assert_eq!(final_failure.observed, BigInt::from(3));
            assert_eq!(final_failure.expected, BigInt::from(2));
            assert_eq!(final_failure.read.range.start as usize, wrong_final.find("= *p +@ 1").unwrap() + 2);
            assert_eq!(final_failure.write.range.start as usize, wrong_final.find("*p = *p +@ 2").unwrap());
            let idle = run(&wrong, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!idle.active && idle.pointer_effect_diagnostic.is_none());

            for excluded in [
                program.replace("core.old(*p)", "*p"),
                program.replace("core.old(*p)", "core.old(*other)"),
                program.replace("= *p +@", "= *p +"),
                program.replace("*p = *p +@ 1;", "*p = *p +@ 1; *p = *p + 2;"),
                program.replace("ensures", "where (true) ensures"),
                program.replace("%%end", "unit() start = fn { u32 local = 15; local = 16; increment(&!local); }; start(); %%end"),
            ] {
                assert!(run(&excluded, package).is_err(), "{excluded}");
            }
        }
    }
}
