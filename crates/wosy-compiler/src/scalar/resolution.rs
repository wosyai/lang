use super::typecheck::{
    expression_type, expression_type_in_module, place_type, place_type_in_module,
};
use super::*;

pub(super) fn resolve_program_types(
    program: &mut ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let names: BTreeMap<String, ScalarType> = program
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(program.enums.iter().map(|enumeration| {
            (
                enumeration.name.clone(),
                ScalarType::Enum(enumeration.id.clone()),
            )
        }))
        .collect();
    for structure in &mut program.structs {
        for field in &mut structure.fields {
            field.ty = resolve_type(&field.ty, &names);
        }
    }
    resolve_program_struct_layouts(program, diagnostics);
    resolve_copy_policies(&mut program.structs, &mut program.enums, diagnostics);
    for item in &mut program.items {
        match item {
            ScalarItem::Binding(binding) => {
                binding.declared_type = resolve_type(&binding.declared_type, &names);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type(&receiver.ty, &names);
                }
                binding.output_sequence.outputs = binding
                    .output_sequence
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: resolve_type(&output.ty, &names),
                        span: output.span,
                    })
                    .collect();
            }
            ScalarItem::Function(function) => {
                if function.generic_parameters.is_empty() {
                    function.signature = resolve_type(&function.signature, &names);
                }
                for arm in &mut function.overload_arms {
                    if arm.generic_parameters.is_empty() {
                        arm.signature = resolve_type(&arm.signature, &names);
                    }
                }
                if let ScalarType::Callable { outputs, .. } = &function.signature {
                    for (value, output) in function
                        .body
                        .final_output_values
                        .iter_mut()
                        .zip(&outputs.outputs)
                    {
                        value.ty = output.ty.clone();
                    }
                }
                resolve_block_types(&mut function.body, &names);
            }
            ScalarItem::Extern(extern_decl) => {
                for function in &mut extern_decl.functions {
                    function.signature = resolve_type(&function.signature, &names);
                }
            }
            ScalarItem::Executable(item) => resolve_item_types(item, &names),
            ScalarItem::Namespace(_) => {}
        }
    }
}

fn resolve_item_types(item: &mut ScalarBlockItem, names: &BTreeMap<String, ScalarType>) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            binding.declared_type = resolve_type(&binding.declared_type, names);
            for receiver in &mut binding.receivers {
                receiver.ty = resolve_type(&receiver.ty, names);
            }
        }
        ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
            resolve_block_types(block, names)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_block_types(&mut while_expression.body, names)
        }
        ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
    }
}

fn resolve_block_types(block: &mut ScalarBlock, names: &BTreeMap<String, ScalarType>) {
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                binding.declared_type = resolve_type(&binding.declared_type, names);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type(&receiver.ty, names);
                }
                resolve_expression_types(&mut binding.value, names);
            }
            ScalarBlockItem::While(while_expression) => {
                resolve_expression_types(&mut while_expression.condition, names);
                resolve_block_types(&mut while_expression.body, names);
            }
            ScalarBlockItem::Expression(expression) => resolve_expression_types(expression, names),
            ScalarBlockItem::Assignment(assignment) => {
                resolve_expression_types(&mut assignment.value, names);
                for value in &mut assignment.values {
                    resolve_expression_types(value, names);
                }
            }
        }
    }
    for output in &mut block.final_output_values {
        resolve_expression_types(&mut output.value, names);
    }
}

fn resolve_expression_types(
    expression: &mut ScalarExpression,
    names: &BTreeMap<String, ScalarType>,
) {
    match expression {
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_types(condition, names);
            resolve_block_types(then_branch, names);
            resolve_block_types(else_branch, names);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_types(condition, names);
            resolve_block_types(then_branch, names);
        }
        ScalarExpression::Block(block) => resolve_block_types(block, names),
        _ => {}
    }
}

fn resolve_type(ty: &ScalarType, names: &BTreeMap<String, ScalarType>) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => names.get(name).cloned().map_or_else(
            || ScalarType::Named {
                name: name.clone(),
                span: *span,
            },
            |ty| ty,
        ),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(resolve_type(inner, names)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_type(inner, names)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_type(element, names)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: resolve_type(&output.ty, names),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| resolve_type(parameter, names))
                .collect(),
        },
        value => value.clone(),
    }
}

pub(super) fn resolve_program_places(
    program: &mut ScalarProgram,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    let context = program.clone();
    let names = program
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(program.enums.iter().map(|enumeration| {
            (
                enumeration.name.clone(),
                ScalarType::Enum(enumeration.id.clone()),
            )
        }))
        .collect::<BTreeMap<_, _>>();
    let declarations = program
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => {
                Some((binding.name.clone(), binding.declared_type.clone()))
            }
            ScalarItem::Function(function) => {
                Some((function.name.clone(), function.signature.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for item in &mut program.items {
        match item {
            ScalarItem::Binding(binding) => {
                resolve_expression_places(
                    &mut binding.value,
                    &declarations,
                    &context,
                    &names,
                    false,
                    overloads,
                    owners,
                );
            }
            ScalarItem::Function(function) => {
                let mut scope = declarations.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        scope.insert(name.clone(), ty.clone());
                    }
                }
                resolve_block_places(
                    &mut function.body,
                    &mut scope,
                    &context,
                    &names,
                    false,
                    overloads,
                    owners,
                );
            }
            ScalarItem::Executable(item) => {
                resolve_item_places(
                    item,
                    &mut declarations.clone(),
                    &context,
                    &names,
                    false,
                    overloads,
                    owners,
                );
            }
            ScalarItem::Extern(_) | ScalarItem::Namespace(_) => {}
        }
    }
}

fn resolve_item_places(
    item: &mut ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            resolve_expression_places(
                &mut binding.value,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            scope.insert(binding.name.clone(), binding.declared_type.clone());
        }
        ScalarBlockItem::Expression(expression) => resolve_expression_places(
            expression,
            scope,
            program,
            names,
            unsafe_context,
            overloads,
            owners,
        ),
        ScalarBlockItem::Assignment(assignment) => {
            for target in &mut assignment.targets {
                resolve_place(
                    &mut target.place,
                    scope,
                    program,
                    names,
                    unsafe_context,
                    overloads,
                    owners,
                );
            }
            resolve_expression_places(
                &mut assignment.value,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            for value in &mut assignment.values {
                resolve_expression_places(
                    value,
                    scope,
                    program,
                    names,
                    unsafe_context,
                    overloads,
                    owners,
                );
            }
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_expression_places(
                &mut while_expression.condition,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            resolve_block_places(
                &mut while_expression.body,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
    }
}

fn resolve_block_places(
    block: &mut ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    let unsafe_context = unsafe_context || block.unsafe_context;
    for item in &mut block.items {
        resolve_item_places(
            item,
            scope,
            program,
            names,
            unsafe_context,
            overloads,
            owners,
        );
    }
    for output in &mut block.final_output_values {
        resolve_expression_places(
            &mut output.value,
            scope,
            program,
            names,
            unsafe_context,
            overloads,
            owners,
        );
    }
}

fn resolve_expression_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    match expression {
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => resolve_place(
            place,
            scope,
            program,
            names,
            unsafe_context,
            overloads,
            owners,
        ),
        ScalarExpression::IndexedRead { place, .. } => resolve_place(
            place,
            scope,
            program,
            names,
            unsafe_context,
            overloads,
            owners,
        ),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_places(
                    &mut field.value,
                    scope,
                    program,
                    names,
                    unsafe_context,
                    overloads,
                    owners,
                );
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                resolve_expression_places(
                    element,
                    scope,
                    program,
                    names,
                    unsafe_context,
                    overloads,
                    owners,
                );
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            resolve_expression_places(
                left,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            resolve_expression_places(
                right,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
        ScalarExpression::Unary { operand, .. } => {
            resolve_expression_places(
                operand,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                for argument in type_arguments {
                    argument.ty = resolve_type(&argument.ty, names);
                }
            }
            for argument in arguments {
                resolve_expression_places(
                    argument,
                    scope,
                    program,
                    names,
                    unsafe_context,
                    overloads,
                    owners,
                );
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_places(
                condition,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            let mut then_scope = scope.clone();
            resolve_block_places(
                then_branch,
                &mut then_scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            let mut else_scope = scope.clone();
            resolve_block_places(
                else_branch,
                &mut else_scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_places(
                condition,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            let mut then_scope = scope.clone();
            resolve_block_places(
                then_branch,
                &mut then_scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
        ScalarExpression::Block(block) => {
            let mut scope = scope.clone();
            resolve_block_places(
                block,
                &mut scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            *enum_tag = program
                .enums
                .iter()
                .find(|enumeration| enumeration.name == *receiver)
                .and_then(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .find(|variant| variant.name == *name)
                })
                .map(|variant| variant.tag);
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

fn resolve_place(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => resolve_expression_places(
            pointer,
            scope,
            program,
            names,
            unsafe_context,
            overloads,
            owners,
        ),
        ScalarPlace::Field { base, field, span } => {
            resolve_place(
                base,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            let base_type = match base.as_ref() {
                ScalarPlace::Dereference { pointer, .. } => {
                    let mut scratch = ScalarInferenceBuilder::new(owners);
                    match expression_type(
                        pointer,
                        scope,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        program,
                        &mut Vec::new(),
                        unsafe_context,
                        overloads,
                        &mut scratch,
                    ) {
                        ScalarType::RawPointer(inner)
                        | ScalarType::CheckedReference { inner, .. } => *inner,
                        _ => ScalarType::Error,
                    }
                }
                ScalarPlace::Name { .. }
                | ScalarPlace::Field { .. }
                | ScalarPlace::Index { .. } => {
                    let mut scratch = ScalarInferenceBuilder::new(owners);
                    place_type(
                        base,
                        scope,
                        program,
                        &mut Vec::new(),
                        overloads,
                        &mut scratch,
                    )
                }
            };
            let structure = match base_type {
                ScalarType::Struct(id) => program.structs.get(id.index),
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => program.structs.get(id.index),
                    _ => None,
                },
                _ => None,
            };
            if let Some(declared) = structure.and_then(|structure| {
                structure
                    .fields
                    .iter()
                    .find(|candidate| {
                        matches!(field, ScalarFieldReference::Unresolved { name, .. } if candidate.name == *name)
                    })
            }) {
                *field = ScalarFieldReference::Resolved(declared.id.clone());
            } else {
                let _ = span;
            }
        }
        ScalarPlace::Index { base, index, .. } => {
            resolve_place(
                base,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
            resolve_expression_places(
                index,
                scope,
                program,
                names,
                unsafe_context,
                overloads,
                owners,
            );
        }
    }
}

pub(super) fn resolve_module_types_in_project(
    module: &mut ScalarModule,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
) {
    for structure in &mut module.structs {
        for field in &mut structure.fields {
            field.ty = resolve_type_in_project(&field.ty, declarations, current_module);
        }
    }
    for item in &mut module.items {
        match item {
            ScalarItem::Binding(binding) => {
                binding.declared_type =
                    resolve_type_in_project(&binding.declared_type, declarations, current_module);
                for receiver in &mut binding.receivers {
                    receiver.ty =
                        resolve_type_in_project(&receiver.ty, declarations, current_module);
                }
            }
            ScalarItem::Function(function) => {
                if function.generic_parameters.is_empty() {
                    function.signature =
                        resolve_type_in_project(&function.signature, declarations, current_module);
                }
                for arm in &mut function.overload_arms {
                    if arm.generic_parameters.is_empty() {
                        arm.signature =
                            resolve_type_in_project(&arm.signature, declarations, current_module);
                    }
                }
                if let ScalarType::Callable { outputs, .. } = &function.signature {
                    for (value, output) in function
                        .body
                        .final_output_values
                        .iter_mut()
                        .zip(&outputs.outputs)
                    {
                        value.ty = output.ty.clone();
                    }
                }
                resolve_block_types_project(&mut function.body, declarations, current_module);
            }
            ScalarItem::Extern(extern_decl) => {
                for function in &mut extern_decl.functions {
                    function.signature =
                        resolve_type_in_project(&function.signature, declarations, current_module);
                }
            }
            ScalarItem::Executable(item) => {
                resolve_item_types_project(item, declarations, current_module)
            }
            ScalarItem::Namespace(_) => {}
        }
    }
    for item in &module.items {
        match item {
            ScalarItem::Binding(binding) if !is_module_private_name(&binding.name) => {
                module
                    .members
                    .insert(binding.name.clone(), binding.declared_type.clone());
            }
            ScalarItem::Function(function) if !is_module_private_name(&function.name) => {
                module
                    .members
                    .insert(function.name.clone(), function.signature.clone());
            }
            _ => {}
        }
    }
}

fn resolve_item_types_project(
    item: &mut ScalarBlockItem,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            binding.declared_type =
                resolve_type_in_project(&binding.declared_type, declarations, current_module);
            for receiver in &mut binding.receivers {
                receiver.ty = resolve_type_in_project(&receiver.ty, declarations, current_module);
            }
        }
        ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
            resolve_block_types_project(block, declarations, current_module)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_block_types_project(&mut while_expression.body, declarations, current_module)
        }
        ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
    }
}

fn resolve_block_types_project(
    block: &mut ScalarBlock,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
) {
    for item in &mut block.items {
        resolve_item_types_project(item, declarations, current_module);
    }
}

fn resolve_type_in_project(
    ty: &ScalarType,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => {
            if let Some(id) = declarations.local_types(current_module).get(name) {
                return id.clone();
            }
            ScalarType::Named {
                name: name.clone(),
                span: *span,
            }
        }
        ScalarType::Qualified {
            receiver,
            receiver_span,
            member,
            member_span,
            span,
        } => match declarations.qualified_type(current_module, receiver, member) {
            Some(ty) => ty,
            None => ScalarType::Qualified {
                receiver: receiver.clone(),
                receiver_span: *receiver_span,
                member: member.clone(),
                member_span: *member_span,
                span: *span,
            },
        },
        ScalarType::RawPointer(inner) => ScalarType::RawPointer(Box::new(resolve_type_in_project(
            inner,
            declarations,
            current_module,
        ))),
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_type_in_project(inner, declarations, current_module)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_type_in_project(
                element,
                declarations,
                current_module,
            )),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::Callable {
            outputs,
            parameters,
        } => ScalarType::Callable {
            outputs: ScalarOutputSequence {
                outputs: outputs
                    .outputs
                    .iter()
                    .map(|output| ScalarOutput {
                        ty: resolve_type_in_project(&output.ty, declarations, current_module),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| resolve_type_in_project(parameter, declarations, current_module))
                .collect(),
        },
        value => value.clone(),
    }
}

pub(super) fn resolve_module_places(
    module: &mut ScalarModule,
    modules: &[ScalarModule],
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    let context = module.clone();
    let names = module
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(module.enums.iter().map(|enumeration| {
            (
                enumeration.name.clone(),
                ScalarType::Enum(enumeration.id.clone()),
            )
        }))
        .collect::<BTreeMap<_, _>>();
    let bindings = module
        .items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Binding(binding) => {
                Some((binding.name.clone(), binding.declared_type.clone()))
            }
            ScalarItem::Function(function) => {
                Some((function.name.clone(), function.signature.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for item in &mut module.items {
        match item {
            ScalarItem::Binding(binding) => {
                resolve_expression_module_places(
                    &mut binding.value,
                    &bindings,
                    &context,
                    modules,
                    &names,
                    false,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
            ScalarItem::Function(function) => {
                let mut scope = bindings.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        scope.insert(name.clone(), ty.clone());
                    }
                }
                resolve_block_module_places(
                    &mut function.body,
                    &mut scope,
                    &context,
                    modules,
                    &names,
                    false,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
            ScalarItem::Executable(item) => {
                let mut scope = bindings.clone();
                resolve_item_module_places(
                    item,
                    &mut scope,
                    &context,
                    modules,
                    &names,
                    false,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
            ScalarItem::Extern(_) | ScalarItem::Namespace(_) => {}
        }
    }
}

fn resolve_item_module_places(
    item: &mut ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            resolve_expression_module_places(
                &mut binding.value,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            scope.insert(binding.name.clone(), binding.declared_type.clone());
        }
        ScalarBlockItem::Expression(expression) => resolve_expression_module_places(
            expression,
            scope,
            module,
            modules,
            names,
            unsafe_context,
            declarations,
            current_module,
            overloads,
            owners,
        ),
        ScalarBlockItem::Assignment(assignment) => {
            for target in &mut assignment.targets {
                resolve_module_place(
                    &mut target.place,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
            resolve_expression_module_places(
                &mut assignment.value,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            for value in &mut assignment.values {
                resolve_expression_module_places(
                    value,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_expression_module_places(
                &mut while_expression.condition,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            resolve_block_module_places(
                &mut while_expression.body,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
    }
}

fn resolve_block_module_places(
    block: &mut ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    let unsafe_context = unsafe_context || block.unsafe_context;
    for item in &mut block.items {
        resolve_item_module_places(
            item,
            scope,
            module,
            modules,
            names,
            unsafe_context,
            declarations,
            current_module,
            overloads,
            owners,
        );
    }
    for output in &mut block.final_output_values {
        resolve_expression_module_places(
            &mut output.value,
            scope,
            module,
            modules,
            names,
            unsafe_context,
            declarations,
            current_module,
            overloads,
            owners,
        );
    }
}

fn resolve_expression_module_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    match expression {
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => resolve_module_place(
            place,
            scope,
            module,
            modules,
            names,
            unsafe_context,
            declarations,
            current_module,
            overloads,
            owners,
        ),
        ScalarExpression::IndexedRead { place, .. } => resolve_module_place(
            place,
            scope,
            module,
            modules,
            names,
            unsafe_context,
            declarations,
            current_module,
            overloads,
            owners,
        ),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_module_places(
                    &mut field.value,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                resolve_expression_module_places(
                    element,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            resolve_expression_module_places(
                left,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            resolve_expression_module_places(
                right,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
        ScalarExpression::Unary { operand, .. } => {
            resolve_expression_module_places(
                operand,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
        ScalarExpression::Call {
            receiver,
            name,
            type_arguments,
            arguments,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                for argument in type_arguments {
                    argument.ty =
                        resolve_type_in_project(&argument.ty, declarations, current_module);
                }
            }
            for argument in arguments {
                resolve_expression_module_places(
                    argument,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_module_places(
                condition,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            let mut then_scope = scope.clone();
            resolve_block_module_places(
                then_branch,
                &mut then_scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            let mut else_scope = scope.clone();
            resolve_block_module_places(
                else_branch,
                &mut else_scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_module_places(
                condition,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            let mut then_scope = scope.clone();
            resolve_block_module_places(
                then_branch,
                &mut then_scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
        ScalarExpression::Block(block) => {
            let mut scope = scope.clone();
            resolve_block_module_places(
                block,
                &mut scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            ..
        } => {
            *enum_tag = enum_member_in_module(module, modules, receiver, name).map(|(_, tag)| tag);
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

pub(super) fn enum_member_in_module(
    module: &ScalarModule,
    modules: &[ScalarModule],
    receiver: &str,
    name: &str,
) -> Option<(ScalarEnumId, u32)> {
    let enumeration = module
        .enums
        .iter()
        .find(|enumeration| enumeration.name == receiver)
        .or_else(|| {
            receiver.split_once('.').and_then(|(namespace, enum_name)| {
                module
                    .namespace_bindings
                    .iter()
                    .find(|binding| binding.binding == namespace)
                    .and_then(|binding| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == binding.target)
                    })
                    .and_then(|target| {
                        target.enums.iter().find(|enumeration| {
                            enumeration.name == enum_name
                                && !is_module_private_name(&enumeration.name)
                        })
                    })
            })
        })?;
    enumeration
        .variants
        .iter()
        .find(|variant| variant.name == name)
        .map(|variant| (enumeration.id.clone(), variant.tag))
}

fn resolve_module_place(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarOverloadEnvironment,
    owners: &ScalarAssignmentOwners,
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => resolve_expression_module_places(
            pointer,
            scope,
            module,
            modules,
            names,
            unsafe_context,
            declarations,
            current_module,
            overloads,
            owners,
        ),
        ScalarPlace::Field { base, field, .. } => {
            resolve_module_place(
                base,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            let base_type = match base.as_ref() {
                ScalarPlace::Dereference { pointer, .. } => {
                    let mut scratch = ScalarInferenceBuilder::new(owners);
                    match expression_type_in_module(
                        pointer,
                        scope,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        module,
                        modules,
                        &mut Vec::new(),
                        unsafe_context,
                        overloads,
                        &mut scratch,
                    ) {
                        ScalarType::RawPointer(inner)
                        | ScalarType::CheckedReference { inner, .. } => *inner,
                        _ => ScalarType::Error,
                    }
                }
                ScalarPlace::Name { .. }
                | ScalarPlace::Field { .. }
                | ScalarPlace::Index { .. } => {
                    let mut scratch = ScalarInferenceBuilder::new(owners);
                    place_type_in_module(
                        base,
                        scope,
                        module,
                        modules,
                        &mut Vec::new(),
                        overloads,
                        &mut scratch,
                    )
                }
            };
            let structure = match base_type {
                ScalarType::Struct(id) => modules
                    .iter()
                    .find_map(|module| module.structs.iter().find(|structure| structure.id == id)),
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => modules.iter().find_map(|module| {
                        module.structs.iter().find(|structure| structure.id == *id)
                    }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(declared) = structure.and_then(|structure| {
                structure
                    .fields
                    .iter()
                    .find(|candidate| {
                        matches!(field, ScalarFieldReference::Unresolved { name, .. } if candidate.name == *name)
                    })
            }) {
                *field = ScalarFieldReference::Resolved(declared.id.clone());
            }
        }
        ScalarPlace::Index { base, index, .. } => {
            resolve_module_place(
                base,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
            resolve_expression_module_places(
                index,
                scope,
                module,
                modules,
                names,
                unsafe_context,
                declarations,
                current_module,
                overloads,
                owners,
            );
        }
    }
}
