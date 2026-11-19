use super::*;
use crate::scalar::ConcreteCallSelection;
use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

#[derive(Default)]
pub(super) struct InitializerSymbols {
    functions: HashMap<SourceSpan, (String, String)>,
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
        self.functions.insert(
            SourceSpan::new(source.clone(), function.name_span),
            (function.name.clone(), symbol),
        );
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
        self.functions.get(declaration).ok_or_else(|| {
            format!("internal error: missing LLVM initializer declaration {declaration:?}")
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

pub(super) fn collect_prepared_body_overload_selections(
    input: GenericInitializerInput<'_>,
    module: usize,
    items: &[ScalarItem],
    symbols: &InitializerSymbols,
    selections: &mut HashMap<SourceIdentity, Vec<(String, ScalarOverloadSelection)>>,
) -> Result<(), String> {
    if matches!(input, GenericInitializerInput::RawItems) {
        return Ok(());
    }
    for (item, declaration) in items.iter().enumerate() {
        let ScalarItem::Function(function) = declaration else {
            continue;
        };
        let mut search = OverloadSearch {
            calls: Vec::new(),
            prepared: true,
        };
        collect_selected_overloads_in_block(&function.body, &mut search);
        if search.calls.is_empty() {
            continue;
        }
        let nodes = input
            .bind_body(module, item, None, &function.body)?
            .expect("prepared body");
        for call in search.calls {
            if let ResolvedCallTarget::ModuleCallable {
                source,
                declaration_span,
                concrete: ConcreteCallSelection::Overload(selection),
            } = nodes.target(call)?
            {
                let (name, _) =
                    symbols.function_entry(&SourceSpan::new(source.clone(), *declaration_span))?;
                selections
                    .entry(source.clone())
                    .or_default()
                    .push((name.clone(), selection.clone()));
            }
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
        self.initializer_nodes.is_some() || self.body_call_nodes.is_some()
    }

    pub(super) fn prepared_call(
        &self,
        expression: &ScalarExpression,
    ) -> Result<Option<PreparedCall>, String> {
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
