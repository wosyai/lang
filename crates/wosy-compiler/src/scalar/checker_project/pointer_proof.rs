//! The closed authored Unit effect route for one sealed entry cell. This
//! installs no reusable summary; a reached call needs its own exact receipt.
use super::*;
use super::super::{ScalarPlace, ScalarReferenceMutability};
use wosy_syntax::SourceSpan;

pub(super) struct Plan {
    target: PreparedCallable,
    proof: SourceSpan,
    goal: SourceSpan,
    amount: BigInt,
    references: std::cell::OnceCell<super::super::reference::ProjectReferenceResults>,
    applied: std::cell::Cell<bool>,
}

pub(super) struct LocalTransition {
    pub(super) binding: ReferenceBindingId,
    pub(super) ty: ScalarType,
    pub(super) entry: BigInt,
    pub(super) exit: BigInt,
}

pub(super) fn is_unit_target(selected: &SelectedModuleProof<'_>) -> bool {
    matches!(selected.body.boundary.results.as_slice(),
        [result] if result.ty == ScalarType::Unit)
}

impl Plan {
    pub(super) fn prepare(source: &SourceIdentity, selected: SelectedModuleProof<'_>) -> Result<Self, ProjectCheck> {
        let declaration = selected.node;
        if source != &selected.owner.source || proof_target_tokens(declaration)?.0.is_some()
            || declaration.descendants().take(49).count() > 48
            || declaration.children().any(|node| matches!(node.kind(),
                SyntaxKind::ProofWhere | SyntaxKind::ProofForall | SyntaxKind::ProofDecreases))
        {
            return Err(ProjectCheck::Unsupported);
        }
        let shape = derive_block(&child(declaration, SyntaxKind::Block)?);
        if !shape.items.is_empty() || shape.unsafe_context || !shape.final_output_values.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        let [parameter] = selected.body.boundary.parameters.as_slice() else { return Err(ProjectCheck::Unsupported); };
        let ScalarType::CheckedReference { mutability: ScalarReferenceMutability::Mutable, inner } = &parameter.ty
            else { return Err(ProjectCheck::Unsupported); };
        if parameter.ordinal != 0 || !is_integer_type(inner) || !selected.body.boundary.root_outputs.is_empty()
            || !proof_input_names(&child(declaration, SyntaxKind::ProofOutputs)?)?.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        let names = proof_input_names(&child(declaration, SyntaxKind::ProofTarget)?)?;
        let [formal] = names.as_slice() else { return Err(ProjectCheck::InvalidProof); };
        let point = |expression: &ScalarExpression| {
            matches!(expression, ScalarExpression::Dereference {
                place: ScalarPlace::Dereference { pointer, .. }, ..
            } if matches!(pointer.as_ref(), ScalarExpression::Name { name, .. } if name.as_str() == formal.as_str()))
        };
        let goal = child(&child(declaration, SyntaxKind::ProofEnsures)?, SyntaxKind::Expression)?;
        let ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. } =
            derive_expression(&goal) else { return Err(ProjectCheck::Unsupported); };
        let ScalarExpression::Binary { operator: BinaryOperator::WrappingAdd, left: old, right: amount, .. } = right.as_ref()
            else { return Err(ProjectCheck::Unsupported); };
        let ScalarExpression::Call { receiver: Some(core), name, arguments, type_arguments, .. } = old.as_ref()
            else { return Err(ProjectCheck::Unsupported); };
        let [snapshot] = arguments.as_slice() else { return Err(ProjectCheck::Unsupported); };
        if !point(&left) || !point(snapshot) || core != "core" || name != "old" || !type_arguments.is_empty() {
            return Err(ProjectCheck::Unsupported);
        }
        let ScalarExpression::Integer { value: amount, .. } = amount.as_ref() else {
            return Err(ProjectCheck::Unsupported);
        };
        if !integer_literal_fits_type(amount, inner) { return Err(ProjectCheck::InvalidProof); }
        let at = |node| SourceSpan::new(source.clone(), wosy_syntax::byte_span(node));
        Ok(Self { target: (selected.function.0, selected.function.1), proof: at(declaration), goal: at(&goal), amount: amount.clone(),
            references: std::cell::OnceCell::new(), applied: std::cell::Cell::new(false) })
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
    ) -> Result<Option<LocalTransition>, ProjectCheck> {
        if target != self.target { return Ok(None); }
        if self.applied.get() || &self.proof.source != source
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
        let receipt = references.pointer_caller_effect(
            &runtime.project.modules, runtime.calls, target.0, caller, target.1, &self.amount,
        ).ok_or(ProjectCheck::Unsupported)?;
        let call = body.occurrences.get(occurrence.0).ok_or(ProjectCheck::Unsupported)?.span;
        if &receipt.call().source != source || receipt.call().range != call { return Err(ProjectCheck::Unsupported); }
        self.applied.set(true);
        Ok(Some(LocalTransition {
            binding: receipt.place().binding.clone(), ty: receipt.width().clone(),
            entry: receipt.entry().clone(), exit: receipt.exit().clone(),
        }))
    }
}

impl OrderedRunEvidence for &Plan {
    type Symbolic<'a> = Root<'a> where Self: 'a;
    type SymbolicReport = Option<PointerEffectProofDiagnostic>;
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        if reached.contains(&self.target) != self.applied.get() { return Err(ProjectCheck::Unsupported); }
        Ok(self.applied.get())
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
        let update = validated.pointer_entry_wrapping_write(&runtime.project.modules, runtime.calls, target.0, target.1)
            .ok_or(ProjectCheck::Unsupported)?;
        if &update.formal().source != source || !integer_literal_fits_type(&self.plan.amount, update.width()) {
            return Err(ProjectCheck::InvalidProof);
        }
        if update.offset() != &self.plan.amount {
            // Both constants fit the same integer width. At the admitted
            // entry value 0 each post-value is exactly its offset, making the
            // disagreement an actual counterexample to the symbolic goal.
            self.diagnostic = Some(PointerEffectProofDiagnostic {
                proof: self.plan.proof.clone(), goal: self.plan.goal.clone(),
                read: update.read().clone(), write: update.write().clone(),
                entry: BigInt::from(0), expected: self.plan.amount.clone(), observed: update.offset().clone(),
            });
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
