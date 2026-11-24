use super::*;
use rowan::WalkEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ContextualStorage {
    pub(super) name: String,
    pub(super) declaration: SourceSpan,
    pub(super) scope: ByteSpan,
    pub(super) active_after: u32,
    pub(super) ty: ScalarType,
    pub(super) binding: ReferenceBindingId,
}

#[derive(Debug, Default)]
pub(super) struct ContextualPlaces {
    pub(super) declarations: Vec<ContextualStorage>,
    pub(super) first_declarations: Vec<usize>,
    pub(super) resolved: HashMap<ByteSpan, ContextualOccurrence>,
    // In source order. A requirement is never entered in `resolved`: its
    // place identity belongs to a caller and no ABI currently supplies it.
    pub(super) inherited: Vec<InheritedContextualOccurrence>,
    pub(super) conflicted_types: HashSet<String>,
    dormant_operands: HashMap<ByteSpan, (usize, ReferenceBindingId)>,
}

impl ContextualPlaces {
    pub(super) fn operand(&self, span: ByteSpan, role: ContextualRole) -> Option<(&ReferenceBindingId, &ScalarType)> {
        if let Some(storage) = self.storage(span, role) {
            return Some((&storage.binding, &storage.ty));
        }
        let (occurrence, binding) = self.dormant_operands.get(&span)?;
        let occurrence = self.inherited.get(*occurrence)?;
        (occurrence.role == role).then_some(())?;
        Some((binding, &self.declarations.get(occurrence.local_type_declaration?)?.ty))
    }

    pub(super) fn storage(&self, span: ByteSpan, role: ContextualRole) -> Option<&ContextualStorage> {
        let occurrence = self.resolved.get(&span)?;
        (occurrence.role == role).then_some(())?;
        self.declarations.get(occurrence.storage)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct InheritedContextualOccurrence {
    pub(super) function: ByteSpan,
    pub(super) span: ByteSpan,
    pub(super) name: String,
    pub(super) role: ContextualRole,
    pub(super) diagnostic: usize,
    // Same-source type evidence only, not caller storage or completion of
    // project-wide type unification. Unknown names remain unresolved.
    pub(super) local_type_declaration: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ContextualOccurrence {
    pub(super) storage: usize,
    pub(super) role: ContextualRole,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContextualRole {
    Declare,
    Read,
    Assign,
}

pub(super) fn reported_error(span: SourceSpan, message: &str) -> Diagnostic {
    Diagnostic {
        code: "B0003".to_owned(),
        severity: DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![DiagnosticLabel {
            kind: DiagnosticLabelKind::Primary,
            span,
            message: "contextual place".to_owned(),
        }],
        notes: Vec::new(),
    }
}

pub(super) fn contextual_formal(source: &SourceIdentity, function: ByteSpan) -> ReferenceBindingId {
    ReferenceBindingId {
        source: source.clone(), function_span: function,
        declaration_span: function, block_span: function, kind: ReferenceBindingKind::ContextualFormal,
    }
}

pub(super) fn prepare(
    canonical: &CanonicalCstRoot, roots: &[CstNode], diagnostics: &mut Vec<Diagnostic>, dormant: bool,
) -> ContextualPlaces {
    let mut result = ContextualPlaces::default();
    let mut scopes = Vec::<(ByteSpan, HashMap<String, usize>)>::new();
    let mut functions = Vec::<(ByteSpan, usize)>::new();
    let mut pending = HashMap::<ByteSpan, Vec<(String, usize)>>::new();
    let mut declared_types = HashMap::<String, usize>::new();
    let mut diagnostic_index = diagnostics.len();
    let mut report = |span, message: &str| {
        let index = diagnostic_index;
        diagnostic_index += 1;
        diagnostics.push(reported_error(SourceSpan::new(canonical.source.clone(), span), message));
        index
    };
    for event in roots.iter().flat_map(CstNode::preorder) {
        match event {
            WalkEvent::Enter(node) if node.kind() == SyntaxKind::FunctionDecl => {
                functions.push((wosy_syntax::byte_span(&node), scopes.len()));
            }
            WalkEvent::Leave(node) if node.kind() == SyntaxKind::FunctionDecl => {
                functions.pop();
            }
            WalkEvent::Enter(node) if node.kind() == SyntaxKind::Block => {
                scopes.push((wosy_syntax::byte_span(&node), HashMap::new()));
            }
            WalkEvent::Leave(node) if node.kind() == SyntaxKind::Block => {
                scopes.pop();
            }
            WalkEvent::Leave(node) if node.kind() == SyntaxKind::LocalBinding => {
                if let Some(bindings) = pending.remove(&wosy_syntax::byte_span(&node)) {
                    let (_, active) = scopes.last_mut().expect("contextual local has a block");
                    for (name, storage) in bindings {
                        active.insert(name, storage);
                    }
                }
            }
            WalkEvent::Enter(node) if node.kind() == SyntaxKind::ContextualPlace => {
                let span = wosy_syntax::byte_span(&node);
                let Some(name) = direct_token(&node, SyntaxKind::Identifier) else {
                    report(span, "invalid contextual name");
                    continue;
                };
                let Some(parent) = node.parent() else { continue; };
                if node.children().next().is_some() || node.ancestors().any(|node| matches!(
                    node.kind(), SyntaxKind::IndexedPlace | SyntaxKind::CheckedAddress | SyntaxKind::RawAddress,
                )) {
                    report(span, "projected or borrowed contextual places are not supported");
                    continue;
                }
                if parent.kind() == SyntaxKind::Receiver {
                    let Some(binding) = parent.ancestors().find(|ancestor| ancestor.kind() == SyntaxKind::LocalBinding) else {
                        report(span, "contextual declaration requires callable-local storage");
                        continue;
                    };
                    let Some((scope, active)) = scopes.last() else {
                        report(span, "contextual declaration requires an active callable block");
                        continue;
                    };
                    let Some(&(function, _)) = functions.last() else {
                        report(span, "contextual storage requires a direct function body");
                        continue;
                    };
                    if binding.children().find(|child| child.kind() == SyntaxKind::ReceiverList)
                        .is_none_or(|receivers| receivers.children().filter(|child| child.kind() == SyntaxKind::Receiver).count() != 1)
                    {
                        report(span, "contextual declarations require a single receiver");
                        continue;
                    }
                    let Some(ty) = parent.children().find(|child| child.kind() == SyntaxKind::TypeSpec).map(|ty| derive_type(&ty)) else {
                        report(span, "contextual declaration requires an integer or bool type");
                        continue;
                    };
                    if ty != ScalarType::Bool && !typecheck::is_integer_type(&ty) {
                        report(span, "contextual declaration requires an integer or bool type");
                        continue;
                    }
                    let waiting = pending.entry(wosy_syntax::byte_span(&binding)).or_default();
                    if active.contains_key(name.text()) || waiting.iter().any(|(prior, _)| prior == name.text()) {
                        report(span, "contextual name is already declared in this scope");
                        continue;
                    }
                    let id = result.declarations.len();
                    let known = declared_types.entry(name.text().to_owned()).or_insert(id);
                    if *known == id {
                        result.first_declarations.push(id);
                    } else if result.declarations[*known].ty != ty {
                        result.conflicted_types.insert(name.text().to_owned());
                        report(span, "contextual name must have one static type");
                        continue;
                    }
                    result.declarations.push(ContextualStorage {
                        name: name.text().to_owned(),
                        declaration: SourceSpan::new(canonical.source.clone(), span),
                        scope: *scope,
                        active_after: wosy_syntax::byte_span(&binding).end,
                        ty,
                        binding: ReferenceBindingId {
                            source: canonical.source.clone(),
                            function_span: function,
                            declaration_span: span,
                            block_span: *scope,
                            kind: ReferenceBindingKind::Declared,
                        },
                    });
                    waiting.push((name.text().to_owned(), id));
                    result.resolved.insert(span, ContextualOccurrence { storage: id, role: ContextualRole::Declare });
                } else {
                    let role = if parent.kind() == SyntaxKind::AssignmentTarget {
                        let Some(targets) = parent.parent().filter(|node| node.kind() == SyntaxKind::AssignmentTargets) else {
                            report(span, "contextual write requires a direct assignment target");
                            continue;
                        };
                        let statement = targets.parent().filter(|node| node.kind() == SyntaxKind::Assignment)
                            .and_then(|node| node.parent())
                            .filter(|node| node.kind() == SyntaxKind::Expression)
                            .and_then(|node| node.parent());
                        if !statement.is_some_and(|node| node.kind() == SyntaxKind::BlockItem) {
                            report(span, "contextual write requires a standalone assignment statement");
                            continue;
                        }
                        if targets.children().filter(|child| child.kind() == SyntaxKind::AssignmentTarget).count() != 1 {
                            report(span, "contextual writes require a single assignment target");
                            continue;
                        }
                        ContextualRole::Assign
                    } else {
                        ContextualRole::Read
                    };
                    let floor = functions.last().map_or(0, |&(_, floor)| floor);
                    if let Some(&storage) = scopes[floor..].iter().rev().find_map(|(_, active)| active.get(name.text())) {
                        result.resolved.insert(span, ContextualOccurrence { storage, role });
                    } else {
                        let diagnostic = report(span, "inherited contextual bindings are not supported by scalar preparation yet");
                        if let Some(&(function, _)) = functions.last() {
                            result.inherited.push(InheritedContextualOccurrence {
                                function, span, name: name.text().to_owned(), role, diagnostic, local_type_declaration: None,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for occurrence in &mut result.inherited {
        occurrence.local_type_declaration = declared_types.get(&occurrence.name).copied()
            .filter(|_| !result.conflicted_types.contains(&occurrence.name));
    }
    if dormant && !result.inherited.is_empty() && result.conflicted_types.is_empty()
        && result.first_declarations.len() == 1
        && result.inherited.iter().all(|occurrence| {
            occurrence.local_type_declaration == result.first_declarations.first().copied()
        })
    {
        let first = result.first_declarations[0];
        if typecheck::is_integer_type(&result.declarations[first].ty) {
            // Formal identity is shared by the same function's reads and
            // writes, but never uses another function's declaration as storage.
            let mut suppressed = result.inherited.iter().map(|occurrence| occurrence.diagnostic).peekable();
            let mut ordinal = 0;
            diagnostics.retain(|_| {
                let remove = suppressed.next_if_eq(&ordinal).is_some();
                ordinal += 1;
                !remove
            });
            for (index, occurrence) in result.inherited.iter().enumerate() {
                result.dormant_operands.insert(occurrence.span, (index, contextual_formal(&canonical.source, occurrence.function)));
            }
        }
    }
    result
}

/// Opaque, single-use evidence minted at the checked call's lexical sweep
/// (or at formal propagation). No caller can wrap an arbitrary binding.
#[cfg(any(test, feature = "internal-checker"))]
pub(super) struct ContextualSupplyAuthority {
    occurrence: super::reference::OccurrenceOrdinal,
    span: ByteSpan,
    binding: ReferenceBindingId,
    name: std::rc::Rc<str>,
    ty: ScalarType,
}

#[cfg(any(test, feature = "internal-checker"))]
impl ContextualSupplyAuthority {
    pub(super) fn into_parts(self) -> (
        super::reference::OccurrenceOrdinal, ByteSpan, ReferenceBindingId, std::rc::Rc<str>, ScalarType,
    ) {
        (self.occurrence, self.span, self.binding, self.name, self.ty)
    }
}

/// Closed, dormant call environment. This is not a prepared build project:
/// only checked call coordinates and their lexical suppliers escape analysis.
#[cfg(any(test, feature = "internal-checker"))]
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DormantContextReceipt {
    pub(super) ty: ScalarType,
    pub(super) edges: Vec<DormantContextEdge>,
}

#[cfg(any(test, feature = "internal-checker"))]
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct DormantContextEdge {
    pub(super) occurrence: ByteSpan,
    pub(super) target: ByteSpan,
    pub(super) supplied: DormantContextSupply,
}

#[cfg(any(test, feature = "internal-checker"))]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum DormantContextSupply {
    Formal(ByteSpan),
    Lexical(ByteSpan),
}

#[cfg(any(test, feature = "internal-checker"))]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum DormantContextRejection {
    Unsupported,
    MissingAtRoot(ByteSpan),
}

/// Side-effect-free same-source, one-integer-name compatibility gate.
/// Validate typed entry and call places with ordinary ownership checking,
/// but never give the internal project or its formal to a backend.
#[cfg(any(test, feature = "internal-checker"))]
#[allow(dead_code)]
pub(super) fn prepare_dormant_context(
    canonical: &CanonicalCstRoot, inputs: ScalarPreparationInputs,
) -> Result<DormantContextReceipt, DormantContextRejection> {
    use self::DormantContextRejection::{MissingAtRoot, Unsupported};
    use super::control::{ModuleInitializationCallTarget, ModuleInitializationDetail};
    const MAX_WORK: usize = 4096;
    let mut work = 0usize;
    let mut charge = |amount| {
        work = work.checked_add(amount).filter(|&cost| cost <= MAX_WORK).ok_or(Unsupported)?;
        Ok(())
    };
    // Extraction, type checking and canonical call preparation retain
    // their normal costs. This meter caps work added by the dormant graph.
    let mut source = derive_scalar_declarations(canonical, true, None);
    let places = &mut source.contextual_places;
    charge(places.declarations.len().saturating_add(places.inherited.len()).saturating_add(source.items.len()))?;
    if !source.diagnostics.is_empty() || places.dormant_operands.is_empty() {
        return Err(Unsupported);
    }
    let [first] = places.first_declarations.as_slice() else { return Err(Unsupported); };
    let ty = places.declarations[*first].ty.clone();
    let name: std::rc::Rc<str> = places.declarations[*first].name.as_str().into();
    // Keep only fixed-size lexical activation coordinates for this one type.
    // The owning source retains the declaration catalog for project assembly.
    let mut lexical_by_owner = HashMap::<ByteSpan, Vec<(u32, ByteSpan, ByteSpan)>>::new();
    for storage in &places.declarations {
        if storage.ty != ty {
            return Err(Unsupported);
        }
        lexical_by_owner.entry(storage.binding.function_span).or_default()
            .push((storage.active_after, storage.scope, storage.declaration.range));
    }
    let uses = std::mem::take(&mut places.inherited);
    let project = project_from_declarations(vec![(source, Vec::new())], vec![canonical.source.clone()], inputs)
        .map_err(|_| Unsupported)?;
    let (project, mut calls) = super::static_calls::prepare_project_occurrences(
        typecheck_scalar_project_with_occurrences(project), &[],
    ).map_err(|_| Unsupported)?;
    let [module] = project.modules.as_slice() else { return Err(Unsupported); };
    let mut by_body = HashMap::new();
    let mut by_target = HashMap::new();
    for (index, item) in module.items.iter().enumerate() {
        match item {
            ScalarItem::Function(function) if function.generic_parameters.is_empty() && function.overload_arms.is_empty() => {
                if by_body.insert(function.span, index).is_some()
                    || by_target.insert(function.name_span, index).is_some()
                {
                    return Err(Unsupported);
                }
            }
            ScalarItem::Executable(_) => {}
            _ => return Err(Unsupported),
        }
    }
    let mut needs = vec![false; module.items.len()];
    let mut pending = Vec::new();
    for occurrence in uses {
        let index = *by_body.get(&occurrence.function).ok_or(Unsupported)?;
        if !std::mem::replace(&mut needs[index], true) { pending.push(index); }
    }
    struct Edge {
        caller: usize,
        caller_span: ByteSpan,
        occurrence: ByteSpan,
        ordinal: super::reference::OccurrenceOrdinal,
        target: ByteSpan,
        callee: usize,
        lexical: Option<(ContextualSupplyAuthority, ByteSpan)>,
    }
    let mut edges = Vec::new();
    let mut parents = vec![Vec::new(); module.items.len()];
    for (caller, item) in module.items.iter().enumerate() {
        let ScalarItem::Function(function) = item else { continue; };
        let body = calls.function_body(0, caller, None).ok_or(Unsupported)?;
        if !body.arms.is_empty() || !body.callable_escapes.is_empty() || !body.allocation_length_calls.is_empty() {
            return Err(Unsupported);
        }
        let mut lexical = lexical_by_owner.remove(&function.span).unwrap_or_default();
        // Activation order differs from declaration-entry order when a
        // declaration initializer contains another lexical block.
        let sort_work = |len: usize| len.saturating_mul((usize::BITS - len.leading_zeros()) as usize + 1);
        charge(sort_work(lexical.len()))?;
        lexical.sort_unstable_by_key(|&(active_after, scope, _)| (active_after, std::cmp::Reverse(scope.end)));
        let mut activation = lexical.into_iter().peekable();
        let mut active = Vec::<(ByteSpan, ByteSpan)>::new();
        charge(sort_work(body.call_occurrences.len()))?;
        let mut ordered = Vec::with_capacity(body.call_occurrences.len());
        for call in &body.call_occurrences {
            let span = body.occurrences.get(call.ordinal.0).ok_or(Unsupported)?.span;
            ordered.push((span, call));
        }
        ordered.sort_unstable_by_key(|(span, call)| (span.start, call.ordinal.0));
        for (occurrence, call) in ordered {
            let site = body.resolved_call_site(call.ordinal).ok_or(Unsupported)?;
            let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete: ConcreteCallSelection::Declaration } = &site.target
                else { return Err(Unsupported); };
            if source != &canonical.source || !body.checked_call_signatures.contains_key(&call.ordinal) {
                return Err(Unsupported);
            }
            let callee = *by_target.get(declaration_span).ok_or(Unsupported)?;
            while activation.peek().is_some_and(|&(start, _, _)| start <= occurrence.start) {
                charge(1)?;
                let (start, scope, binding) = activation.next().ok_or(Unsupported)?;
                while active.last().is_some_and(|&(outer, _)| outer.end <= start) {
                    charge(1)?;
                    active.pop();
                }
                if active.last().is_some_and(|&(outer, _)| scope.end > outer.end || scope.start < outer.start) {
                    return Err(Unsupported);
                }
                active.push((scope, binding));
            }
            while active.last().is_some_and(|&(scope, _)| scope.end <= occurrence.start) {
                charge(1)?;
                active.pop();
            }
            let binding = match active.last() {
                None => None,
                Some(&(scope, declaration)) if occurrence.end <= scope.end && scope.start <= occurrence.start => {
                    Some((ContextualSupplyAuthority {
                        occurrence: call.ordinal, span: occurrence, name: std::rc::Rc::clone(&name), ty: ty.clone(),
                        binding: ReferenceBindingId {
                            source: canonical.source.clone(), function_span: function.span,
                            declaration_span: declaration, block_span: scope, kind: ReferenceBindingKind::Declared,
                        },
                    }, declaration))
                }
                Some(_) => return Err(Unsupported),
            };
            if binding.is_none() { parents[callee].push(caller); }
            edges.push(Edge { caller, caller_span: function.span, occurrence, ordinal: call.ordinal,
                target: *declaration_span, callee, lexical: binding });
        }
    }
    // Propagate a single named demand over authenticated reverse edges.
    // Cycles cannot revisit a function that already requires the same name.
    while let Some(callee) = pending.pop() {
        charge(parents[callee].len())?;
        for &caller in &parents[callee] {
            if !std::mem::replace(&mut needs[caller], true) { pending.push(caller); }
        }
    }
    let init = calls.initialization_region(0).ok_or(Unsupported)?;
    let ModuleInitializationDetail::Occurrences(topology) = &init.detail else { return Err(Unsupported); };
    if !topology.callable_escapes.is_empty() { return Err(Unsupported); }
    let [root] = topology.calls.as_slice() else { return Err(Unsupported); };
    let ModuleInitializationCallTarget::Direct { declaration, concrete: ConcreteCallSelection::Declaration }
        = topology.call_targets.get(root).ok_or(Unsupported)? else { return Err(Unsupported); };
    if declaration.source != canonical.source || !topology.checked_call_signatures.contains_key(root) {
        return Err(Unsupported);
    }
    if needs[*by_target.get(&declaration.range).ok_or(Unsupported)?] {
        return Err(MissingAtRoot(topology.occurrences.get(root.0).ok_or(Unsupported)?.span));
    }
    for (index, needed) in needs.iter().enumerate() {
        if *needed { calls.install_contextual_formal(index, std::rc::Rc::clone(&name), ty.clone()).ok_or(Unsupported)?; }
    }
    let mut verified = Vec::new();
    for edge in edges.into_iter().filter(|edge| needs[edge.callee]) {
        let (authority, supplied) = match edge.lexical {
            Some((authority, declaration)) => (authority, DormantContextSupply::Lexical(declaration)),
            None if needs[edge.caller] => (ContextualSupplyAuthority {
                occurrence: edge.ordinal, span: edge.occurrence, name: std::rc::Rc::clone(&name), ty: ty.clone(),
                binding: contextual_formal(&canonical.source, edge.caller_span),
            }, DormantContextSupply::Formal(edge.caller_span)),
            None => return Err(Unsupported),
        };
        calls.install_contextual_call(edge.caller, edge.callee, authority).ok_or(Unsupported)?;
        verified.push(DormantContextEdge { occurrence: edge.occurrence, target: edge.target, supplied });
    }
    let references = super::reference::analyze_project_references(&project.modules, &calls);
    if !references.errors().is_empty() {
        return Err(Unsupported);
    }
    Ok(DormantContextReceipt { ty, edges: verified })
}
