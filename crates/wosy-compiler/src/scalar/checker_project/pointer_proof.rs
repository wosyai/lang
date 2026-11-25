//! The closed authored Unit effect route. No summary or real-call import is
//! issued: the exact selected symbolic root has one authenticated entry cell.
use super::*;
use super::super::{ScalarPlace, ScalarReferenceMutability};
use wosy_syntax::SourceSpan;

pub(super) struct Plan {
    target: PreparedCallable,
    proof: SourceSpan,
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
        Ok(Self { target: (selected.function.0, selected.function.1), proof: at(declaration), goal: at(&goal), amount: amount.clone() })
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
    fn observe(&mut self, _: PreparedCallable, _: &[&BigInt], _: &BigInt) {}
}

impl OrderedRunEvidence for &Plan {
    type Symbolic<'a> = Root<'a> where Self: 'a;
    type SymbolicReport = Option<PointerEffectProofDiagnostic>;
    const HAS_NAMED_ROOT: bool = true;

    fn complete_startup(&self, reached: &HashSet<PreparedCallable>) -> Result<bool, ProjectCheck> {
        // A physically reached pointer call may not borrow a symbolic atom
        // or this effect's receipt in place of its actual caller state.
        if reached.contains(&self.target) { return Err(ProjectCheck::Unsupported); }
        Ok(false)
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
        let [module] = runtime.project.modules.as_slice() else { return Err(ProjectCheck::Unsupported); };
        if &self.plan.proof.source != source || runtime.visits != Some(&[InitializationVisit::Completed])
            || module.items.len() > 32 || !module.structs.is_empty() || !module.enums.is_empty()
        {
            return Err(ProjectCheck::Unsupported);
        }
        // Bound the one opt-in project reference pass before requesting it.
        // There is no pass on a dormant proof or any other proof shape.
        let mut remaining = 512usize;
        for (item, declaration) in module.items.iter().enumerate() {
            if matches!(declaration, ScalarItem::Function(_)) {
                let sites = runtime.calls.function_body(target.0, item, None).ok_or(ProjectCheck::Unsupported)?;
                if !sites.arms.is_empty() { return Err(ProjectCheck::Unsupported); }
                remaining = remaining.checked_sub(sites.occurrences.len()).ok_or(ProjectCheck::Unsupported)?;
            }
        }
        if !std::ptr::eq(body, runtime.calls.function_body(target.0, target.1, None).ok_or(ProjectCheck::Unsupported)?) {
            return Err(ProjectCheck::Unsupported);
        }
        let validated = super::super::reference::analyze_project_references_with_call_points(
            &runtime.project.modules, runtime.calls, true,
        );
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
            let idle = run(&wrong, ScalarCheckerScope::MainFlowOnly).unwrap();
            assert!(!idle.active && idle.pointer_effect_diagnostic.is_none());

            for excluded in [
                program.replace("core.old(*p)", "*p"),
                program.replace("core.old(*p)", "core.old(*other)"),
                program.replace("= *p +@", "= *p +"),
                program.replace("*p = *p +@ 1;", "*p = *p +@ 1; *p = *p +@ 2;"),
                program.replace("ensures", "where (true) ensures"),
                program.replace("%%end", "unit() start = fn { u32 local = 15; increment(&!local); }; start(); %%end"),
            ] {
                assert!(run(&excluded, package).is_err(), "{excluded}");
            }
        }
    }
}
