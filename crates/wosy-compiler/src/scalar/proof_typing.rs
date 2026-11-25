//! Static proof typing for Build. Nothing in this module verifies a goal or
//! contributes occurrences, effects, or values to the runtime project.
use super::{
    declarations::ScalarDeclarationEnvironment, overload::ScalarOverloadEnvironment,
    proof_match::{AuthoredMatch, Selector},
    typecheck::{expression_span, expression_type_in_module_expected, expect_module_type, is_integer_type,
        module_diagnostic, validate_module_type, ScalarModuleTyping},
    *,
};

pub(super) fn validate_build_proofs(
    proofs: &HashMap<SourceIdentity, Vec<CstNode>>,
    modules: &[ScalarModuleTyping<'_>],
    runtime_scopes: &[BTreeMap<String, ScalarType>],
    declarations: &ScalarDeclarationEnvironment,
    overloads: &ScalarOverloadEnvironment,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let indexes: HashMap<_, _> = modules.iter().enumerate().map(|(index, module)| (module.source, index)).collect();
    let mut proof_names_by_module = HashMap::new();
    let mut all_headers = Vec::new();
    for (ordinal, module) in modules.iter().enumerate() {
        let Some(proofs) = proofs.get(module.source) else { continue; };
        let mut proof_names = BTreeMap::new();
        let mut headers = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let proof_name = proof.children_with_tokens().filter_map(|element| element.into_token())
                .find(|token| token.kind() == SyntaxKind::Identifier).expect("parsed proof name");
            let target = node(proof, SyntaxKind::ProofTarget);
            let qualified = node(&target, SyntaxKind::QualifiedCall);
            let mut target_parts = qualified.children_with_tokens().filter_map(|element| element.into_token())
                .filter(|token| token.kind() == SyntaxKind::Identifier);
            let first = target_parts.next().expect("parsed target");
            let second = target_parts.next();
            let (receiver, name) = second.as_ref().map_or((None, first.text()), |member| (Some(first.text()), member.text()));
            let target_ordinal = match receiver {
                None => Some(ordinal),
                Some(receiver) => module.namespace_bindings.iter().find(|binding| binding.binding == receiver)
                    .and_then(|binding| indexes.get(&binding.target).copied()),
            };
            let target_type = target_ordinal.and_then(|target| {
                let target_typing = overloads.typing(target);
                let declared = if target == ordinal && receiver.is_none() {
                    runtime_scopes[target].get(name)
                } else {
                    modules[target].members.get(name)
                }?;
                // There are no argument expressions or type arguments in a
                // proof header. A single concrete checked overload has a
                // canonical signature; heterogeneous/generic schemes do not.
                if let Some(overload) = overloads.for_call(module.source, receiver, name) {
                    let [arm] = overload.arms.as_slice() else { return None; };
                    return arm.generic_parameters.is_empty().then_some(&arm.signature);
                }
                // The canonical registry records every declared function,
                // including an empty generic parameter list.
                if target_typing.generics(name).is_some_and(|parameters| !parameters.is_empty()) {
                    return None;
                }
                Some(declared)
            });
            let Some(ScalarType::Callable { outputs, parameters }) = target_type else {
                diagnostics.push(module_diagnostic(module, "B0003", "proof target must name one resolved nongeneric callable signature", wosy_syntax::byte_span(&qualified)));
                continue;
            };
            if proof_names.insert(proof_name.text().to_owned(), parameters.as_slice()).is_some() {
                diagnostics.push(module_diagnostic(module, "B0002", "duplicate proof name", wosy_syntax::byte_span(proof)));
                continue;
            }
            let (Some(inputs), Some(result_names)) = (
                parameter_names(&target), parameter_names(&node(proof, SyntaxKind::ProofOutputs)),
            ) else {
                diagnostics.push(module_diagnostic(module, "B0004", "invalid proof parameter list", wosy_syntax::byte_span(proof)));
                continue;
            };
            let is_unit = outputs.outputs.len() == 1 && outputs.outputs[0].ty == ScalarType::Unit;
            let declared_null = is_unit && result_names.as_slice() == ["null"];
            let result_count = if is_unit { 0 } else { outputs.outputs.len() };
            if inputs.len() != parameters.len() || (!declared_null && result_names.len() != result_count) {
                diagnostics.push(module_diagnostic(module, "B0004", "proof names must match callable input and output arity", wosy_syntax::byte_span(proof)));
                continue;
            }
            headers.push((proof, inputs.into_iter().zip(parameters), result_names.into_iter()
                .filter(move |_| !declared_null).zip(&outputs.outputs)));
        }
        proof_names_by_module.insert(module.source, proof_names);
        all_headers.push((ordinal, headers));
    }
    for (ordinal, headers) in all_headers {
        let module = &modules[ordinal];
        let typing = overloads.typing(ordinal);
        let owners = ScalarAssignmentOwners::new(&[]);
        for (proof, inputs, results) in headers {
            let mut check = ProofTyping { module, modules, typing: &typing, owners: &owners, diagnostics,
                declarations, ordinal, proof_names: &proof_names_by_module, entry_scope: BTreeMap::new() };
            let mut scope = BTreeMap::new();
            for (name, ty) in inputs { check.bind(&mut scope, name, ty.clone(), proof); }
            check.entry_scope = scope.clone();
            if let Some(guard) = optional_node(proof, SyntaxKind::ProofWhere) {
                check.check_cst(&node(&guard, SyntaxKind::Expression), Some(&ScalarType::Bool), &scope);
            }
            if let Some(decreases) = optional_node(proof, SyntaxKind::ProofDecreases) {
                for measure in decreases.children().filter(|child| child.kind() == SyntaxKind::Expression) {
                    let ty = check.check_cst(&measure, None, &scope);
                    if ty != ScalarType::Error && !is_integer_type(&ty) && !matches!(ty, ScalarType::F32 | ScalarType::F64) {
                        check.error(&measure, "proof decreases measure must be numeric");
                    }
                }
            }
            for (name, output) in results { check.bind(&mut scope, name, output.ty.clone(), proof); }
            for forall in proof.children().filter(|child| child.kind() == SyntaxKind::ProofForall) {
                check.quantifier(&forall, &mut scope);
            }
            let ensures = node(proof, SyntaxKind::ProofEnsures);
            let witness_type = if let Some(exists) = optional_node(&ensures, SyntaxKind::ProofExists) {
                let mut existential = scope.clone();
                let ty = check.quantifier(&exists, &mut existential);
                check.check_cst(&node(&exists, SyntaxKind::Expression), Some(&ScalarType::Bool), &existential);
                Some(ty)
            } else {
                check.check_cst(&node(&ensures, SyntaxKind::Expression), Some(&ScalarType::Bool), &scope);
                None
            };
            if let Some(body) = optional_node(proof, SyntaxKind::Block) {
                check.steps(&derive_block(&body), &scope, witness_type.as_ref());
            } else if let Some(matching) = optional_node(proof, SyntaxKind::ProofMatchBlock) {
                check.check_match(&matching, &scope, witness_type.as_ref());
            } else {
                check.error(proof, "missing proof body");
            }
        }
    }
}

struct ProofTyping<'a, 'd> {
    module: &'a ScalarModuleTyping<'a>,
    modules: &'a [ScalarModuleTyping<'a>],
    typing: &'a overload::ScalarTypingView<'a>,
    owners: &'a ScalarAssignmentOwners,
    diagnostics: &'d mut Vec<Diagnostic>,
    declarations: &'a ScalarDeclarationEnvironment,
    ordinal: usize,
    proof_names: &'a HashMap<&'a SourceIdentity, BTreeMap<String, &'a [ScalarType]>>,
    entry_scope: BTreeMap<String, ScalarType>,
}

impl ProofTyping<'_, '_> {
    fn error(&mut self, node: &CstNode, message: &str) {
        self.diagnostics.push(module_diagnostic(self.module, "B0003", message, wosy_syntax::byte_span(node)));
    }

    fn bind(&mut self, scope: &mut BTreeMap<String, ScalarType>, name: String, ty: ScalarType, node: &CstNode) {
        if scope.insert(name, ty).is_some() {
            self.error(node, "duplicate proof binder");
        }
    }

    fn quantifier(&mut self, quantifier: &CstNode, scope: &mut BTreeMap<String, ScalarType>) -> ScalarType {
        let declared = node(quantifier, SyntaxKind::TypeSpec);
        let ty = resolution::resolve_type_in_project(&derive_type(&declared), self.declarations, self.ordinal);
        validate_module_type(self.module.source, &ty, wosy_syntax::byte_span(&declared), self.diagnostics);
        let name = quantifier.children_with_tokens().filter_map(|element| element.into_token())
            .find(|token| token.kind() == SyntaxKind::Identifier).expect("parsed quantified binder");
        self.bind(scope, name.text().to_owned(), ty.clone(), quantifier);
        if let Some(guard) = optional_node(quantifier, SyntaxKind::ProofQuantifierGuard) {
            self.check_cst(&node(&guard, SyntaxKind::Expression), Some(&ScalarType::Bool), scope);
        }
        ty
    }

    fn check_cst(&mut self, cst: &CstNode, expected: Option<&ScalarType>, scope: &BTreeMap<String, ScalarType>) -> ScalarType {
        self.check(&derive_expression(cst), expected, scope)
    }

    fn check(&mut self, expression: &ScalarExpression, expected: Option<&ScalarType>, scope: &BTreeMap<String, ScalarType>) -> ScalarType {
        self.check_with_hole(expression, expected, scope, None).0
    }

    fn check_with_hole(&mut self, expression: &ScalarExpression, expected: Option<&ScalarType>,
        scope: &BTreeMap<String, ScalarType>, hole: Option<&ScalarType>,
    ) -> (ScalarType, bool) {
        let visible = scope.keys().cloned().collect::<BTreeSet<_>>();
        let mut inference = ScalarInferenceBuilder::new(self.owners);
        inference.proof_entry_scope = Some(&self.entry_scope);
        inference.proof_rewrite_hole = hole.map(|ty| (ty, None, false));
        let actual = expression_type_in_module_expected(expression, expected, scope, &visible, &BTreeMap::new(),
            self.module, self.modules, self.diagnostics, false, self.typing, &mut inference);
        if let Some(expected) = expected {
            expect_module_type(self.module, expected, &actual, expression_span(expression), self.diagnostics);
        }
        (actual, matches!(inference.proof_rewrite_hole, Some((_, Some(_), false))))
    }

    fn check_match(&mut self, matching: &CstNode, scope: &BTreeMap<String, ScalarType>, witness: Option<&ScalarType>) {
        let Some(matching) = optional_node(matching, SyntaxKind::ProofMatch) else {
            self.error(matching, "missing authored match");
            return;
        };
        let source = self.module.source;
        let enums = self.module.enums;
        let checked = AuthoredMatch::prepare(&matching);
        let checked = checked.as_ref().map_err(|error| (error.span, error.message)).and_then(|matching| {
            matching.resolve(source, enums, &mut 0).map_err(|error| (error.span, error.message))
        });
        match checked {
            Ok(matching) => {
                let inputs = self.entry_scope.clone();
                self.check_match_selector(&matching.selector, &inputs);
                for (_, arm) in matching.arms {
                    self.steps(arm, scope, witness);
                }
            }
            Err((span, message)) => self.diagnostics.push(module_diagnostic(self.module, "B0003", message, span)),
        }
    }

    fn check_match_selector(&mut self, selector: &Selector<'_>, scope: &BTreeMap<String, ScalarType>) {
        if let Selector::If { condition, then_branch, else_branch } = selector {
            self.check(condition, Some(&ScalarType::Bool), scope);
            self.check_match_selector(then_branch, scope);
            self.check_match_selector(else_branch, scope);
        }
    }

    fn steps(&mut self, block: &ScalarBlock, scope: &BTreeMap<String, ScalarType>, witness: Option<&ScalarType>) {
        for item in &block.items {
            match item {
                ScalarBlockItem::Expression(ScalarExpression::UnitIf { condition, then_branch, .. }) => {
                    self.check(condition, Some(&ScalarType::Bool), scope);
                    self.steps(then_branch, scope, witness);
                }
                ScalarBlockItem::Expression(ScalarExpression::If { condition, then_branch, else_branch, .. }) => {
                    self.check(condition, Some(&ScalarType::Bool), scope);
                    self.steps(then_branch, scope, witness);
                    self.steps(else_branch, scope, witness);
                }
                ScalarBlockItem::Expression(ScalarExpression::Call { receiver, name, type_arguments, arguments, span, .. })
                    if receiver.as_deref() == Some("core") && type_arguments.is_empty() => {
                    match (name.as_str(), arguments.as_slice()) {
                        ("show", [claim]) => { self.check(claim, Some(&ScalarType::Bool), scope); }
                        ("witness", [value]) if witness.is_some() => { self.check(value, witness, scope); }
                        ("rewrite", [ScalarExpression::Binary { operator: BinaryOperator::Equal, left, right, .. }, context]) => {
                            let (typed, other) = if matches!(left.as_ref(), ScalarExpression::Integer { .. } | ScalarExpression::Float { .. }) {
                                (right.as_ref(), left.as_ref())
                            } else {
                                (left.as_ref(), right.as_ref())
                            };
                            let equality_type = self.check(typed, None, scope);
                            self.check(other, Some(&equality_type), scope);
                            let (_, one_hole) = self.check_with_hole(context, Some(&ScalarType::Bool), scope, Some(&equality_type));
                            if !one_hole {
                                self.diagnostics.push(module_diagnostic(self.module, "B0003", "proof rewrite context must have one hole", *span));
                            }
                        }
                        ("use", [ScalarExpression::Call { receiver, name, type_arguments, arguments, .. }])
                            if type_arguments.is_empty() => {
                            let source = match receiver {
                                None => Some(self.module.source),
                                Some(namespace) => self.module.namespace_bindings.iter()
                                    .find(|binding| &binding.binding == namespace).map(|binding| &binding.target),
                            };
                            match source.and_then(|source| self.proof_names.get(source)).and_then(|names| names.get(name)) {
                                Some(params) if params.len() == arguments.len() => for (arg, ty) in arguments.iter().zip(*params) {
                                    self.check(arg, Some(ty), scope);
                                },
                                _ => self.diagnostics.push(module_diagnostic(self.module, "B0003", "unknown proof or mismatched arity in core.use", *span)),
                            }
                        }
                        _ => self.diagnostics.push(module_diagnostic(self.module, "B0003", "invalid proof step", *span)),
                    }
                }
                ScalarBlockItem::Expression(expression) => {
                    self.diagnostics.push(module_diagnostic(self.module, "B0003", "expected a proof step", expression_span(expression)));
                }
                _ => self.diagnostics.push(module_diagnostic(self.module, "B0003", "expected a proof step", block.span)),
            }
        }
    }
}

fn optional_node(parent: &CstNode, kind: SyntaxKind) -> Option<CstNode> {
    parent.children().find(|node| node.kind() == kind)
}

fn node(parent: &CstNode, kind: SyntaxKind) -> CstNode {
    optional_node(parent, kind).expect("parsed proof clause")
}

fn names(parent: &CstNode) -> Vec<String> {
    parent.children_with_tokens().filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Identifier).map(|token| token.text().to_owned()).collect()
}

// The canonical CST represents () by one Parameters token, and a nonempty
// parameter list by a Parameters node. Preserve that distinction from the
// contextual proof header and never assume the rule has a child node.
fn parameter_names(parent: &CstNode) -> Option<Vec<String>> {
    let mut children = parent.children_with_tokens().filter(|child| child.kind() == SyntaxKind::Parameters);
    let parameters = children.next()?;
    if children.next().is_some() { return None; }
    match parameters.into_node() {
        Some(node) => Some(names(&node)),
        None => Some(Vec::new()),
    }
}
