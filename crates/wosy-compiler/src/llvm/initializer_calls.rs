use super::*;
use crate::scalar::ConcreteCallSelection;
use wosy_syntax::{ByteSpan, SourceIdentity, SourceSpan};

#[derive(Default)]
pub(super) struct InitializerSymbols {
    functions: HashMap<SourceSpan, (String, String)>,
    externs: HashMap<SourceSpan, String>,
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

impl<'ctx, 'module> EmitState<'ctx, 'module> {
    pub(super) fn initializer_target(
        &self,
        expression: &ScalarExpression,
    ) -> Result<Option<&'module ModuleInitializationCallTarget>, String> {
        self.initializer_nodes
            .map(|nodes| nodes.target(expression))
            .transpose()
    }

    pub(super) fn initializer_symbol(
        &self,
        target: &ModuleInitializationCallTarget,
    ) -> Result<Option<String>, String> {
        let symbols = self
            .initializer_symbols
            .expect("prepared initializer symbol table");
        let name = match target {
            ModuleInitializationCallTarget::Direct {
                declaration,
                concrete,
            } => {
                let (_, base) = symbols.function_entry(declaration)?;
                match concrete {
                    ConcreteCallSelection::Declaration => base.clone(),
                    ConcreteCallSelection::Overload(selection) => {
                        let key = selected_overload_lookup_key(base, selection);
                        self.call_targets.get(&key).cloned().ok_or_else(|| {
                            format!("internal error: missing LLVM initializer overload {key}")
                        })?
                    }
                }
            }
            ModuleInitializationCallTarget::Generic {
                declaration,
                type_arguments,
            } => {
                let (_, base) = symbols.function_entry(declaration)?;
                generic_specialization_name(base, type_arguments)
            }
            ModuleInitializationCallTarget::Extern(declaration) => {
                symbols.externs.get(declaration).cloned().ok_or_else(|| {
                    format!("internal error: missing LLVM initializer extern {declaration:?}")
                })?
            }
            ModuleInitializationCallTarget::Core(_)
            | ModuleInitializationCallTarget::CallableBinding(_)
            | ModuleInitializationCallTarget::CallableField { .. } => return Ok(None),
        };
        if !self.functions.contains_key(&name) {
            return Err(format!(
                "internal error: missing LLVM initializer symbol {name}"
            ));
        }
        Ok(Some(name))
    }

    pub(super) fn initializer_call_output(
        &self,
        expression: &ScalarExpression,
    ) -> Result<Option<ScalarType>, String> {
        let target = self
            .initializer_target(expression)?
            .expect("prepared initializer call");
        match target {
            ModuleInitializationCallTarget::Core(operation) => {
                let ScalarExpression::Call { type_arguments, .. } = expression else {
                    unreachable!("initializer call node");
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
            _ => {
                let Some(symbol) = self.initializer_symbol(target)? else {
                    return Ok(None);
                };
                let Some(ScalarType::Callable { outputs, .. }) = self.signatures.get(&symbol)
                else {
                    return Err(format!(
                        "internal error: missing LLVM initializer callable signature {symbol}"
                    ));
                };
                Ok(outputs.outputs.first().map(|output| output.ty.clone()))
            }
        }
    }
}
