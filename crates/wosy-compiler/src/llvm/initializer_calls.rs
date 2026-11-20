use super::*;
use crate::scalar::ConcreteCallSelection;
use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

#[derive(Default)]
pub(super) struct InitializerSymbols {
    functions: HashMap<SourceIdentity, HashMap<ByteSpan, (String, String)>>,
    externs: HashMap<SourceSpan, String>,
    callable_bindings: std::collections::HashSet<SourceSpan>,
}

impl InitializerSymbols {
    pub(super) fn function(
        &mut self,
        source: &SourceIdentity,
        function: &ScalarFunction,
        symbol: String,
    ) {
        if let Some(functions) = self.functions.get_mut(source) {
            functions.insert(function.name_span, (function.name.clone(), symbol));
        } else {
            self.functions.insert(
                source.clone(),
                HashMap::from([(function.name_span, (function.name.clone(), symbol))]),
            );
        }
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
    symbols: &InitializerSymbols,
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
    symbols: &InitializerSymbols,
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
    Indirect,
}

impl PreparedCall {
    pub(super) fn symbol(&self) -> Option<&str> {
        match self {
            Self::Symbol(symbol) => Some(symbol),
            Self::Core(_) | Self::Indirect => None,
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
                ResolvedCallTarget::LocalCallable(_) | ResolvedCallTarget::CallableField { .. } => {
                    PreparedCall::Indirect
                }
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

    fn symbols(&self) -> &InitializerSymbols {
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
