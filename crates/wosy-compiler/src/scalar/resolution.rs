use super::typecheck::{
    expression_type, expression_type_in_module, place_type, place_type_in_module,
};
use super::*;

pub(super) fn resolve_program_types(
    program: &mut ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let source = program.source.clone();
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
        compute_initial_struct_layout(structure, program.target_layout);
    }
    resolve_program_struct_layouts(program, &source, diagnostics);
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

pub(super) fn resolve_program_places(program: &mut ScalarProgram) {
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
                );
            }
            ScalarItem::Function(function) => {
                let mut scope = declarations.clone();
                if let ScalarType::Callable { parameters, .. } = &function.signature {
                    for (name, ty) in function.parameters.iter().zip(parameters) {
                        scope.insert(name.clone(), ty.clone());
                    }
                }
                resolve_block_places(&mut function.body, &mut scope, &context, &names, false);
            }
            ScalarItem::Executable(item) => {
                resolve_item_places(item, &mut declarations.clone(), &context, &names, false);
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
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            resolve_expression_places(&mut binding.value, scope, program, names, unsafe_context);
            scope.insert(binding.name.clone(), binding.declared_type.clone());
        }
        ScalarBlockItem::Expression(expression) => {
            resolve_expression_places(expression, scope, program, names, unsafe_context)
        }
        ScalarBlockItem::Assignment(assignment) => {
            for target in &mut assignment.targets {
                resolve_place(&mut target.place, scope, program, names, unsafe_context);
            }
            resolve_expression_places(&mut assignment.value, scope, program, names, unsafe_context);
            for value in &mut assignment.values {
                resolve_expression_places(value, scope, program, names, unsafe_context);
            }
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_expression_places(
                &mut while_expression.condition,
                scope,
                program,
                names,
                unsafe_context,
            );
            resolve_block_places(
                &mut while_expression.body,
                scope,
                program,
                names,
                unsafe_context,
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
) {
    let unsafe_context = unsafe_context || block.unsafe_context;
    for item in &mut block.items {
        resolve_item_places(item, scope, program, names, unsafe_context);
    }
    for output in &mut block.final_output_values {
        resolve_expression_places(&mut output.value, scope, program, names, unsafe_context);
    }
}

fn resolve_expression_places(
    expression: &mut ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    names: &BTreeMap<String, ScalarType>,
    unsafe_context: bool,
) {
    match expression {
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => {
            resolve_place(place, scope, program, names, unsafe_context)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            resolve_place(place, scope, program, names, unsafe_context)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_places(&mut field.value, scope, program, names, unsafe_context);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                resolve_expression_places(element, scope, program, names, unsafe_context);
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            resolve_expression_places(left, scope, program, names, unsafe_context);
            resolve_expression_places(right, scope, program, names, unsafe_context);
        }
        ScalarExpression::Unary { operand, .. } => {
            resolve_expression_places(operand, scope, program, names, unsafe_context);
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
                resolve_expression_places(argument, scope, program, names, unsafe_context);
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            resolve_expression_places(condition, scope, program, names, unsafe_context);
            let mut then_scope = scope.clone();
            resolve_block_places(then_branch, &mut then_scope, program, names, unsafe_context);
            let mut else_scope = scope.clone();
            resolve_block_places(else_branch, &mut else_scope, program, names, unsafe_context);
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            resolve_expression_places(condition, scope, program, names, unsafe_context);
            let mut then_scope = scope.clone();
            resolve_block_places(then_branch, &mut then_scope, program, names, unsafe_context);
        }
        ScalarExpression::Block(block) => {
            let mut scope = scope.clone();
            resolve_block_places(block, &mut scope, program, names, unsafe_context);
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
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            resolve_expression_places(pointer, scope, program, names, unsafe_context)
        }
        ScalarPlace::Field { base, field, span } => {
            resolve_place(base, scope, program, names, unsafe_context);
            let base_type = match base.as_ref() {
                ScalarPlace::Dereference { pointer, .. } => {
                    match expression_type(
                        pointer,
                        scope,
                        &BTreeSet::new(),
                        &BTreeMap::new(),
                        program,
                        &mut Vec::new(),
                        unsafe_context,
                    ) {
                        ScalarType::RawPointer(inner)
                        | ScalarType::CheckedReference { inner, .. } => *inner,
                        _ => ScalarType::Error,
                    }
                }
                ScalarPlace::Name { .. }
                | ScalarPlace::Field { .. }
                | ScalarPlace::Index { .. } => place_type(base, scope, program, &mut Vec::new()),
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
            resolve_place(base, scope, program, names, unsafe_context);
            resolve_expression_places(index, scope, program, names, unsafe_context);
        }
    }
}

pub(super) fn resolve_module_types_in_project(module: &mut ScalarModule, modules: &[ScalarModule]) {
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
    for structure in &mut module.structs {
        for field in &mut structure.fields {
            field.ty = resolve_type_in_project(&field.ty, &names, &context, modules);
        }
    }
    for item in &mut module.items {
        match item {
            ScalarItem::Binding(binding) => {
                binding.declared_type =
                    resolve_type_in_project(&binding.declared_type, &names, &context, modules);
                for receiver in &mut binding.receivers {
                    receiver.ty = resolve_type_in_project(&receiver.ty, &names, &context, modules);
                }
            }
            ScalarItem::Function(function) => {
                if function.generic_parameters.is_empty() {
                    function.signature =
                        resolve_type_in_project(&function.signature, &names, &context, modules);
                }
                for arm in &mut function.overload_arms {
                    if arm.generic_parameters.is_empty() {
                        arm.signature =
                            resolve_type_in_project(&arm.signature, &names, &context, modules);
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
                resolve_block_types_project(&mut function.body, &names, &context, modules);
            }
            ScalarItem::Extern(extern_decl) => {
                for function in &mut extern_decl.functions {
                    function.signature =
                        resolve_type_in_project(&function.signature, &names, &context, modules);
                }
            }
            ScalarItem::Executable(item) => {
                resolve_item_types_project(item, &names, &context, modules)
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
    names: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            binding.declared_type =
                resolve_type_in_project(&binding.declared_type, names, module, modules);
            for receiver in &mut binding.receivers {
                receiver.ty = resolve_type_in_project(&receiver.ty, names, module, modules);
            }
        }
        ScalarBlockItem::Expression(ScalarExpression::Block(block)) => {
            resolve_block_types_project(block, names, module, modules)
        }
        ScalarBlockItem::While(while_expression) => {
            resolve_block_types_project(&mut while_expression.body, names, module, modules)
        }
        ScalarBlockItem::Expression(_) | ScalarBlockItem::Assignment(_) => {}
    }
}

fn resolve_block_types_project(
    block: &mut ScalarBlock,
    names: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) {
    for item in &mut block.items {
        resolve_item_types_project(item, names, module, modules);
    }
}

fn resolve_type_in_project(
    ty: &ScalarType,
    names: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) -> ScalarType {
    match ty {
        ScalarType::Named { name, span } => {
            if let Some(id) = names.get(name) {
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
        } => match module
            .namespace_bindings
            .iter()
            .find(|namespace| namespace.binding == *receiver)
            .and_then(|namespace| {
                modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)
            })
            .and_then(|target| {
                target.structs.iter().find(|structure| {
                    structure.name == *member && !is_module_private_name(&structure.name)
                })
            }) {
            Some(structure) => ScalarType::Struct(structure.id.clone()),
            None => module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
                .and_then(|namespace| {
                    modules
                        .iter()
                        .find(|candidate| candidate.source == namespace.target)
                })
                .and_then(|target| {
                    target.enums.iter().find(|enumeration| {
                        enumeration.name == *member && !is_module_private_name(&enumeration.name)
                    })
                })
                .map_or_else(
                    || ScalarType::Qualified {
                        receiver: receiver.clone(),
                        receiver_span: *receiver_span,
                        member: member.clone(),
                        member_span: *member_span,
                        span: *span,
                    },
                    |enumeration| ScalarType::Enum(enumeration.id.clone()),
                ),
        },
        ScalarType::RawPointer(inner) => ScalarType::RawPointer(Box::new(resolve_type_in_project(
            inner, names, module, modules,
        ))),
        ScalarType::CheckedReference { mutability, inner } => ScalarType::CheckedReference {
            mutability: *mutability,
            inner: Box::new(resolve_type_in_project(inner, names, module, modules)),
        },
        ScalarType::Array {
            element,
            length,
            length_span,
            span,
        } => ScalarType::Array {
            element: Box::new(resolve_type_in_project(element, names, module, modules)),
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
                        ty: resolve_type_in_project(&output.ty, names, module, modules),
                        span: output.span,
                    })
                    .collect(),
                span: outputs.span,
            },
            parameters: parameters
                .iter()
                .map(|parameter| resolve_type_in_project(parameter, names, module, modules))
                .collect(),
        },
        value => value.clone(),
    }
}

fn resolve_program_struct_layouts(
    program: &mut ScalarProgram,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (cycles, order) = aggregate_layout_order(&program.structs);
    for cycle in cycles {
        let (structure_index, field_index) = cycle_participant(&program.structs, &cycle);
        let field = &program.structs[structure_index].fields[field_index];
        recursive_layout_diagnostic(source, field.name_span, diagnostics);
        for index in cycle {
            clear_struct_layout(&mut program.structs[index]);
        }
    }
    for index in order {
        let structs = program.structs.clone();
        recompute_struct_layout(
            &mut program.structs[index],
            &structs,
            program.target_layout,
            source,
            diagnostics,
        );
    }
}

pub(super) fn resolve_project_struct_layouts(
    project: &mut ScalarProject,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let locations = project
        .modules
        .iter()
        .enumerate()
        .flat_map(|(module_index, module)| {
            module
                .structs
                .iter()
                .enumerate()
                .map(move |(structure_index, structure)| {
                    (module_index, structure_index, structure.clone())
                })
        })
        .collect::<Vec<_>>();
    let structures = locations
        .iter()
        .map(|(_, _, structure)| structure.clone())
        .collect::<Vec<_>>();
    let (cycles, order) = aggregate_layout_order(&structures);
    for cycle in cycles {
        let (participant, field_index) = cycle_participant(&structures, &cycle);
        let (module_index, structure_index, _) = locations[participant];
        let field = &project.modules[module_index].structs[structure_index].fields[field_index];
        let source = project.modules[module_index].source.clone();
        recursive_layout_diagnostic(&source, field.name_span, diagnostics);
        for index in cycle {
            let (module_index, structure_index, _) = locations[index];
            clear_struct_layout(&mut project.modules[module_index].structs[structure_index]);
        }
    }
    for index in order {
        let (module_index, structure_index, _) = locations[index];
        let modules = project.modules.clone();
        let target_layout = project.modules[module_index].target_layout;
        let source = project.modules[module_index].source.clone();
        recompute_struct_layout_project(
            &mut project.modules[module_index].structs[structure_index],
            &modules,
            target_layout,
            &source,
            diagnostics,
        );
    }
}

fn aggregate_layout_order(structures: &[ScalarStruct]) -> (Vec<Vec<usize>>, Vec<usize>) {
    let indices = structures
        .iter()
        .enumerate()
        .map(|(index, structure)| (structure.id.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let dependencies = structures
        .iter()
        .map(|structure| {
            let mut ids = Vec::new();
            for field in &structure.fields {
                by_value_struct_dependencies(&field.ty, &mut ids);
            }
            ids.into_iter()
                .filter_map(|id| indices.get(&id).copied())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let components = aggregate_layout_components(&dependencies);
    let mut cyclic = vec![false; structures.len()];
    let mut cycles = Vec::new();
    for component in components {
        let self_edge = component.len() == 1 && dependencies[component[0]].contains(&component[0]);
        if component.len() > 1 || self_edge {
            for &index in &component {
                cyclic[index] = true;
            }
            cycles.push(component);
        }
    }
    cycles.sort_unstable_by_key(|cycle| cycle_participant(structures, cycle));
    let mut visited = vec![false; structures.len()];
    let mut order = Vec::new();
    for index in 0..structures.len() {
        aggregate_layout_visit(index, &dependencies, &cyclic, &mut visited, &mut order);
    }
    (cycles, order)
}

fn by_value_struct_dependencies(ty: &ScalarType, dependencies: &mut Vec<ScalarStructId>) {
    match ty {
        ScalarType::Struct(id) => dependencies.push(id.clone()),
        ScalarType::Array { element, .. } => by_value_struct_dependencies(element, dependencies),
        _ => {}
    }
}

fn aggregate_layout_components(dependencies: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut index = 0;
    let mut indices = vec![None; dependencies.len()];
    let mut lowlinks = vec![0; dependencies.len()];
    let mut stack = Vec::new();
    let mut on_stack = vec![false; dependencies.len()];
    let mut components = Vec::new();
    for node in 0..dependencies.len() {
        if indices[node].is_none() {
            aggregate_layout_component_visit(
                node,
                dependencies,
                &mut index,
                &mut indices,
                &mut lowlinks,
                &mut stack,
                &mut on_stack,
                &mut components,
            );
        }
    }
    components
}

fn aggregate_layout_component_visit(
    node: usize,
    dependencies: &[Vec<usize>],
    index: &mut usize,
    indices: &mut [Option<usize>],
    lowlinks: &mut [usize],
    stack: &mut Vec<usize>,
    on_stack: &mut [bool],
    components: &mut Vec<Vec<usize>>,
) {
    indices[node] = Some(*index);
    lowlinks[node] = *index;
    *index += 1;
    stack.push(node);
    on_stack[node] = true;
    for &dependency in &dependencies[node] {
        if indices[dependency].is_none() {
            aggregate_layout_component_visit(
                dependency,
                dependencies,
                index,
                indices,
                lowlinks,
                stack,
                on_stack,
                components,
            );
            lowlinks[node] = lowlinks[node].min(lowlinks[dependency]);
        } else if on_stack[dependency] {
            lowlinks[node] = lowlinks[node].min(indices[dependency].expect("component index"));
        }
    }
    if lowlinks[node] == indices[node].expect("component index") {
        let mut component = Vec::new();
        loop {
            let member = stack.pop().expect("component member");
            on_stack[member] = false;
            component.push(member);
            if member == node {
                break;
            }
        }
        component.sort_unstable();
        components.push(component);
    }
}

fn aggregate_layout_visit(
    node: usize,
    dependencies: &[Vec<usize>],
    cyclic: &[bool],
    visited: &mut [bool],
    order: &mut Vec<usize>,
) {
    if visited[node] || cyclic[node] {
        return;
    }
    visited[node] = true;
    for &dependency in &dependencies[node] {
        aggregate_layout_visit(dependency, dependencies, cyclic, visited, order);
    }
    order.push(node);
}

fn cycle_participant(structures: &[ScalarStruct], cycle: &[usize]) -> (usize, usize) {
    for &structure_index in cycle {
        for (field_index, field) in structures[structure_index].fields.iter().enumerate() {
            let mut dependencies = Vec::new();
            by_value_struct_dependencies(&field.ty, &mut dependencies);
            if dependencies.iter().any(|dependency| {
                cycle
                    .iter()
                    .any(|&candidate| structures[candidate].id == *dependency)
            }) {
                return (structure_index, field_index);
            }
        }
    }
    panic!("recursive aggregate component has a participating field")
}

fn recursive_layout_diagnostic(
    source: &SourceIdentity,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    diagnostics.push(super::Diagnostic {
        code: "B0003".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: "recursive by-value struct layout is unsupported".to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(source.clone(), span),
            message: "field participates in a recursive struct layout".to_owned(),
        }],
        notes: Vec::new(),
    });
}

fn recompute_struct_layout(
    structure: &mut ScalarStruct,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let mut offset = 0;
    let mut alignment = 1;
    clear_struct_layout(structure);
    for field in &mut structure.fields {
        let layout = match layout_for_type_in_structs(&field.ty, structs, target_layout) {
            Ok(layout) => layout,
            Err(error) => {
                layout_error_source(source, error, diagnostics);
                return;
            }
        };
        alignment = alignment.max(layout.alignment);
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            layout_error_source(
                source,
                LayoutError::AggregateSizeOverflow(field.name_span),
                diagnostics,
            );
            clear_struct_layout(structure);
            return;
        };
        field.offset = Some(field_offset);
        field.layout = Some(layout.clone());
        offset = next_offset;
    }
    let Some(size) = checked_align_offset(offset, alignment) else {
        layout_error_source(
            source,
            LayoutError::AggregateSizeOverflow(structure.name_span),
            diagnostics,
        );
        clear_struct_layout(structure);
        return;
    };
    structure.layout = Some(ScalarLayout { size, alignment });
}

fn recompute_struct_layout_project(
    structure: &mut ScalarStruct,
    modules: &[ScalarModule],
    target_layout: ScalarTargetLayout,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let mut offset = 0;
    let mut alignment = 1;
    clear_struct_layout(structure);
    for field in &mut structure.fields {
        let layout = match layout_for_type_in_modules(&field.ty, modules, target_layout) {
            Ok(layout) => layout,
            Err(error) => {
                layout_error_source(source, error, diagnostics);
                return;
            }
        };
        alignment = alignment.max(layout.alignment);
        let Some((field_offset, next_offset)) = checked_layout_offset(offset, &layout) else {
            layout_error_source(
                source,
                LayoutError::AggregateSizeOverflow(field.name_span),
                diagnostics,
            );
            clear_struct_layout(structure);
            return;
        };
        field.offset = Some(field_offset);
        field.layout = Some(layout.clone());
        offset = next_offset;
    }
    let Some(size) = checked_align_offset(offset, alignment) else {
        layout_error_source(
            source,
            LayoutError::AggregateSizeOverflow(structure.name_span),
            diagnostics,
        );
        clear_struct_layout(structure);
        return;
    };
    structure.layout = Some(ScalarLayout { size, alignment });
}

fn layout_for_type_in_structs(
    ty: &ScalarType,
    structs: &[ScalarStruct],
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    match ty {
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type_in_structs(element, structs, target_layout)?;
            Ok(ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            })
        }
        ScalarType::Struct(id) => structs
            .iter()
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.layout.clone())
            .ok_or(LayoutError::InvalidType),
        _ => layout_for_type(ty, target_layout),
    }
}

fn layout_for_type_in_modules(
    ty: &ScalarType,
    modules: &[ScalarModule],
    target_layout: ScalarTargetLayout,
) -> Result<ScalarLayout, LayoutError> {
    match ty {
        ScalarType::Array {
            element,
            length,
            length_span,
            ..
        } => {
            let element = layout_for_type_in_modules(element, modules, target_layout)?;
            Ok(ScalarLayout {
                size: element
                    .size
                    .checked_mul(*length)
                    .ok_or(LayoutError::ArraySizeOverflow(*length_span))?,
                alignment: element.alignment,
            })
        }
        ScalarType::Struct(id) => modules
            .iter()
            .flat_map(|module| module.structs.iter())
            .find(|structure| structure.id == *id)
            .and_then(|structure| structure.layout.clone())
            .ok_or(LayoutError::InvalidType),
        _ => layout_for_type(ty, target_layout),
    }
}

pub(super) fn resolve_module_places(module: &mut ScalarModule, modules: &[ScalarModule]) {
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
    let declarations = module
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
                    &declarations,
                    &context,
                    modules,
                    &names,
                    false,
                );
            }
            ScalarItem::Function(function) => {
                let mut scope = declarations.clone();
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
                );
            }
            ScalarItem::Executable(item) => {
                let mut scope = declarations.clone();
                resolve_item_module_places(item, &mut scope, &context, modules, &names, false);
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
                );
            }
            resolve_expression_module_places(
                &mut assignment.value,
                scope,
                module,
                modules,
                names,
                unsafe_context,
            );
            for value in &mut assignment.values {
                resolve_expression_module_places(
                    value,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
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
            );
            resolve_block_module_places(
                &mut while_expression.body,
                scope,
                module,
                modules,
                names,
                unsafe_context,
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
) {
    let unsafe_context = unsafe_context || block.unsafe_context;
    for item in &mut block.items {
        resolve_item_module_places(item, scope, module, modules, names, unsafe_context);
    }
    for output in &mut block.final_output_values {
        resolve_expression_module_places(
            &mut output.value,
            scope,
            module,
            modules,
            names,
            unsafe_context,
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
) {
    match expression {
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => {
            resolve_module_place(place, scope, module, modules, names, unsafe_context)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            resolve_module_place(place, scope, module, modules, names, unsafe_context)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                resolve_expression_module_places(
                    &mut field.value,
                    scope,
                    module,
                    modules,
                    names,
                    unsafe_context,
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
                );
            }
        }
        ScalarExpression::Binary { left, right, .. } => {
            resolve_expression_module_places(left, scope, module, modules, names, unsafe_context);
            resolve_expression_module_places(right, scope, module, modules, names, unsafe_context);
        }
        ScalarExpression::Unary { operand, .. } => {
            resolve_expression_module_places(
                operand,
                scope,
                module,
                modules,
                names,
                unsafe_context,
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
                    argument.ty = resolve_type_in_project(&argument.ty, names, module, modules);
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
            );
            let mut then_scope = scope.clone();
            resolve_block_module_places(
                then_branch,
                &mut then_scope,
                module,
                modules,
                names,
                unsafe_context,
            );
            let mut else_scope = scope.clone();
            resolve_block_module_places(
                else_branch,
                &mut else_scope,
                module,
                modules,
                names,
                unsafe_context,
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
            );
            let mut then_scope = scope.clone();
            resolve_block_module_places(
                then_branch,
                &mut then_scope,
                module,
                modules,
                names,
                unsafe_context,
            );
        }
        ScalarExpression::Block(block) => {
            let mut scope = scope.clone();
            resolve_block_module_places(block, &mut scope, module, modules, names, unsafe_context);
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
) {
    match place {
        ScalarPlace::Name { .. } => {}
        ScalarPlace::Dereference { pointer, .. } => {
            resolve_expression_module_places(pointer, scope, module, modules, names, unsafe_context)
        }
        ScalarPlace::Field { base, field, .. } => {
            resolve_module_place(base, scope, module, modules, names, unsafe_context);
            let base_type = match base.as_ref() {
                ScalarPlace::Dereference { pointer, .. } => match expression_type_in_module(
                    pointer,
                    scope,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    module,
                    modules,
                    &mut Vec::new(),
                    unsafe_context,
                ) {
                    ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
                        *inner
                    }
                    _ => ScalarType::Error,
                },
                ScalarPlace::Name { .. }
                | ScalarPlace::Field { .. }
                | ScalarPlace::Index { .. } => {
                    place_type_in_module(base, scope, module, modules, &mut Vec::new())
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
            resolve_module_place(base, scope, module, modules, names, unsafe_context);
            resolve_expression_module_places(index, scope, module, modules, names, unsafe_context);
        }
    }
}
