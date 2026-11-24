use super::*;
use rowan::WalkEvent;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ContextualStorage {
    pub(super) name: String,
    pub(super) declaration: SourceSpan,
    pub(super) scope: ByteSpan,
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
}

impl ContextualPlaces {
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

pub(super) fn prepare(
    canonical: &CanonicalCstRoot, roots: &[CstNode], diagnostics: &mut Vec<Diagnostic>,
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
    result
}
