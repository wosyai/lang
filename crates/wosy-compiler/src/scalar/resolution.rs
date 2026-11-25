use super::typecheck::{
    expression_type, expression_type_in_module, place_type, place_type_in_module,
};
use super::*;

fn secondary_binding_initializers_mut(
    binding: &mut ScalarBinding,
) -> impl Iterator<Item = &mut ScalarExpression> {
    let outputs = match (binding.is_allocation, &binding.output_origin) {
        (false, ScalarBindingOutputOrigin::IndependentExpressions) => {
            binding.output_values.as_mut_slice()
        }
        _ => &mut [],
    };
    outputs.iter_mut().skip(1).map(|output| &mut output.value)
}

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
        .chain(program.enums.iter().filter(|enumeration| enumeration.has_tag_only_layout()).map(|enumeration| {
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
                for initializer in secondary_binding_initializers_mut(binding) {
                    resolve_expression_types(initializer, &names, &[]);
                }
            }
            ScalarItem::Function(function) => {
                resolve_callable_types(function, &names);
                for arm in &mut function.overload_arms {
                    resolve_callable_types(arm, &names);
                }
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

fn resolve_callable_types(function: &mut ScalarFunction, names: &BTreeMap<String, ScalarType>) {
    if function.generic_parameters.is_empty() {
        function.signature = resolve_type(&function.signature, names);
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
    resolve_block_types(&mut function.body, names, &function.generic_parameters);
}

fn resolve_item_types(item: &mut ScalarBlockItem, names: &BTreeMap<String, ScalarType>) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            binding.declared_type = resolve_type(&binding.declared_type, names);
            for receiver in &mut binding.receivers {
                receiver.ty = resolve_type(&receiver.ty, names);
            }
            for initializer in secondary_binding_initializers_mut(binding) {
                resolve_expression_types(initializer, names, &[]);
            }
        }
        ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
            resolve_block_types(block, names, &[])
        }
        ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
    }
}

fn resolve_block_types(
    block: &mut ScalarBlock,
    names: &BTreeMap<String, ScalarType>,
    generics: &[ScalarGenericParameter],
) {
    for item in &mut block.items {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                binding.declared_type =
                    resolve_scoped_type(&binding.declared_type, names, generics);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_scoped_type(&receiver.ty, names, generics);
                }
                resolve_expression_types(&mut binding.value, names, generics);
                for initializer in secondary_binding_initializers_mut(binding) {
                    resolve_expression_types(initializer, names, generics);
                }
            }
            ScalarBlockItem::Expression(expression) => {
                resolve_expression_types(expression, names, generics)
            }
            ScalarBlockItem::Assignment(assignment) => {
                resolve_expression_types(&mut assignment.value, names, generics);
                for value in &mut assignment.values {
                    resolve_expression_types(value, names, generics);
                }
            }
        }
    }
    for output in &mut block.final_output_values {
        resolve_expression_types(&mut output.value, names, generics);
    }
}

fn resolve_expression_types(
    expression: &mut ScalarExpression,
    names: &BTreeMap<String, ScalarType>,
    generics: &[ScalarGenericParameter],
) {
    match expression {
        ScalarExpression::Call {
            receiver,
            type_arguments,
            arguments,
            ..
        } => {
            if receiver.as_deref() != Some("core") {
                for argument in type_arguments {
                    argument.ty = resolve_scoped_type(&argument.ty, names, generics);
                }
            }
            for argument in arguments {
                resolve_expression_types(argument, names, generics);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_types(condition, names, generics);
            resolve_block_types(then_branch, names, generics);
            resolve_block_types(else_branch, names, generics);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_types(condition, names, generics);
            resolve_block_types(then_branch, names, generics);
        }
        ScalarExpression::Block(block) => resolve_block_types(block, names, generics),
        _ => resolve_expression_type_operands(expression, &mut |operand| {
            resolve_expression_types(operand, names, generics);
        }),
    }
}

fn resolve_expression_type_operands(
    expression: &mut ScalarExpression,
    resolve: &mut dyn FnMut(&mut ScalarExpression),
) {
    match expression {
        ScalarExpression::Binary { left, right, .. } => {
            resolve(left);
            resolve(right);
        }
        ScalarExpression::Unary { operand, .. } | ScalarExpression::ValueField { receiver: operand, .. } => resolve(operand),
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve(&mut field.value);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                resolve(element);
            }
        }
        _ => {}
    }
}

fn resolve_type(ty: &ScalarType, names: &BTreeMap<String, ScalarType>) -> ScalarType {
    resolve_scoped_type(ty, names, &[])
}

fn resolve_scoped_type(
    ty: &ScalarType,
    names: &BTreeMap<String, ScalarType>,
    generics: &[ScalarGenericParameter],
) -> ScalarType {
    match ty {
        ScalarType::Named { name, .. }
            if generics.iter().any(|parameter| parameter.name == *name) =>
        {
            ty.clone()
        }
        ScalarType::Named { name, span } => names.get(name).cloned().map_or_else(
            || ScalarType::Named {
                name: name.clone(),
                span: *span,
            },
            |ty| ty,
        ),
        ScalarType::RawPointer(inner) => {
            ScalarType::RawPointer(Box::new(resolve_scoped_type(inner, names, generics)))
        }
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_scoped_type(inner, names, generics)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_scoped_type(element, names, generics)),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::RuntimeArray { element, span } => ScalarType::RuntimeArray {
            element: Box::new(resolve_scoped_type(element, names, generics)),
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
                        ty: resolve_scoped_type(&output.ty, names, generics),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| resolve_scoped_type(parameter, names, generics))
                .collect(),
        },
        value => value.clone(),
    }
}

pub(super) fn resolve_program_places(
    program: &mut ScalarProgram,
    overloads: &ScalarTypingView<'_>,
    owners: &ScalarAssignmentOwners,
) {
    let context = ScalarProgramTyping {
        source: &program.source,
        structs: &program.structs,
        enums: &program.enums,
        target_layout: program.target_layout,
    };
    let names = program
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(program.enums.iter().filter(|enumeration| enumeration.has_tag_only_layout()).map(|enumeration| {
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
                for initializer in secondary_binding_initializers_mut(binding) {
                    resolve_expression_places(
                        initializer,
                        &declarations,
                        &context,
                        &names,
                        false,
                        overloads,
                        owners,
                    );
                }
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
    program: &ScalarProgramTyping<'_>,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
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
            for initializer in secondary_binding_initializers_mut(binding) {
                resolve_expression_places(
                    initializer,
                    scope,
                    program,
                    names,
                    unsafe_context,
                    overloads,
                    owners,
                );
            }
            if binding.contextual_storage().is_none() {
                scope.insert(binding.name.clone(), binding.declared_type.clone());
            }
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
    }
}

fn resolve_block_places(
    block: &mut ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
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
    program: &ScalarProgramTyping<'_>,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    owners: &ScalarAssignmentOwners,
) {
    match expression {
        ScalarExpression::Contextual { .. } => {},
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
        ScalarExpression::Unary { operand, .. } | ScalarExpression::ValueField { receiver: operand, .. } => {
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
                .find(|enumeration| enumeration.name == *receiver && enumeration.has_tag_only_layout())
                .and_then(|enumeration| {
                    enumeration
                        .variants()
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
    program: &ScalarProgramTyping<'_>,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    owners: &ScalarAssignmentOwners,
) {
    match place {
        ScalarPlace::Contextual { .. } => {},
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
                ScalarPlace::Contextual { ty, .. } => ty.clone(),
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
                resolve_binding_types_project(binding, declarations, current_module, &[]);
            }
            ScalarItem::Function(function) => {
                resolve_callable_types_project(function, declarations, current_module);
                for arm in &mut function.overload_arms {
                    resolve_callable_types_project(arm, declarations, current_module);
                }
            }
            ScalarItem::Extern(extern_decl) => {
                for function in &mut extern_decl.functions {
                    function.signature =
                        resolve_type_in_project(&function.signature, declarations, current_module);
                }
            }
            ScalarItem::Executable(item) => {
                resolve_item_types_project(item, declarations, current_module, &[])
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

fn resolve_callable_types_project(
    function: &mut ScalarFunction,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
) {
    if function.generic_parameters.is_empty() {
        function.signature =
            resolve_type_in_project(&function.signature, declarations, current_module);
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
    resolve_block_types_project(
        &mut function.body,
        declarations,
        current_module,
        &function.generic_parameters,
    );
}

fn resolve_binding_types_project(
    binding: &mut ScalarBinding,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    generics: &[ScalarGenericParameter],
) {
    binding.declared_type = resolve_scoped_type_in_project(
        &binding.declared_type,
        declarations,
        current_module,
        generics,
    );
    for receiver in &mut binding.receivers {
        receiver.ty =
            resolve_scoped_type_in_project(&receiver.ty, declarations, current_module, generics);
    }
    resolve_expression_types_project(&mut binding.value, declarations, current_module, generics);
    if binding.output_origin == ScalarBindingOutputOrigin::IndependentExpressions {
        for output in &mut binding.output_values {
            resolve_expression_types_project(
                &mut output.value,
                declarations,
                current_module,
                generics,
            );
        }
    }
}

fn resolve_item_types_project(
    item: &mut ScalarBlockItem,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    generics: &[ScalarGenericParameter],
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            resolve_binding_types_project(binding, declarations, current_module, generics);
        }
        ScalarBlockItem::Expression(expression) => {
            resolve_expression_types_project(expression, declarations, current_module, generics);
        }
        ScalarBlockItem::Assignment(assignment) => {
            for value in &mut assignment.values {
                resolve_expression_types_project(value, declarations, current_module, generics);
            }
            resolve_expression_types_project(
                &mut assignment.value,
                declarations,
                current_module,
                generics,
            );
        }
    }
}

fn resolve_block_types_project(
    block: &mut ScalarBlock,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    generics: &[ScalarGenericParameter],
) {
    for item in &mut block.items {
        resolve_item_types_project(item, declarations, current_module, generics);
    }
    for output in &mut block.final_output_values {
        resolve_expression_types_project(&mut output.value, declarations, current_module, generics);
    }
}

fn resolve_expression_types_project(
    expression: &mut ScalarExpression,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    generics: &[ScalarGenericParameter],
) {
    match expression {
        ScalarExpression::Call {
            receiver,
            type_arguments,
            arguments,
            ..
        } => {
            if receiver.as_deref() != Some("core") {
                for argument in type_arguments {
                    argument.ty = resolve_scoped_type_in_project(
                        &argument.ty,
                        declarations,
                        current_module,
                        generics,
                    );
                }
            }
            for argument in arguments {
                resolve_expression_types_project(argument, declarations, current_module, generics);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_types_project(condition, declarations, current_module, generics);
            resolve_block_types_project(then_branch, declarations, current_module, generics);
            resolve_block_types_project(else_branch, declarations, current_module, generics);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_types_project(condition, declarations, current_module, generics);
            resolve_block_types_project(then_branch, declarations, current_module, generics);
        }
        ScalarExpression::Block(block) => {
            resolve_block_types_project(block, declarations, current_module, generics);
        }
        _ => resolve_expression_type_operands(expression, &mut |operand| {
            resolve_expression_types_project(operand, declarations, current_module, generics);
        }),
    }
}

pub(super) fn resolve_type_in_project(
    ty: &ScalarType,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
) -> ScalarType {
    resolve_scoped_type_in_project(ty, declarations, current_module, &[])
}

fn resolve_scoped_type_in_project(
    ty: &ScalarType,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    generics: &[ScalarGenericParameter],
) -> ScalarType {
    match ty {
        ScalarType::Named { name, .. }
            if generics.iter().any(|parameter| parameter.name == *name) =>
        {
            ty.clone()
        }
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
        ScalarType::RawPointer(inner) => ScalarType::RawPointer(Box::new(
            resolve_scoped_type_in_project(inner, declarations, current_module, generics),
        )),
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_scoped_type_in_project(
                inner,
                declarations,
                current_module,
                generics,
            )),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_scoped_type_in_project(
                element,
                declarations,
                current_module,
                generics,
            )),
            length: *length,
            length_span: *length_span,
            span: *span,
        },
        ScalarType::RuntimeArray { element, span } => ScalarType::RuntimeArray {
            element: Box::new(resolve_scoped_type_in_project(
                element, declarations, current_module, generics,
            )),
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
                        ty: resolve_scoped_type_in_project(
                            &output.ty,
                            declarations,
                            current_module,
                            generics,
                        ),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| {
                    resolve_scoped_type_in_project(
                        parameter,
                        declarations,
                        current_module,
                        generics,
                    )
                })
                .collect(),
        },
        value => value.clone(),
    }
}

pub(super) fn resolve_module_places(
    items: &mut [ScalarItem],
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarTypingView<'_>,
    owners: &ScalarAssignmentOwners,
) {
    let names = module
        .structs
        .iter()
        .map(|structure| {
            (
                structure.name.clone(),
                ScalarType::Struct(structure.id.clone()),
            )
        })
        .chain(module.enums.iter().filter(|enumeration| enumeration.has_tag_only_layout()).map(|enumeration| {
            (
                enumeration.name.clone(),
                ScalarType::Enum(enumeration.id.clone()),
            )
        }))
        .collect::<BTreeMap<_, _>>();
    let bindings = items
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
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                resolve_expression_module_places(
                    &mut binding.value,
                    &bindings,
                    module,
                    modules,
                    &names,
                    false,
                    declarations,
                    current_module,
                    overloads,
                    owners,
                );
                for initializer in secondary_binding_initializers_mut(binding) {
                    resolve_expression_module_places(
                        initializer,
                        &bindings,
                        module,
                        modules,
                        &names,
                        false,
                        declarations,
                        current_module,
                        overloads,
                        owners,
                    );
                }
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
                    module,
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
                    module,
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarTypingView<'_>,
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
            for initializer in secondary_binding_initializers_mut(binding) {
                resolve_expression_module_places(
                    initializer,
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
            if binding.contextual_storage().is_none() {
                scope.insert(binding.name.clone(), binding.declared_type.clone());
            }
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
    }
}

fn resolve_block_module_places(
    block: &mut ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarTypingView<'_>,
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarTypingView<'_>,
    owners: &ScalarAssignmentOwners,
) {
    match expression {
        ScalarExpression::Contextual { binding, ty, .. } => {
            if *ty == ScalarType::Error && matches!(binding.kind, ReferenceBindingKind::ContextualFormal) {
                if let Some(checked) = declarations.contextual_formal_type(binding) { *ty = checked.clone(); }
            }
        }
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
        ScalarExpression::Unary { operand, .. } | ScalarExpression::ValueField { receiver: operand, .. } => {
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    receiver: &str,
    name: &str,
) -> Option<(ScalarEnumId, u32)> {
    let enumeration = module
        .enums
        .iter()
        .find(|enumeration| enumeration.name == receiver && enumeration.has_tag_only_layout())
        .or_else(|| {
            receiver.split_once('.').and_then(|(namespace, enum_name)| {
                module
                    .namespace_bindings
                    .iter()
                    .find(|binding| binding.binding == namespace)
                    .and_then(|binding| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == &binding.target)
                    })
                    .and_then(|target| {
                        target.enums.iter().find(|enumeration| {
                            enumeration.name == enum_name
                                && enumeration.has_tag_only_layout()
                                && !is_module_private_name(&enumeration.name)
                        })
                    })
            })
        })?;
    enumeration
        .variants()
        .iter()
        .find(|variant| variant.name == name)
        .map(|variant| (enumeration.id.clone(), variant.tag))
}

fn resolve_module_place(
    place: &mut ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
    declarations: &ScalarDeclarationEnvironment,
    current_module: usize,
    overloads: &ScalarTypingView<'_>,
    owners: &ScalarAssignmentOwners,
) {
    match place {
        ScalarPlace::Contextual { binding, ty, .. } => {
            if *ty == ScalarType::Error && matches!(binding.kind, ReferenceBindingKind::ContextualFormal) {
                if let Some(checked) = declarations.contextual_formal_type(binding) { *ty = checked.clone(); }
            }
        }
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
                ScalarPlace::Contextual { ty, .. } => ty.clone(),
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
