use super::*;

#[derive(Clone, Copy)]
pub(super) enum PlaceTypingContext<'a> {
    Program(&'a ScalarProgram),
    Module(&'a ScalarModule, &'a [ScalarModule]),
}

#[derive(Clone, Copy)]
pub(super) enum PlaceAccess {
    Read,
    Write,
}

impl PlaceTypingContext<'_> {
    fn expression(
        &self,
        expression: &ScalarExpression,
        scope: &BTreeMap<String, ScalarType>,
        expected: Option<&ScalarType>,
        diagnostics: &mut Vec<super::Diagnostic>,
        unsafe_context: bool,
    ) -> ScalarType {
        match self {
            Self::Program(program) => match expected {
                Some(expected) => expression_type_expected(
                    expression,
                    expected,
                    scope,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    program,
                    diagnostics,
                    unsafe_context,
                ),
                None => expression_type(
                    expression,
                    scope,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    program,
                    diagnostics,
                    unsafe_context,
                ),
            },
            Self::Module(module, modules) => expression_type_in_module_expected(
                expression,
                expected,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                unsafe_context,
            ),
        }
    }

    fn error(&self, code: &str, message: &str, span: ByteSpan) -> super::Diagnostic {
        match self {
            Self::Program(program) => diagnostic(program, code, message, span),
            Self::Module(module, _) => module_diagnostic(module, code, message, span),
        }
    }

    fn field_type(&self, field: &ScalarStructFieldId) -> Option<ScalarType> {
        let structure = match self {
            Self::Program(program) => program
                .structs
                .iter()
                .find(|structure| structure.id == field.structure),
            Self::Module(_, modules) => modules
                .iter()
                .flat_map(|module| module.structs.iter())
                .find(|structure| structure.id == field.structure),
        };
        structure
            .and_then(|structure| structure.fields.get(field.index))
            .filter(|candidate| candidate.id == *field)
            .map(|candidate| candidate.ty.clone())
    }
}

pub(super) fn typed_place_projection(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    context: PlaceTypingContext<'_>,
    access: PlaceAccess,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match place {
        ScalarPlace::Name { name, span } => match scope.get(name) {
            Some(ty) => ty.clone(),
            None => {
                diagnostics.push(context.error("B0001", "unknown name", *span));
                ScalarType::Error
            }
        },
        ScalarPlace::Dereference { pointer, span } => {
            match context.expression(pointer, scope, None, diagnostics, unsafe_context) {
                ScalarType::CheckedReference { mutability, inner } => {
                    if matches!(access, PlaceAccess::Write)
                        && mutability != ScalarReferenceMutability::Mutable
                    {
                        diagnostics.push(context.error(
                            "B0003",
                            "assignment dereference requires a mutable checked reference",
                            *span,
                        ));
                        ScalarType::Error
                    } else {
                        *inner
                    }
                }
                ScalarType::RawPointer(inner) => *inner,
                ScalarType::Error => ScalarType::Error,
                _ => {
                    let message = match access {
                        PlaceAccess::Read => "cannot dereference value",
                        PlaceAccess::Write => {
                            "assignment dereference requires a mutable checked reference"
                        }
                    };
                    diagnostics.push(context.error("B0003", message, *span));
                    ScalarType::Error
                }
            }
        }
        ScalarPlace::Field { base, field, span } => {
            let base_type =
                typed_place_projection(base, scope, context, access, diagnostics, unsafe_context);
            let structure = match base_type {
                ScalarType::Struct(structure) => structure,
                ScalarType::RawPointer(inner) if matches!(access, PlaceAccess::Read) => {
                    match *inner {
                        ScalarType::Struct(structure) => structure,
                        _ => {
                            diagnostics.push(context.error("B0003", "value has no field", *span));
                            return ScalarType::Error;
                        }
                    }
                }
                _ => {
                    diagnostics.push(context.error("B0003", "value has no field", *span));
                    return ScalarType::Error;
                }
            };
            if let ScalarFieldReference::Resolved(field) = field {
                if field.structure == structure {
                    if let Some(ty) = context.field_type(field) {
                        return ty;
                    }
                }
            }
            diagnostics.push(context.error("B0003", "value has no field", *span));
            ScalarType::Error
        }
        ScalarPlace::Index {
            base,
            index,
            index_span,
            span,
        } => {
            let base_type =
                typed_place_projection(base, scope, context, access, diagnostics, unsafe_context);
            let index_type = context.expression(
                index,
                scope,
                Some(&ScalarType::U64),
                diagnostics,
                unsafe_context,
            );
            match &context {
                PlaceTypingContext::Program(program) => expect_type(
                    program,
                    &ScalarType::U64,
                    &index_type,
                    *index_span,
                    diagnostics,
                ),
                PlaceTypingContext::Module(module, _) => expect_module_type(
                    module,
                    &ScalarType::U64,
                    &index_type,
                    *index_span,
                    diagnostics,
                ),
            }
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(context.error(
                        "B0003",
                        "indexed place requires a fixed array",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
        }
    }
}
