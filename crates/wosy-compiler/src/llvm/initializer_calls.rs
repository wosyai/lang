use super::*;
use crate::scalar::ConcreteCallSelection;
use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

struct FunctionSymbol<'input> {
    identity: (String, String),
    ordinary_body: Option<(&'input ScalarFunction, &'input ScalarReferenceCfg)>,
    closed_shared_reader: bool,
    local_purity: std::cell::Cell<Option<bool>>,
}

#[derive(Default)]
pub(super) struct InitializerSymbols<'input> {
    functions: HashMap<SourceIdentity, HashMap<ByteSpan, FunctionSymbol<'input>>>,
    externs: HashMap<SourceSpan, String>,
    callable_bindings: std::collections::HashSet<SourceSpan>,
}

impl<'input> InitializerSymbols<'input> {
    pub(super) fn function(
        &mut self,
        source: &SourceIdentity,
        function: &ScalarFunction,
        symbol: String,
    ) {
        let entry = FunctionSymbol {
            identity: (function.name.clone(), symbol),
            ordinary_body: None,
            closed_shared_reader: false,
            local_purity: std::cell::Cell::new(None),
        };
        if let Some(functions) = self.functions.get_mut(source) {
            functions.insert(function.name_span, entry);
        } else {
            self.functions.insert(source.clone(), HashMap::from([(function.name_span, entry)]));
        }
    }

    pub(super) fn scalar_local_target(
        &mut self, source: &SourceIdentity, function: &'input ScalarFunction, cfg: &'input ScalarReferenceCfg,
        closed_shared_reader: bool,
    ) {
        // No CFG analysis until a prepared initializer or replacement resolves
        // this exact declaration as a local callable value.
        let entry = self.functions.get_mut(source).and_then(|functions| functions.get_mut(&function.name_span))
            .expect("registered ordinary callable declaration");
        entry.ordinary_body = Some((function, cfg));
        entry.closed_shared_reader = closed_shared_reader;
    }

    pub(super) fn external(&mut self, source: &SourceIdentity, span: ByteSpan, symbol: &str) {
        self.externs
            .insert(SourceSpan::new(source.clone(), span), symbol.to_owned());
    }

    pub(super) fn callable_binding(
        &mut self,
        source: &SourceIdentity,
        binding: &crate::ScalarBinding,
    ) {
        if matches!(binding.declared_type, ScalarType::Callable { .. }) {
            self.callable_bindings
                .insert(SourceSpan::new(source.clone(), binding.name_span));
        }
        for receiver in binding.receivers.iter().skip(1) {
            if matches!(receiver.ty, ScalarType::Callable { .. }) {
                self.callable_bindings
                    .insert(SourceSpan::new(source.clone(), receiver.name_span));
            }
        }
    }

    fn function_entry(&self, declaration: &SourceSpan) -> Result<&(String, String), String> {
        self.function_entry_at(&declaration.source, declaration.range)
    }

    fn function_entry_at(
        &self,
        source: &SourceIdentity,
        range: ByteSpan,
    ) -> Result<&(String, String), String> {
        self.functions
            .get(source)
            .and_then(|functions| functions.get(&range))
            .map(|entry| &entry.identity)
            .ok_or_else(|| {
                format!(
                    "internal error: missing LLVM initializer declaration {:?}",
                    SourceSpan::new(source.clone(), range)
                )
            })
    }
}

pub(super) fn prepared_initializer_overloads<'a>(
    regions: impl Iterator<Item = &'a ModuleInitializationRegion>,
    symbols: &InitializerSymbols<'_>,
) -> Result<HashMap<SourceIdentity, Vec<(String, ScalarOverloadSelection)>>, String> {
    let mut selections = HashMap::<_, Vec<_>>::new();
    for region in regions {
        for target in region.call_targets()? {
            if let ModuleInitializationCallTarget::Direct {
                declaration,
                concrete: ConcreteCallSelection::Overload(selection),
            } = target?
            {
                let (name, _) = symbols.function_entry(declaration)?;
                selections
                    .entry(declaration.source.clone())
                    .or_default()
                    .push((name.clone(), selection.clone()));
            }
        }
    }
    Ok(selections)
}

pub(super) fn collect_body_overload_reservations(
    input: GenericInitializerInput<'_>,
    module: usize,
    items: &[ScalarItem],
    region: &ModuleInitializationRegion,
    symbols: &InitializerSymbols<'_>,
    selections: &mut HashMap<SourceIdentity, Vec<(String, ScalarOverloadSelection)>>,
) -> Result<(), String> {
    let mut initializer = if input.prepared_render() {
        None
    } else {
        Some(region.legacy_reservation_targets()?.peekable())
    };
    let mut reserve = |declaration: &SourceSpan, selection: &ScalarOverloadSelection| {
        let (name, _) = symbols.function_entry(declaration)?;
        selections
            .entry(declaration.source.clone())
            .or_default()
            .push((name.clone(), selection.clone()));
        Ok::<_, String>(())
    };
    for (item, declaration) in items.iter().enumerate() {
        if let Some(initializer) = &mut initializer {
            while initializer.peek().is_some_and(|&(root, _, _)| root == item) {
                if let Some((
                    _,
                    _,
                    ModuleInitializationCallTarget::Direct {
                        declaration,
                        concrete: ConcreteCallSelection::Overload(selection),
                    },
                )) = initializer.next()
                {
                    reserve(declaration, selection)?;
                }
            }
        }
        if matches!(declaration, ScalarItem::Function(_)) {
            input.visit_body_overloads(module, item, |target| {
                if let ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete: ConcreteCallSelection::Overload(selection),
                } = target
                {
                    reserve(
                        &SourceSpan::new(source.clone(), *declaration_span),
                        selection,
                    )?;
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

pub(super) enum PreparedCall {
    Core(CoreOperationId),
    Symbol(String),
    Local((u32, u32)),
    Indirect,
}

impl PreparedCall {
    pub(super) fn symbol(&self) -> Option<&str> {
        match self {
            Self::Symbol(symbol) => Some(symbol),
            Self::Core(_) | Self::Local(_) | Self::Indirect => None,
        }
    }
}

impl<'ctx, 'module> EmitState<'ctx, 'module> {
    pub(super) fn has_prepared_calls(&self) -> bool {
        self.prepared_render && (self.initializer_nodes.is_some() || self.body_call_nodes.is_some())
    }

    pub(super) fn prepared_call(
        &self,
        expression: &ScalarExpression,
    ) -> Result<Option<PreparedCall>, String> {
        if !self.prepared_render {
            return Ok(None);
        }
        let selected = if let Some(nodes) = self.body_call_nodes {
            match nodes.target(expression)? {
                ResolvedCallTarget::Core(operation) => PreparedCall::Core(*operation),
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete,
                } => {
                    let declaration = SourceSpan::new(source.clone(), *declaration_span);
                    if self.symbols().callable_bindings.contains(&declaration) {
                        PreparedCall::Indirect
                    } else {
                        let ScalarExpression::Call { type_arguments, .. } = expression else {
                            unreachable!("callable body call node");
                        };
                        let (_, base) = self.symbols().function_entry(&declaration)?;
                        let symbol = match concrete {
                            ConcreteCallSelection::Declaration if !type_arguments.is_empty() => {
                                let lookup = specialization_lookup_key(base, type_arguments);
                                self.call_targets
                                    .get(&lookup)
                                    .cloned()
                                    .ok_or_else(|| format!("unknown LLVM callable {base}"))?
                            }
                            _ => self.direct_symbol(base, concrete)?,
                        };
                        PreparedCall::Symbol(symbol)
                    }
                }
                ResolvedCallTarget::ExternCallable {
                    source,
                    declaration_span,
                } => PreparedCall::Symbol(
                    self.external_symbol(&SourceSpan::new(source.clone(), *declaration_span))?,
                ),
                ResolvedCallTarget::LocalCallable(binding) => {
                    let key = (binding.declaration_span.start, binding.declaration_span.end);
                    if let Some(slot) = self.local_callable_slots.get(&key) {
                        if slot.parameters.iter().any(|ty| matches!(ty, ScalarType::CheckedReference { .. }))
                            && !nodes.local_shared_call_closed(expression)
                        {
                            return Err("local shared callable family has no closed checked-read authority".into());
                        }
                        PreparedCall::Local(key)
                    } else {
                        PreparedCall::Indirect
                    }
                }
                ResolvedCallTarget::CallableField { .. } => PreparedCall::Indirect,
            }
        } else if let Some(nodes) = self.initializer_nodes {
            match nodes.target(expression)? {
                ModuleInitializationCallTarget::Core(operation) => PreparedCall::Core(*operation),
                target => match self.initializer_symbol(target)? {
                    Some(symbol) => PreparedCall::Symbol(symbol),
                    None => PreparedCall::Indirect,
                },
            }
        } else {
            return Ok(None);
        };
        if let Some(symbol) = selected.symbol() {
            if !self.functions.contains_key(symbol) {
                let region = if self.body_call_nodes.is_some() {
                    "body"
                } else {
                    "initializer"
                };
                return Err(format!(
                    "internal error: missing LLVM {region} symbol {symbol}"
                ));
            }
        }
        Ok(Some(selected))
    }

    fn bound_overload(
        &self,
        expression: &ScalarExpression,
    ) -> Result<Option<(&SourceIdentity, ByteSpan, &ScalarOverloadSelection)>, String> {
        if let Some(nodes) = self.body_call_nodes {
            return Ok(match nodes.target(expression)? {
                ResolvedCallTarget::ModuleCallable {
                    source,
                    declaration_span,
                    concrete: ConcreteCallSelection::Overload(selection),
                } => Some((source, *declaration_span, selection)),
                _ => None,
            });
        }
        if let Some(nodes) = self.initializer_nodes {
            return Ok(match nodes.target(expression)? {
                ModuleInitializationCallTarget::Direct {
                    declaration,
                    concrete: ConcreteCallSelection::Overload(selection),
                } => Some((&declaration.source, declaration.range, selection)),
                _ => None,
            });
        }
        Err("internal error: missing bound LLVM call context".into())
    }

    pub(super) fn raw_call_symbol(
        &self,
        expression: &ScalarExpression,
        qualified: &str,
        type_arguments: &[crate::scalar::ScalarTypeArgument],
    ) -> Result<Option<&String>, String> {
        if let Some((source, range, selection)) = self.bound_overload(expression)? {
            let (_, base) = self.symbols().function_entry_at(source, range)?;
            let key = selected_overload_lookup_key(base, selection);
            return self
                .call_targets
                .get(&key)
                .map(Some)
                .ok_or_else(|| format!("unknown LLVM callable {qualified}"));
        }
        Ok(self.call_targets.get(qualified).or_else(|| {
            self.call_targets
                .get(&specialization_lookup_key(qualified, type_arguments))
        }))
    }

    pub(super) fn direct_aggregate_symbol(
        &self,
        expression: &ScalarExpression,
        initializer_target: Option<&ModuleInitializationCallTarget>,
    ) -> Result<&str, String> {
        let (source, range, concrete) = if let Some(target) = initializer_target {
            let ModuleInitializationCallTarget::Direct { declaration, concrete } = target else {
                return Err("missing closed initializer callable declaration".into());
            };
            (&declaration.source, declaration.range, concrete)
        } else {
            let nodes = self.body_call_nodes.ok_or_else(|| "missing closed aggregate body binding".to_owned())?;
            let ResolvedCallTarget::ModuleCallable { source, declaration_span, concrete } =
                nodes.target(expression)?
            else {
                return Err("missing closed aggregate callable declaration".into());
            };
            (source, *declaration_span, concrete)
        };
        let (_, base) = self.symbols().function_entry_at(source, range)?;
        match concrete {
            ConcreteCallSelection::Declaration => Ok(base.as_str()),
            ConcreteCallSelection::Overload(selection) => self.call_targets
                .get(&selected_overload_lookup_key(base, selection))
                .map(String::as_str)
                .ok_or_else(|| format!("missing closed aggregate selected definition {base}")),
        }
    }

    pub(super) fn closed_local_callable_symbol(&self, expression: &ScalarExpression) -> Option<&str> {
        let declaration = self.body_call_nodes?.callable_value_declaration(expression)?;
        let entry = self.symbols().functions.get(&declaration.source)?.get(&declaration.range)?;
        let closed = entry.local_purity.get().unwrap_or_else(|| {
            let closed = entry.closed_shared_reader || entry.ordinary_body.is_some_and(|(function, cfg)| {
                super::local_callable::closed_scalar_target(function, cfg)
            });
            entry.local_purity.set(Some(closed));
            closed
        });
        closed.then_some(entry.identity.1.as_str())
    }

    fn symbols(&self) -> &InitializerSymbols<'module> {
        self.initializer_symbols
            .expect("prepared LLVM symbol table")
    }

    fn direct_symbol(
        &self,
        base: &str,
        concrete: &ConcreteCallSelection,
    ) -> Result<String, String> {
        match concrete {
            ConcreteCallSelection::Declaration => Ok(base.to_owned()),
            ConcreteCallSelection::Overload(selection) => {
                let key = selected_overload_lookup_key(base, selection);
                self.call_targets.get(&key).cloned().ok_or_else(|| {
                    if self.body_call_nodes.is_some() {
                        format!("unknown LLVM callable {base}")
                    } else {
                        format!("internal error: missing LLVM initializer overload {key}")
                    }
                })
            }
        }
    }

    fn external_symbol(&self, declaration: &SourceSpan) -> Result<String, String> {
        self.symbols()
            .externs
            .get(declaration)
            .cloned()
            .ok_or_else(|| {
                let region = if self.body_call_nodes.is_some() {
                    "body"
                } else {
                    "initializer"
                };
                format!("internal error: missing LLVM {region} extern {declaration:?}")
            })
    }

    fn initializer_symbol(
        &self,
        target: &ModuleInitializationCallTarget,
    ) -> Result<Option<String>, String> {
        let symbols = self.symbols();
        let name = match target {
            ModuleInitializationCallTarget::Direct {
                declaration,
                concrete,
            } => {
                let (_, base) = symbols.function_entry(declaration)?;
                self.direct_symbol(base, concrete)?
            }
            ModuleInitializationCallTarget::Generic {
                declaration,
                type_arguments,
            } => {
                let (_, base) = symbols.function_entry(declaration)?;
                generic_specialization_name(base, type_arguments)
            }
            ModuleInitializationCallTarget::Extern(declaration) => {
                self.external_symbol(declaration)?
            }
            ModuleInitializationCallTarget::Core(_)
            | ModuleInitializationCallTarget::CallableBinding(_)
            | ModuleInitializationCallTarget::CallableField { .. } => return Ok(None),
        };
        Ok(Some(name))
    }

    pub(super) fn prepared_call_output(
        &self,
        expression: &ScalarExpression,
    ) -> Result<Option<ScalarType>, String> {
        let target = self.prepared_call(expression)?.expect("prepared call node");
        match target {
            PreparedCall::Core(operation) => {
                let ScalarExpression::Call { type_arguments, .. } = expression else {
                    unreachable!("prepared call node");
                };
                use CoreOperationId::*;
                Ok(match operation {
                    IntTrunc | IntExtend | UintToFloat | SintToFloat | FloatToSintTrunc
                    | FloatToUintTrunc | FloatTrunc | FloatExtend | Bitcast | PointerCast => {
                        type_arguments.first().map(|argument| argument.ty.clone())
                    }
                    _ => None,
                })
            }
            PreparedCall::Indirect => Ok(None),
            PreparedCall::Local(key) => Ok(self.local_callable_slots.get(&key)
                .and_then(|slot| slot.outputs.outputs.first()).map(|output| output.ty.clone())),
            PreparedCall::Symbol(symbol) => {
                let Some(ScalarType::Callable { outputs, .. }) = self.signatures.get(&symbol)
                else {
                    return Err(self.missing_prepared_signature(&symbol));
                };
                Ok(outputs.outputs.first().map(|output| output.ty.clone()))
            }
        }
    }

    pub(super) fn missing_prepared_signature(&self, symbol: &str) -> String {
        let region = if self.body_call_nodes.is_some() {
            "body"
        } else {
            "initializer"
        };
        format!("internal error: missing LLVM {region} callable signature {symbol}")
    }
}
