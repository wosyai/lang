use super::*;

pub(super) fn validate_module_type(
    module: &ScalarModule,
    ty: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { span, .. } | ScalarType::Qualified { span, .. } => diagnostics.push(
            module_diagnostic(module, "B0003", "unknown scalar type", *span),
        ),
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_module_type(module, &output.ty, output.span, diagnostics);
            }
            for parameter in parameters {
                validate_module_type(module, parameter, span, diagnostics);
            }
        }
        ScalarType::Unit
        | ScalarType::Bool
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::U128
        | ScalarType::F32
        | ScalarType::F64
        | ScalarType::Char
        | ScalarType::ArtifactId => {}
        ScalarType::CheckedReference { inner, .. } => {
            validate_module_type(module, inner, span, diagnostics)
        }
        ScalarType::RawPointer(inner)
            if matches!(inner.as_ref(), ScalarType::U8 | ScalarType::Struct(_)) => {}
        ScalarType::RawPointer(_) => diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "unsupported raw pointer type",
            span,
        )),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
            validate_module_type(module, element, span, diagnostics)
        }
        ScalarType::Struct(_) | ScalarType::Enum(_) => {}
        ScalarType::Error => {}
    }
}

pub(super) fn validate_module_generic_type(
    module: &ScalarModule,
    ty: &ScalarType,
    span: ByteSpan,
    generic_parameters: &[ScalarGenericParameter],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { name, .. }
            if generic_parameters
                .iter()
                .any(|parameter| parameter.name == *name) => {}
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            validate_module_generic_type(module, inner, span, generic_parameters, diagnostics)
        }
        ScalarType::Array { element, .. } => {
            validate_module_generic_type(module, element, span, generic_parameters, diagnostics)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_module_generic_type(
                    module,
                    &output.ty,
                    output.span,
                    generic_parameters,
                    diagnostics,
                );
            }
            for parameter in parameters {
                validate_module_generic_type(
                    module,
                    parameter,
                    span,
                    generic_parameters,
                    diagnostics,
                );
            }
        }
        _ => validate_module_type(module, ty, span, diagnostics),
    }
}

pub(super) fn validate_extern(
    module: &ScalarModule,
    extern_decl: &ScalarExtern,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let valid_binding = extern_decl.kind == "wasm"
        && matches!(extern_decl.actual_module, ScalarExternModule::Valid(_))
        && !extern_decl.functions.is_empty();
    if !valid_binding {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "unsupported extern declaration",
            extern_decl.span,
        ));
    }
    for function in &extern_decl.functions {
        validate_identifier_style(module, &function.name, function.name_span, diagnostics);
        validate_module_type(
            module,
            &function.signature,
            function.signature_span,
            diagnostics,
        );
        if contains_fixed_array(&function.signature) {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "direct FFI arrays are not supported",
                function.signature_span,
            ));
        }
        if contains_checked_reference(&function.signature) {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "direct FFI checked references are not supported",
                function.signature_span,
            ));
        }
    }
}

fn contains_checked_reference(ty: &ScalarType) -> bool {
    match ty {
        ScalarType::CheckedReference { .. } => true,
        ScalarType::RawPointer(inner) | ScalarType::Array { element: inner, .. } => {
            contains_checked_reference(inner)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            outputs
                .outputs
                .iter()
                .any(|output| contains_checked_reference(&output.ty))
                || parameters.iter().any(contains_checked_reference)
        }
        _ => false,
    }
}

fn contains_fixed_array(ty: &ScalarType) -> bool {
    match ty {
        ScalarType::Array { .. } => true,
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            contains_fixed_array(inner)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            outputs
                .outputs
                .iter()
                .any(|output| contains_fixed_array(&output.ty))
                || parameters.iter().any(contains_fixed_array)
        }
        _ => false,
    }
}

pub(super) fn call_output_sequence_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
) -> Option<ScalarOutputSequence> {
    let ScalarExpression::Call {
        receiver,
        name,
        type_arguments,
        arguments,
        ..
    } = expression
    else {
        return None;
    };
    let (callable, target, member_visible) = match receiver {
        None => (scope.get(name), module, true),
        Some(binding) => {
            let namespace = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding);
            if let Some(namespace) = namespace {
                let target = modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)?;
                (
                    target.members.get(name),
                    target,
                    target.members.contains_key(name),
                )
            } else {
                (
                    module.items.iter().find_map(|item| match item {
                        ScalarItem::Extern(extern_decl) if extern_decl.binding == *binding => {
                            extern_decl
                                .functions
                                .iter()
                                .find(|function| function.name == *name)
                                .map(|function| &function.signature)
                        }
                        _ => None,
                    }),
                    module,
                    true,
                )
            }
        }
    };
    if member_visible {
        if let Some(overload) = target.items.iter().find_map(|item| match item {
            ScalarItem::Function(function)
                if function.name == *name && !function.overload_arms.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        }) {
            let argument_types = arguments
                .iter()
                .map(|argument| overload_argument_type(argument, scope))
                .collect::<Option<Vec<_>>>()?;
            let ScalarType::Callable { outputs, .. } =
                resolve_overload_candidate(overload, &argument_types, None)
                    .ok()?
                    .0
            else {
                return None;
            };
            return Some(outputs);
        }
    }
    let callable = callable?;
    let extern_callable = match receiver {
        None => false,
        Some(binding) => {
            if let Some(namespace) = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding)
            {
                modules
                    .iter()
                    .find(|candidate| candidate.source == namespace.target)
                    .is_some_and(|target| {
                        target.items.iter().any(|item| {
                            matches!(
                                item,
                                ScalarItem::Extern(extern_decl)
                                    if extern_decl.functions.iter().any(|function| function.name == *name)
                            )
                        })
                    })
            } else {
                module.items.iter().any(|item| {
                    matches!(
                        item,
                        ScalarItem::Extern(extern_decl)
                            if extern_decl.binding == *binding
                                && extern_decl.functions.iter().any(|function| function.name == *name)
                    )
                })
            }
        }
    };
    if extern_callable && !type_arguments.is_empty() {
        return None;
    }
    let generic_parameters = if member_visible {
        target.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == *name => {
                Some(&function.generic_parameters)
            }
            _ => None,
        })
    } else {
        None
    };
    let callable = if let Some(parameters) = generic_parameters {
        substitute_generic_callable(callable, parameters, type_arguments)?
    } else {
        callable.clone()
    };
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs)
}

fn call_output_sequence(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
) -> Option<ScalarOutputSequence> {
    let ScalarExpression::Call {
        receiver,
        name,
        type_arguments,
        arguments,
        ..
    } = expression
    else {
        return None;
    };
    let callable = if receiver.is_none() {
        scope.get(name)
    } else {
        program.items.iter().find_map(|item| match item {
            ScalarItem::Extern(extern_decl)
                if receiver.as_deref() == Some(extern_decl.binding.as_str()) =>
            {
                extern_decl
                    .functions
                    .iter()
                    .find(|function| function.name == *name)
                    .map(|function| &function.signature)
            }
            _ => None,
        })
    }?;
    let extern_callable = program.items.iter().any(|item| {
        matches!(
            item,
            ScalarItem::Extern(extern_decl)
                if receiver.as_deref() == Some(extern_decl.binding.as_str())
                    && extern_decl.functions.iter().any(|function| function.name == *name)
        )
    });
    if extern_callable && !type_arguments.is_empty() {
        return None;
    }
    if let Some(overload) = receiver
        .is_none()
        .then(|| {
            program.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if function.name == *name && !function.overload_arms.is_empty() =>
                {
                    Some(function)
                }
                _ => None,
            })
        })
        .flatten()
    {
        let argument_types = arguments
            .iter()
            .map(|argument| overload_argument_type(argument, scope))
            .collect::<Option<Vec<_>>>()?;
        let ScalarType::Callable { outputs, .. } =
            resolve_overload_candidate(overload, &argument_types, None)
                .ok()?
                .0
        else {
            return None;
        };
        return Some(outputs);
    }
    let generic_parameters = receiver.is_none().then(|| {
        program.items.iter().find_map(|item| match item {
            ScalarItem::Function(function) if function.name == *name => {
                Some(&function.generic_parameters)
            }
            _ => None,
        })
    });
    let callable = if let Some(Some(parameters)) = generic_parameters {
        substitute_generic_callable(callable, parameters, type_arguments)?
    } else {
        callable.clone()
    };
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs)
}

fn scalar_call_result(outputs: &ScalarOutputSequence) -> ScalarType {
    outputs
        .outputs
        .first()
        .map_or(ScalarType::Unit, |output| output.ty.clone())
}

fn scalar_call_result_in_module(outputs: &ScalarOutputSequence) -> ScalarType {
    outputs
        .outputs
        .first()
        .map_or(ScalarType::Unit, |output| output.ty.clone())
}

pub(super) fn expression_span(expression: &ScalarExpression) -> ByteSpan {
    match expression {
        ScalarExpression::Name { span, .. }
        | ScalarExpression::Integer { span, .. }
        | ScalarExpression::InvalidInteger { span, .. }
        | ScalarExpression::Float { span, .. }
        | ScalarExpression::InvalidFloat { span }
        | ScalarExpression::Boolean { span, .. }
        | ScalarExpression::Char { span, .. }
        | ScalarExpression::Utf8 { span, .. }
        | ScalarExpression::RawAddress { span, .. }
        | ScalarExpression::CheckedAddress { span, .. }
        | ScalarExpression::Dereference { span, .. }
        | ScalarExpression::IndexedRead { span, .. }
        | ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::ArrayLiteral { span, .. }
        | ScalarExpression::Binary { span, .. }
        | ScalarExpression::Unary { span, .. }
        | ScalarExpression::Call { span, .. }
        | ScalarExpression::If { span, .. }
        | ScalarExpression::UnitIf { span, .. } => *span,
        ScalarExpression::Member { span, .. } => *span,
        ScalarExpression::Block(block) => block.span,
    }
}

pub(super) fn validate_output_receivers_in_module(
    expression: &ScalarExpression,
    receivers: &[ScalarOutputReceiver],
    binding_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let Some(outputs) = call_output_sequence_in_module(expression, scope, module, modules) else {
        return;
    };
    if receivers.len() > outputs.outputs.len() {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "call has fewer outputs than receivers",
            binding_span,
        ));
    }
    for (receiver, output) in receivers.iter().zip(&outputs.outputs) {
        expect_module_type(module, &output.ty, &receiver.ty, receiver.span, diagnostics);
    }
}

fn validate_output_receivers(
    expression: &ScalarExpression,
    receivers: &[ScalarOutputReceiver],
    binding_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let Some(outputs) = call_output_sequence(expression, scope, program) else {
        return;
    };
    if receivers.len() > outputs.outputs.len() {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "call has fewer outputs than receivers",
            binding_span,
        ));
    }
    for (receiver, output) in receivers.iter().zip(&outputs.outputs) {
        expect_type(
            program,
            &output.ty,
            &receiver.ty,
            receiver.span,
            diagnostics,
        );
    }
}

pub(super) fn block_type_in_module(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = block_item_type_in_module(
            item,
            &mut scope,
            &mut visible_names,
            &mut folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

pub(super) fn block_type_in_module_expected(
    block: &ScalarBlock,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = match item {
            ScalarBlockItem::Expression(expression)
                if index + 1 == block.items.len() && !block.terminated_items[index] =>
            {
                expression_type_in_module_expected(
                    expression,
                    expected,
                    &scope,
                    &visible_names,
                    &folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                )
            }
            _ => block_item_type_in_module(
                item,
                &mut scope,
                &mut visible_names,
                &mut folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            ),
        };
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

pub(super) fn block_item_type_in_module(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            for receiver in &binding.receivers {
                validate_identifier_style(module, &receiver.name, receiver.name_span, diagnostics);
            }
            if binding.is_allocation {
                for receiver in &binding.receivers {
                    let Some(length) = &receiver.allocation_length else {
                        continue;
                    };
                    let actual = expression_type_in_module_expected(
                        length,
                        Some(&ScalarType::U64),
                        scope,
                        visible_names,
                        folded_names,
                        module,
                        modules,
                        diagnostics,
                        unsafe_context,
                    );
                    expect_module_type(
                        module,
                        &ScalarType::U64,
                        &actual,
                        receiver.span,
                        diagnostics,
                    );
                }
                if declare_module_name(
                    module,
                    &binding.name,
                    binding.span,
                    visible_names,
                    folded_names,
                    diagnostics,
                ) {
                    for receiver in &binding.receivers {
                        scope.insert(receiver.name.clone(), receiver.ty.clone());
                    }
                }
                return ScalarType::Unit;
            }
            let actual = expression_type_in_module_expected(
                &binding.value,
                Some(&binding.declared_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            validate_output_receivers_in_module(
                &binding.value,
                &binding.receivers,
                binding.span,
                scope,
                module,
                modules,
                diagnostics,
            );
            expect_module_type(
                module,
                &binding.declared_type,
                &actual,
                binding.span,
                diagnostics,
            );
            if declare_module_name(
                module,
                &binding.name,
                binding.span,
                visible_names,
                folded_names,
                diagnostics,
            ) {
                scope.insert(binding.name.clone(), binding.declared_type.clone());
                for receiver in &binding.receivers {
                    scope.insert(receiver.name.clone(), receiver.ty.clone());
                }
            }
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => expression_type_in_module(
            expression,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
        ScalarBlockItem::Assignment(assignment) => assignment_type_in_module(
            assignment,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
        ScalarBlockItem::While(while_expression) => while_type_in_module(
            while_expression,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
    }
}

fn assignment_type_in_module(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut inferred_names = visible_names.clone();
    let mut inferred_folded_names = folded_names.clone();
    let expected = assignment
        .targets
        .iter()
        .map(|target| assignment_target_type_in_module(target, scope, module, modules, diagnostics))
        .collect::<Vec<_>>();
    validate_assignment_target_distinctness_in_module(assignment, module, diagnostics);
    let outputs = assignment
        .values
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let actual = expression_type_in_module_expected(
                value,
                expected.get(position).and_then(Option::as_ref),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            match call_output_sequence_in_module(value, scope, module, modules) {
                Some(outputs) => Some(outputs.outputs),
                None if matches!(value, ScalarExpression::Call { .. })
                    && is_error_type(&actual) =>
                {
                    None
                }
                None => Some(vec![ScalarOutput {
                    ty: actual,
                    span: expression_span(value),
                }]),
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|outputs| ScalarOutputSequence {
            outputs: outputs.into_iter().flatten().collect(),
            span: assignment.span,
        });
    if let Some(outputs) = outputs {
        record_assignment_outputs(
            &module.resolved_assignment_outputs,
            &module.source,
            &module.items,
            assignment,
            &outputs,
        );
        if assignment.targets.len() > outputs.outputs.len() {
            diagnostics.push(module_diagnostic(
                module,
                "B0004",
                "call has fewer outputs than assignment targets",
                assignment.span,
            ));
        }
        for ((index, (target, expected)), output) in assignment
            .targets
            .iter()
            .zip(&expected)
            .enumerate()
            .zip(&outputs.outputs)
        {
            if let Some(expected) = expected {
                expect_module_type(
                    module,
                    expected,
                    &output.ty,
                    if index == 0 {
                        assignment.span
                    } else {
                        target.target_span
                    },
                    diagnostics,
                );
            } else if let ScalarPlace::Name { name, span } = &target.place {
                validate_identifier_style(module, name, *span, diagnostics);
                if declare_module_name(
                    module,
                    name,
                    *span,
                    &mut inferred_names,
                    &mut inferred_folded_names,
                    diagnostics,
                ) {
                    scope.insert(name.clone(), output.ty.clone());
                }
            }
        }
    }
    ScalarType::Unit
}

fn assignment_target_type_in_module(
    target: &ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> Option<ScalarType> {
    if let Some(receiver) = &target.receiver {
        if let Some(namespace) = module
            .namespace_bindings
            .iter()
            .find(|namespace| namespace.binding == *receiver)
        {
            let Some(target_module) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown assignment target",
                    target
                        .receiver_span
                        .expect("qualified assignment receiver span"),
                ));
                return Some(ScalarType::Error);
            };
            let Some(expected) = target_module.members.get(&target.target).cloned() else {
                diagnostics.push(unknown_member_diagnostic(
                    module,
                    target.target_span,
                    &target_module.source,
                ));
                return Some(ScalarType::Error);
            };
            if is_const_binding_name(&target.target) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    target.target_span,
                ));
            }
            return Some(expected);
        }
    }
    match &target.place {
        ScalarPlace::Name { name, span } => {
            let expected = scope.get(name).cloned();
            if expected.is_some() && is_const_binding_name(name) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    *span,
                ));
            }
            expected
        }
        place => Some(writable_place_type_in_module(
            place,
            target.span,
            scope,
            module,
            modules,
            diagnostics,
        )),
    }
}

fn validate_assignment_target_distinctness_in_module(
    assignment: &ScalarAssignment,
    module: &ScalarModule,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (index, target) in assignment.targets.iter().enumerate() {
        for previous in &assignment.targets[..index] {
            if scalar_place_identity(&target.place) == scalar_place_identity(&previous.place) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "duplicate assignment target",
                    target.target_span,
                ));
            } else if place_contains_dynamic_index(&target.place)
                || place_contains_dynamic_index(&previous.place)
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "assignment targets require a distinctness proof",
                    target.target_span,
                ));
            }
        }
    }
}

fn while_type_in_module(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let condition = expression_type_in_module(
        &while_expression.condition,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    if !is_error_type(&condition) && !scalar_type_equal(&condition, &ScalarType::Bool) {
        diagnostics.push(module_diagnostic(
            module,
            "B0005",
            "while expression requires bool",
            while_expression.span,
        ));
    }
    let _ = block_type_in_module(
        &while_expression.body,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    ScalarType::Unit
}

fn type_core_memory_in_module(
    operation: &str,
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if matches!(operation, "alloc" | "free" | "invalidate" | "rebind") && !unsafe_context {
        diagnostics.push(module_diagnostic(
            module,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    match operation {
        "alloc" => {
            if arguments.len() != 2 {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "core.alloc requires size and alignment",
                    span,
                ));
                return ScalarType::Error;
            }
            for argument in arguments {
                let actual = expression_type_in_module_expected(
                    argument,
                    Some(&ScalarType::U64),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                expect_module_type(
                    module,
                    &ScalarType::U64,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        }
        "free" => {
            let [pointer] = arguments else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "core.free requires one pointer",
                    span,
                ));
                return ScalarType::Error;
            };
            let actual = expression_type_in_module(
                pointer,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            if !matches!(
                actual,
                ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. }
            ) && !is_error_type(&actual)
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "core.free requires a raw pointer or checked allocation place",
                    expression_span(pointer),
                ));
                return ScalarType::Error;
            }
            if matches!(actual, ScalarType::CheckedReference { .. })
                && !matches!(
                    pointer,
                    ScalarExpression::CheckedAddress { .. } | ScalarExpression::Name { .. }
                )
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "core.free requires a checked allocation place",
                    expression_span(pointer),
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        "invalidate" | "rebind" => {
            let expected_arity = if operation == "invalidate" { 1 } else { 3 };
            if arguments.len() != expected_arity {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    &format!("core.{operation} requires {expected_arity} arguments"),
                    span,
                ));
                return ScalarType::Error;
            }
            let address = &arguments[0];
            let actual = expression_type_in_module(
                address,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&actual) && !matches!(actual, ScalarType::CheckedReference { .. }) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    &format!("core.{operation} requires a checked reference to an allocation"),
                    expression_span(address),
                ));
            }
            if operation == "rebind" {
                let raw = &arguments[1];
                let raw_type = expression_type_in_module(
                    raw,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                if !is_error_type(&raw_type) && !matches!(raw_type, ScalarType::RawPointer(_)) {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "core.rebind requires a raw backing address",
                        expression_span(raw),
                    ));
                }
                let length = &arguments[2];
                let length_type = expression_type_in_module_expected(
                    length,
                    Some(&ScalarType::U64),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                expect_module_type(
                    module,
                    &ScalarType::U64,
                    &length_type,
                    expression_span(length),
                    diagnostics,
                );
            }
            ScalarType::Unit
        }
        "system_panic" => {
            if !arguments.is_empty() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "core.system_panic accepts no arguments",
                    span,
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        _ => unreachable!(),
    }
}

fn type_core_memory(
    operation: &str,
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if matches!(operation, "alloc" | "free" | "invalidate" | "rebind") && !unsafe_context {
        diagnostics.push(diagnostic(
            program,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    match operation {
        "alloc" => {
            if arguments.len() != 2 {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "core.alloc requires size and alignment",
                    span,
                ));
                return ScalarType::Error;
            }
            for argument in arguments {
                let actual = expression_type_expected(
                    argument,
                    &ScalarType::U64,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    &ScalarType::U64,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            ScalarType::RawPointer(Box::new(ScalarType::U8))
        }
        "free" => {
            let [pointer] = arguments else {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "core.free requires one pointer",
                    span,
                ));
                return ScalarType::Error;
            };
            let actual = expression_type(
                pointer,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !matches!(
                actual,
                ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. }
            ) && !is_error_type(&actual)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "core.free requires a raw pointer or checked allocation place",
                    expression_span(pointer),
                ));
                return ScalarType::Error;
            }
            if matches!(actual, ScalarType::CheckedReference { .. })
                && !matches!(
                    pointer,
                    ScalarExpression::CheckedAddress { .. } | ScalarExpression::Name { .. }
                )
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "core.free requires a checked allocation place",
                    expression_span(pointer),
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        "invalidate" | "rebind" => {
            let expected_arity = if operation == "invalidate" { 1 } else { 3 };
            if arguments.len() != expected_arity {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    &format!("core.{operation} requires {expected_arity} arguments"),
                    span,
                ));
                return ScalarType::Error;
            }
            let address = &arguments[0];
            let actual = expression_type(
                address,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&actual) && !matches!(actual, ScalarType::CheckedReference { .. }) {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    &format!("core.{operation} requires a checked reference to an allocation"),
                    expression_span(address),
                ));
            }
            if operation == "rebind" {
                let raw = &arguments[1];
                let raw_type = expression_type(
                    raw,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                if !is_error_type(&raw_type) && !matches!(raw_type, ScalarType::RawPointer(_)) {
                    diagnostics.push(diagnostic(
                        program,
                        "B0003",
                        "core.rebind requires a raw backing address",
                        expression_span(raw),
                    ));
                }
                let length = &arguments[2];
                let length_type = expression_type_expected(
                    length,
                    &ScalarType::U64,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    &ScalarType::U64,
                    &length_type,
                    expression_span(length),
                    diagnostics,
                );
            }
            ScalarType::Unit
        }
        "system_panic" => {
            if !arguments.is_empty() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "core.system_panic accepts no arguments",
                    span,
                ));
                return ScalarType::Error;
            }
            ScalarType::Unit
        }
        _ => unreachable!(),
    }
}

fn type_core_int_conversion_in_module(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !is_integer_type(destination) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} has an invalid integer destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = expression_type_in_module_expected(
        &arguments[0],
        source_context.as_ref(),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    let source_width = integer_width(&actual);
    let destination_width = integer_width(destination);
    if !is_error_type(&actual) && (source_width.is_none() || destination_width.is_none()) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires integer source and destination types"),
            expression_span(&arguments[0]),
        ));
    }
    if source_width.is_some_and(|source| {
        destination_width.is_some_and(|destination| match operation {
            "int_trunc" => source <= destination,
            "int_extend" => source >= destination,
            _ => unreachable!(),
        })
    }) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate integer width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || source_width.is_none() {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn is_bitcast_scalar(ty: &ScalarType) -> bool {
    is_integer_type(ty)
        || matches!(
            ty,
            ScalarType::Char | ScalarType::Bool | ScalarType::F32 | ScalarType::F64
        )
}

fn type_core_bitcast_in_module(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !is_bitcast_scalar(destination) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} has an invalid bitcast destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let actual = expression_type_in_module_expected(
        &arguments[0],
        None,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    if is_error_type(&actual) {
        return ScalarType::Error;
    }
    if !is_bitcast_scalar(&actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a compatible scalar source type"),
            expression_span(&arguments[0]),
        ));
        return ScalarType::Error;
    }
    let sizes_match = match (
        layout_for_type(&actual, module.target_layout),
        layout_for_type(destination, module.target_layout),
    ) {
        (Ok(source), Ok(destination)) => source.size == destination.size,
        _ => false,
    };
    if !sizes_match {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires source and destination types of equal size"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    destination.clone()
}

fn type_core_pointer_cast_in_module(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if !unsafe_context {
        diagnostics.push(module_diagnostic(
            module,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    let source_context = match destination {
        ScalarType::RawPointer(_) => destination.clone(),
        ScalarType::CheckedReference { inner, .. } => ScalarType::RawPointer(inner.clone()),
        _ => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                &format!("core.{operation} has an invalid pointer destination type"),
                type_arguments[0].span,
            ));
            return ScalarType::Error;
        }
    };
    if arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let actual = expression_type_in_module_expected(
        &arguments[0],
        Some(&source_context),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    if is_error_type(&actual) {
        return ScalarType::Error;
    }
    if !matches!(actual, ScalarType::RawPointer(_)) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a raw pointer source type"),
            expression_span(&arguments[0]),
        ));
        return ScalarType::Error;
    }
    destination.clone()
}
fn float_conversion_destination_matches(operation: &str, destination: &ScalarType) -> bool {
    match operation {
        "uint_to_float" | "sint_to_float" => is_float_type(destination),
        "float_to_sint_trunc" => matches!(
            destination,
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128
        ),
        "float_to_uint_trunc" => matches!(
            destination,
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128
        ),
        "float_trunc" => *destination == ScalarType::F32,
        "float_extend" => *destination == ScalarType::F64,
        _ => unreachable!(),
    }
}

fn float_conversion_source_matches(operation: &str, actual: &ScalarType) -> bool {
    match operation {
        "uint_to_float" => matches!(
            actual,
            ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128
        ),
        "sint_to_float" => matches!(
            actual,
            ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 | ScalarType::I128
        ),
        "float_to_sint_trunc" | "float_to_uint_trunc" | "float_trunc" | "float_extend" => {
            is_float_type(actual)
        }
        _ => unreachable!(),
    }
}

fn float_conversion_direction_mismatched(
    operation: &str,
    destination: &ScalarType,
    actual: &ScalarType,
) -> bool {
    if !is_float_type(destination) || !is_float_type(actual) {
        return false;
    }
    match operation {
        "float_trunc" => !(*destination == ScalarType::F32 && *actual == ScalarType::F64),
        "float_extend" => !(*destination == ScalarType::F64 && *actual == ScalarType::F32),
        _ => false,
    }
}

fn type_core_float_conversion_in_module(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !float_conversion_destination_matches(operation, destination) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} has an invalid destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = expression_type_in_module_expected(
        &arguments[0],
        source_context.as_ref(),
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    let source_valid = float_conversion_source_matches(operation, &actual);
    if !is_error_type(&actual) && !source_valid {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a compatible source type"),
            expression_span(&arguments[0]),
        ));
    }
    if float_conversion_direction_mismatched(operation, destination, &actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate float width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || !source_valid {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn type_core_float_conversion(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !float_conversion_destination_matches(operation, destination) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} has an invalid destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = match source_context {
        Some(source) => expression_type_expected(
            &arguments[0],
            &source,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        None => expression_type(
            &arguments[0],
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
    };
    let source_valid = float_conversion_source_matches(operation, &actual);
    if !is_error_type(&actual) && !source_valid {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a compatible source type"),
            expression_span(&arguments[0]),
        ));
    }
    if float_conversion_direction_mismatched(operation, destination, &actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate float width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || !source_valid {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn type_core_int_conversion(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !is_integer_type(destination) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} has an invalid integer destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let source_context = integer_conversion_source_context(operation, destination, &arguments[0]);
    let actual = match source_context {
        Some(source) => expression_type_expected(
            &arguments[0],
            &source,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        None => expression_type(
            &arguments[0],
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
    };
    let source_width = integer_width(&actual);
    let destination_width = integer_width(destination);
    if !is_error_type(&actual) && (source_width.is_none() || destination_width.is_none()) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires integer source and destination types"),
            expression_span(&arguments[0]),
        ));
    }
    if source_width.is_some_and(|source| {
        destination_width.is_some_and(|destination| match operation {
            "int_trunc" => source <= destination,
            "int_extend" => source >= destination,
            _ => unreachable!(),
        })
    }) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a destination with the appropriate integer width"),
            type_arguments[0].span,
        ));
    }
    if is_error_type(&actual) || source_width.is_none() {
        ScalarType::Error
    } else {
        destination.clone()
    }
}

fn type_core_bitcast(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    if !is_bitcast_scalar(destination) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} has an invalid bitcast destination type"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let actual = expression_type(
        &arguments[0],
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    if is_error_type(&actual) {
        return ScalarType::Error;
    }
    if !is_bitcast_scalar(&actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a compatible scalar source type"),
            expression_span(&arguments[0]),
        ));
        return ScalarType::Error;
    }
    let sizes_match = match (
        layout_for_type(&actual, program.target_layout),
        layout_for_type(destination, program.target_layout),
    ) {
        (Ok(source), Ok(destination)) => source.size == destination.size,
        _ => false,
    };
    if !sizes_match {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires source and destination types of equal size"),
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    destination.clone()
}

fn type_core_pointer_cast(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if !unsafe_context {
        diagnostics.push(diagnostic(
            program,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one destination type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let destination = &type_arguments[0].ty;
    let source_context = match destination {
        ScalarType::RawPointer(_) => destination.clone(),
        ScalarType::CheckedReference { inner, .. } => ScalarType::RawPointer(inner.clone()),
        _ => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                &format!("core.{operation} has an invalid pointer destination type"),
                type_arguments[0].span,
            ));
            return ScalarType::Error;
        }
    };
    if arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one value argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let actual = expression_type_expected(
        &arguments[0],
        &source_context,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    if is_error_type(&actual) {
        return ScalarType::Error;
    }
    if !matches!(actual, ScalarType::RawPointer(_)) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            &format!("core.{operation} requires a raw pointer source type"),
            expression_span(&arguments[0]),
        ));
        return ScalarType::Error;
    }
    destination.clone()
}
fn type_core_raw_memory_in_module(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if !unsafe_context {
        diagnostics.push(module_diagnostic(
            module,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    if type_arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            &format!("core.{operation} requires one pointee type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let pointee = &type_arguments[0].ty;
    if operation == "load"
        && matches!(
            pointee,
            ScalarType::Unit | ScalarType::Callable { .. } | ScalarType::Error
        )
    {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "core.load has an invalid pointee type",
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if operation == "offset" && arguments.len() != 2 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "core.offset requires a pointer and an offset",
            span,
        ));
        return ScalarType::Error;
    }
    if operation == "load" && arguments.len() != 1 {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "core.load requires one pointer",
            span,
        ));
        return ScalarType::Error;
    }
    let pointer = expression_type_in_module(
        &arguments[0],
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    );
    let mut valid = true;
    if !is_error_type(&pointer) {
        match &pointer {
            ScalarType::RawPointer(inner) => {
                if !is_error_type(pointee) && !scalar_type_equal(inner, pointee) {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        &format!("core.{operation} requires a raw pointer to the pointee type"),
                        expression_span(&arguments[0]),
                    ));
                    valid = false;
                }
            }
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    &format!("core.{operation} requires a raw pointer"),
                    expression_span(&arguments[0]),
                ));
                valid = false;
            }
        }
    }
    if operation == "offset" {
        let count = expression_type_in_module_expected(
            &arguments[1],
            Some(&ScalarType::I64),
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        expect_module_type(
            module,
            &ScalarType::I64,
            &count,
            expression_span(&arguments[1]),
            diagnostics,
        );
        if is_error_type(&count) {
            valid = false;
        }
    }
    if !valid || is_error_type(&pointer) || is_error_type(pointee) {
        return ScalarType::Error;
    }
    if operation == "offset" {
        ScalarType::RawPointer(Box::new(pointee.clone()))
    } else {
        pointee.clone()
    }
}

fn type_core_raw_memory(
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if !unsafe_context {
        diagnostics.push(diagnostic(
            program,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    if type_arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            &format!("core.{operation} requires one pointee type argument"),
            span,
        ));
        return ScalarType::Error;
    }
    let pointee = &type_arguments[0].ty;
    if operation == "load"
        && matches!(
            pointee,
            ScalarType::Unit | ScalarType::Callable { .. } | ScalarType::Error
        )
    {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "core.load has an invalid pointee type",
            type_arguments[0].span,
        ));
        return ScalarType::Error;
    }
    if operation == "offset" && arguments.len() != 2 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "core.offset requires a pointer and an offset",
            span,
        ));
        return ScalarType::Error;
    }
    if operation == "load" && arguments.len() != 1 {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "core.load requires one pointer",
            span,
        ));
        return ScalarType::Error;
    }
    let pointer = expression_type(
        &arguments[0],
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    let mut valid = true;
    if !is_error_type(&pointer) {
        match &pointer {
            ScalarType::RawPointer(inner) => {
                if !is_error_type(pointee) && !scalar_type_equal(inner, pointee) {
                    diagnostics.push(diagnostic(
                        program,
                        "B0003",
                        &format!("core.{operation} requires a raw pointer to the pointee type"),
                        expression_span(&arguments[0]),
                    ));
                    valid = false;
                }
            }
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    &format!("core.{operation} requires a raw pointer"),
                    expression_span(&arguments[0]),
                ));
                valid = false;
            }
        }
    }
    if operation == "offset" {
        let count = expression_type_expected(
            &arguments[1],
            &ScalarType::I64,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        );
        expect_type(
            program,
            &ScalarType::I64,
            &count,
            expression_span(&arguments[1]),
            diagnostics,
        );
        if is_error_type(&count) {
            valid = false;
        }
    }
    if !valid || is_error_type(&pointer) || is_error_type(pointee) {
        return ScalarType::Error;
    }
    if operation == "offset" {
        ScalarType::RawPointer(Box::new(pointee.clone()))
    } else {
        pointee.clone()
    }
}

fn integer_conversion_source_context(
    operation: &str,
    destination: &ScalarType,
    expression: &ScalarExpression,
) -> Option<ScalarType> {
    if matches!(
        operation,
        "float_to_sint_trunc" | "float_to_uint_trunc" | "float_trunc" | "float_extend"
    ) {
        return float_conversion_source_context(operation, expression);
    }
    let value = match expression {
        ScalarExpression::Integer { value, .. } => value.clone(),
        ScalarExpression::Unary {
            operator: UnaryOperator::Negate,
            operand,
            ..
        } => match operand.as_ref() {
            ScalarExpression::Integer { value, .. } => -value,
            _ => return None,
        },
        _ => return None,
    };
    let source_types: &[ScalarType] = match operation {
        "int_trunc" => &[
            ScalarType::I64,
            ScalarType::U64,
            ScalarType::I128,
            ScalarType::U128,
        ],
        "int_extend" => &[
            ScalarType::I32,
            ScalarType::U32,
            ScalarType::I64,
            ScalarType::U64,
            ScalarType::I128,
            ScalarType::U128,
        ],
        "uint_to_float" => &[ScalarType::U32, ScalarType::U64, ScalarType::U128],
        "sint_to_float" => &[ScalarType::I32, ScalarType::I64, ScalarType::I128],
        _ => unreachable!(),
    };
    source_types
        .iter()
        .find(|source| {
            let Some(source_width) = integer_width(source) else {
                return false;
            };
            let width_matches = match operation {
                "int_trunc" | "int_extend" => {
                    let Some(destination_width) = integer_width(destination) else {
                        return false;
                    };
                    match operation {
                        "int_trunc" => source_width > destination_width,
                        "int_extend" => source_width < destination_width,
                        _ => unreachable!(),
                    }
                }
                "uint_to_float" | "sint_to_float" => true,
                _ => unreachable!(),
            };
            width_matches && integer_literal_fits_type(&value, source)
        })
        .cloned()
}

fn float_conversion_source_context(
    operation: &str,
    expression: &ScalarExpression,
) -> Option<ScalarType> {
    let is_float_literal = match expression {
        ScalarExpression::Float { .. } => true,
        ScalarExpression::Unary {
            operator: UnaryOperator::Negate,
            operand,
            ..
        } => matches!(operand.as_ref(), ScalarExpression::Float { .. }),
        _ => false,
    };
    if !is_float_literal {
        return None;
    }
    match operation {
        "float_trunc" => Some(ScalarType::F64),
        "float_extend" => Some(ScalarType::F32),
        "float_to_sint_trunc" | "float_to_uint_trunc" => {
            let value = match expression {
                ScalarExpression::Float { value, .. } => *value,
                ScalarExpression::Unary {
                    operator: UnaryOperator::Negate,
                    operand,
                    ..
                } => match operand.as_ref() {
                    ScalarExpression::Float { value, .. } => -*value,
                    _ => return None,
                },
                _ => return None,
            };
            [ScalarType::F32, ScalarType::F64]
                .into_iter()
                .find(|source| float_literal_fits_type(value, source))
        }
        _ => unreachable!(),
    }
}

fn float_literal_fits_type(value: f64, ty: &ScalarType) -> bool {
    match ty {
        ScalarType::F32 => (value as f32).is_finite(),
        ScalarType::F64 => value.is_finite(),
        _ => false,
    }
}

fn integer_literal_fits_type(value: &BigInt, ty: &ScalarType) -> bool {
    let (minimum, maximum) = match ty {
        ScalarType::I8 => (BigInt::from(i8::MIN), BigInt::from(i8::MAX)),
        ScalarType::I16 => (BigInt::from(i16::MIN), BigInt::from(i16::MAX)),
        ScalarType::I32 => (BigInt::from(i32::MIN), BigInt::from(i32::MAX)),
        ScalarType::I64 => (BigInt::from(i64::MIN), BigInt::from(i64::MAX)),
        ScalarType::I128 => (BigInt::from(i128::MIN), BigInt::from(i128::MAX)),
        ScalarType::U8 => (BigInt::from(0), BigInt::from(u8::MAX)),
        ScalarType::U16 => (BigInt::from(0), BigInt::from(u16::MAX)),
        ScalarType::U32 => (BigInt::from(0), BigInt::from(u32::MAX)),
        ScalarType::U64 => (BigInt::from(0), BigInt::from(u64::MAX)),
        ScalarType::U128 => (BigInt::from(0), BigInt::from(u128::MAX)),
        _ => return false,
    };
    value >= &minimum && value <= &maximum
}

pub(super) fn expression_type_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match expression {
        ScalarExpression::Name { name, span } if name == "null" => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "null requires a pointer context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::RawAddress { place, span } => {
            if !unsafe_context {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0012",
                    "raw address requires an unsafe block",
                    *span,
                ));
            }
            ScalarType::RawPointer(Box::new(place_type_in_module(
                place,
                scope,
                module,
                modules,
                diagnostics,
            )))
        }
        ScalarExpression::CheckedAddress {
            mutability,
            place,
            span,
        } => checked_address_type_in_module(
            *mutability,
            place,
            *span,
            scope,
            module,
            modules,
            diagnostics,
        ),
        ScalarExpression::Dereference { place, span } => checked_dereference_type_in_module(
            place,
            *span,
            scope,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
        ScalarExpression::IndexedRead { place, .. } => {
            place_type_in_module(place, scope, module, modules, diagnostics)
        }
        ScalarExpression::StructLiteral { .. } => ScalarType::Error,
        ScalarExpression::ArrayLiteral { span, .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "array literal requires a fixed array context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            name_span,
            span,
            ..
        } => {
            if let Some((enum_id, _)) = enum_member_in_module(module, modules, receiver, name) {
                if enum_tag.is_none() {
                    diagnostics.push(module_diagnostic(
                        module,
                        "M0002",
                        "unknown enum variant",
                        *name_span,
                    ));
                    return ScalarType::Error;
                }
                return ScalarType::Enum(enum_id);
            }
            if let Some(receiver_type) = scope.get(receiver) {
                if matches!(receiver_type, ScalarType::Struct(_))
                    || matches!(
                        receiver_type,
                        ScalarType::RawPointer(inner)
                            if matches!(inner.as_ref(), ScalarType::Struct(_))
                    )
                {
                    return field_type_in_module(
                        receiver_type,
                        name,
                        *name_span,
                        *span,
                        module,
                        modules,
                        diagnostics,
                    );
                }
            }
            let Some(namespace) = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
            else {
                diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
                return ScalarType::Error;
            };
            let Some(target) = modules
                .iter()
                .find(|candidate| candidate.source == namespace.target)
            else {
                diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
                return ScalarType::Error;
            };
            let Some(member) = target.members.get(name) else {
                diagnostics.push(unknown_member_diagnostic(
                    module,
                    *name_span,
                    &target.source,
                ));
                return ScalarType::Error;
            };
            member.clone()
        }
        ScalarExpression::Integer { value, span } => {
            validate_integer_range(module, value, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::Float { .. } => ScalarType::Error,
        ScalarExpression::InvalidFloat { .. } => ScalarType::Error,
        ScalarExpression::InvalidInteger {
            span: _,
            error_span,
        } => {
            if error_span.is_some() {
                ScalarType::Error
            } else {
                ScalarType::I32
            }
        }
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Char { .. } => ScalarType::Char,
        ScalarExpression::Utf8 { span, .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "string literal requires a std.utf8 context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Unary {
            operator,
            operand,
            span,
        } => {
            if matches!(operator, UnaryOperator::Negate) {
                if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                    let value = -value;
                    validate_integer_range(module, &value, *span, diagnostics);
                    return ScalarType::I32;
                }
            }
            let operand_type = expression_type_in_module(
                operand,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            match operator {
                UnaryOperator::LogicalNot => {
                    expect_module_type(
                        module,
                        &ScalarType::Bool,
                        &operand_type,
                        *span,
                        diagnostics,
                    );
                    ScalarType::Bool
                }
                UnaryOperator::BitwiseNot => {
                    expect_module_integer(module, &operand_type, *span, diagnostics);
                    operand_type
                }
                UnaryOperator::Negate => {
                    expect_module_integer(module, &operand_type, *span, diagnostics);
                    operand_type
                }
            }
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            if comparison && (is_null_expression(left) || is_null_expression(right)) {
                return expression_type_in_module_expected(
                    expression,
                    None,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            let left_type = expression_type_in_module(
                left,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let right_type = if matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            ) {
                expression_type_in_module_expected(
                    right,
                    Some(&left_type),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                )
            } else {
                expression_type_in_module(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                )
            };
            if is_error_type(&left_type) || is_error_type(&right_type) {
                return ScalarType::Error;
            }
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            let boolean = matches!(operator, BinaryOperator::And | BinaryOperator::Or);
            let bitwise = matches!(
                operator,
                BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
            );
            if boolean {
                expect_module_type(module, &ScalarType::Bool, &left_type, *span, diagnostics);
                expect_module_type(module, &ScalarType::Bool, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if comparison {
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if bitwise {
                expect_module_integer(module, &left_type, *span, diagnostics);
                expect_module_integer(module, &right_type, *span, diagnostics);
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                left_type
            } else {
                expect_module_type(module, &ScalarType::I32, &left_type, *span, diagnostics);
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                ScalarType::I32
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            name_span,
            arguments,
            span,
            type_arguments,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "this_artifact_id" {
                if !arguments.is_empty() {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                return ScalarType::ArtifactId;
            }
            if receiver.as_deref() == Some("core") && name == "declare_artifact" {
                if arguments.len() != 1 {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                let actual = expression_type_in_module_expected(
                    &arguments[0],
                    Some(&utf8_type_from_namespace_target(
                        module,
                        modules,
                        *span,
                        diagnostics,
                    )),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                let expected = utf8_type_from_namespace_target(module, modules, *span, diagnostics);
                expect_module_type(module, &expected, &actual, *span, diagnostics);
                return if is_error_type(&actual) {
                    ScalarType::Error
                } else {
                    ScalarType::ArtifactId
                };
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "alloc" | "free" | "invalidate" | "rebind" | "system_panic"
                )
            {
                return type_core_memory_in_module(
                    name,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return type_core_int_conversion_in_module(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "uint_to_float"
                        | "sint_to_float"
                        | "float_to_sint_trunc"
                        | "float_to_uint_trunc"
                        | "float_trunc"
                        | "float_extend"
                )
            {
                return type_core_float_conversion_in_module(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return type_core_bitcast_in_module(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core") && matches!(name.as_str(), "offset" | "load") {
                return type_core_raw_memory_in_module(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return type_core_pointer_cast_in_module(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            if let Some(receiver_type) = receiver.as_ref().and_then(|binding| scope.get(binding)) {
                if let ScalarType::Struct(id) = receiver_type {
                    let structure = modules.iter().find_map(|candidate| {
                        candidate
                            .structs
                            .iter()
                            .find(|structure| structure.id == *id)
                    });
                    let field = structure.and_then(|structure| {
                        resolved_struct_field(receiver_type, name, std::slice::from_ref(structure))
                    });
                    let Some(field) = field else {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0001",
                            "unknown callable field",
                            *name_span,
                        ));
                        return ScalarType::Error;
                    };
                    let ScalarType::Callable {
                        outputs,
                        parameters,
                    } = &field.ty
                    else {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0003",
                            "struct field is not callable",
                            *name_span,
                        ));
                        return ScalarType::Error;
                    };
                    if !type_arguments.is_empty() || arguments.len() != parameters.len() {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0004",
                            "call argument arity does not match callable type",
                            *span,
                        ));
                        return ScalarType::Error;
                    }
                    let mut error_argument = false;
                    for (argument, parameter) in arguments.iter().zip(parameters) {
                        let actual = expression_type_in_module_expected(
                            argument,
                            Some(parameter),
                            scope,
                            visible_names,
                            folded_names,
                            module,
                            modules,
                            diagnostics,
                            unsafe_context,
                        );
                        expect_module_type(
                            module,
                            parameter,
                            &actual,
                            expression_span(argument),
                            diagnostics,
                        );
                        error_argument |= is_error_type(&actual);
                    }
                    return if error_argument {
                        ScalarType::Error
                    } else {
                        scalar_call_result_in_module(outputs)
                    };
                }
            }
            let overload = match receiver {
                None => module.items.iter().find_map(|item| match item {
                    ScalarItem::Function(function)
                        if function.name == *name && !function.overload_arms.is_empty() =>
                    {
                        Some(function)
                    }
                    _ => None,
                }),
                Some(binding) => module
                    .namespace_bindings
                    .iter()
                    .find(|namespace| namespace.binding == *binding)
                    .and_then(|namespace| {
                        modules
                            .iter()
                            .find(|candidate| candidate.source == namespace.target)
                    })
                    .and_then(|target| {
                        target.members.get(name).and_then(|_| {
                            target.items.iter().find_map(|item| match item {
                                ScalarItem::Function(function)
                                    if function.name == *name
                                        && !function.overload_arms.is_empty() =>
                                {
                                    Some(function)
                                }
                                _ => None,
                            })
                        })
                    }),
            };
            if overload.is_some() {
                return expression_type_in_module_expected(
                    expression,
                    None,
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
            }
            let (callable, target, unsafe_callable, extern_callable) = match receiver {
                None => (scope.get(name), module, false, false),
                Some(binding) => {
                    let namespace = module
                        .namespace_bindings
                        .iter()
                        .find(|namespace| namespace.binding == *binding);
                    if let Some(namespace) = namespace {
                        let Some(target) = modules
                            .iter()
                            .find(|module| module.source == namespace.target)
                        else {
                            diagnostics.push(module_diagnostic(
                                module,
                                "B0001",
                                "unknown callable name",
                                *span,
                            ));
                            return ScalarType::Error;
                        };
                        let Some(callable) = target.members.get(name) else {
                            diagnostics.push(unknown_member_diagnostic(
                                module,
                                *name_span,
                                &target.source,
                            ));
                            return ScalarType::Error;
                        };
                        (
                            Some(callable),
                            target,
                            false,
                            target.items.iter().any(|item| {
                                matches!(
                                    item,
                                    ScalarItem::Extern(extern_decl)
                                        if extern_decl.functions.iter().any(|function| function.name == *name)
                                )
                            }),
                        )
                    } else {
                        let extern_decl = module.items.iter().find_map(|item| match item {
                            ScalarItem::Extern(extern_decl) if extern_decl.binding == *binding => {
                                Some(extern_decl)
                            }
                            _ => None,
                        });
                        let Some(extern_decl) = extern_decl else {
                            diagnostics.push(module_diagnostic(
                                module,
                                "B0001",
                                "unknown callable name",
                                *span,
                            ));
                            return ScalarType::Error;
                        };
                        let Some(function) = extern_decl
                            .functions
                            .iter()
                            .find(|function| function.name == *name)
                        else {
                            diagnostics.push(module_diagnostic(
                                module,
                                "B0001",
                                "unknown callable name",
                                *span,
                            ));
                            return ScalarType::Error;
                        };
                        (
                            Some(&function.signature),
                            module,
                            function.unsafe_marker,
                            true,
                        )
                    }
                }
            };
            let Some(callable) = callable else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown callable name",
                    *span,
                ));
                return ScalarType::Error;
            };
            if extern_callable && !type_arguments.is_empty() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "generic argument arity does not match callable declaration",
                    *span,
                ));
                return ScalarType::Error;
            }
            let generic_parameters = target.items.iter().find_map(|item| match item {
                ScalarItem::Function(function) if function.name == *name => {
                    Some(&function.generic_parameters)
                }
                _ => None,
            });
            let callable = if let Some(generic_parameters) = generic_parameters {
                let Some(callable) =
                    substitute_generic_callable(&callable, generic_parameters, type_arguments)
                else {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0004",
                        "generic argument arity does not match callable declaration",
                        *span,
                    ));
                    return ScalarType::Error;
                };
                callable
            } else {
                callable.clone()
            };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = &callable
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0001",
                    "unknown callable name",
                    *span,
                ));
                return ScalarType::Error;
            };
            if arguments.len() != parameters.len() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            let mut error_argument = false;
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let actual = expression_type_in_module_expected(
                    argument,
                    Some(parameter),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                expect_module_type(target, parameter, &actual, *span, diagnostics);
                error_argument |= is_error_type(&actual);
            }
            if unsafe_callable && !unsafe_context {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0012",
                    "unsafe extern call requires an unsafe block",
                    *span,
                ));
            }
            if error_argument {
                ScalarType::Error
            } else {
                scalar_call_result_in_module(outputs)
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let condition_type = expression_type_in_module(
                condition,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            expect_module_type(
                module,
                &ScalarType::Bool,
                &condition_type,
                *span,
                diagnostics,
            );
            let then_type = block_type_in_module(
                then_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let else_type = block_type_in_module(
                else_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && !scalar_type_equal(&then_type, &else_type)
            {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            if is_error_type(&condition_type) || is_error_type(&then_type) {
                ScalarType::Error
            } else if is_error_type(&else_type) {
                ScalarType::Error
            } else {
                then_type
            }
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            let condition_type = expression_type_in_module(
                condition,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            expect_module_type(
                module,
                &ScalarType::Bool,
                &condition_type,
                expression_span(condition),
                diagnostics,
            );
            block_type_in_module(
                then_branch,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            ScalarType::Unit
        }
        ScalarExpression::Block(block) => block_type_in_module(
            block,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
    }
}

pub(super) fn expression_type_in_module_expected(
    expression: &ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if expected.is_some_and(is_error_type) {
        return ScalarType::Error;
    }
    if let ScalarExpression::Call {
        receiver,
        name,
        name_span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        let overload = match receiver {
            None => module.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if function.name == *name && !function.overload_arms.is_empty() =>
                {
                    Some(function)
                }
                _ => None,
            }),
            Some(binding) => module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding)
                .and_then(|namespace| {
                    modules
                        .iter()
                        .find(|candidate| candidate.source == namespace.target)
                })
                .and_then(|target| {
                    target.members.get(name).and_then(|_| {
                        target.items.iter().find_map(|item| match item {
                            ScalarItem::Function(function)
                                if function.name == *name && !function.overload_arms.is_empty() =>
                            {
                                Some(function)
                            }
                            _ => None,
                        })
                    })
                }),
        };
        if let Some(overload) = overload {
            if let Some(type_argument) = type_arguments.first() {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0004",
                    "overload calls do not accept explicit type arguments",
                    type_argument.span,
                ));
                return ScalarType::Error;
            }
            let argument_types = arguments
                .iter()
                .map(|argument| {
                    expression_type_in_module(
                        argument,
                        scope,
                        visible_names,
                        folded_names,
                        module,
                        modules,
                        diagnostics,
                        unsafe_context,
                    )
                })
                .collect::<Vec<_>>();
            let overload_arguments = arguments
                .iter()
                .zip(&argument_types)
                .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                .collect::<Vec<_>>();
            let callable = match resolve_overload_candidate(overload, &overload_arguments, expected)
            {
                Ok((callable, _)) => callable,
                Err(message) => {
                    diagnostics.push(module_diagnostic(module, "B0004", message, *name_span));
                    return ScalarType::Error;
                }
            };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
            else {
                unreachable!("overload arms are callable")
            };
            for (argument, parameter) in arguments.iter().zip(&parameters) {
                let actual = expression_type_in_module_expected(
                    argument,
                    Some(parameter),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                );
                expect_module_type(
                    module,
                    parameter,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            return scalar_call_result_in_module(&outputs);
        }
    }
    if let ScalarExpression::Unary {
        operator: UnaryOperator::Negate,
        operand,
        span,
    } = expression
    {
        if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
            if let Some(
                expected @ (ScalarType::I8
                | ScalarType::I16
                | ScalarType::I32
                | ScalarType::I64
                | ScalarType::I128
                | ScalarType::U8
                | ScalarType::U16
                | ScalarType::U32
                | ScalarType::U64
                | ScalarType::U128),
            ) = expected
            {
                let value = -value;
                validate_integer_range_for_type(module, &value, *span, expected, diagnostics);
                return expected.clone();
            }
        }
    }
    if let ScalarExpression::If {
        condition,
        then_branch,
        else_branch,
        span,
    } = expression
    {
        let condition_type = expression_type_in_module(
            condition,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        if !is_error_type(&condition_type) && !scalar_type_equal(&condition_type, &ScalarType::Bool)
        {
            diagnostics.push(module_diagnostic(
                module,
                "B0005",
                "conditional expression requires bool",
                *span,
            ));
        }
        let then_type = block_type_in_module_expected(
            then_branch,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        let else_type = block_type_in_module_expected(
            else_branch,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
        if !is_error_type(&then_type)
            && !is_error_type(&else_type)
            && !scalar_type_equal(&then_type, &else_type)
        {
            diagnostics.push(module_diagnostic(
                module,
                "B0006",
                "conditional branches must have equal types",
                *span,
            ));
        }
        return then_type;
    }
    if let ScalarExpression::Name { name, span } = expression {
        if name == "null" {
            if let Some(
                pointer @ ScalarType::RawPointer(_) | pointer @ ScalarType::CheckedReference { .. },
            ) = expected
            {
                return pointer.clone();
            }
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "null requires a pointer context",
                *span,
            ));
            return ScalarType::Error;
        }
    }
    if let ScalarExpression::Utf8 { span, .. } = expression {
        let Some(expected) = expected else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "string literal requires a std.utf8 context",
                *span,
            ));
            return ScalarType::Error;
        };
        if let Some(utf8) = utf8_type_from_context(module, modules, expected) {
            return utf8;
        }
        let utf8 = utf8_type_from_namespace_target(module, modules, *span, diagnostics);
        expect_module_type(module, &utf8, expected, *span, diagnostics);
        return ScalarType::Error;
    }
    if let ScalarExpression::ArrayLiteral { elements, span } = expression {
        let Some(ScalarType::Array {
            element, length, ..
        }) = expected
        else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "array literal requires a fixed array context",
                *span,
            ));
            return ScalarType::Error;
        };
        if elements.len() as u64 != *length {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "array literal element count does not match fixed array length",
                *span,
            ));
        }
        for value in elements {
            let actual = expression_type_in_module_expected(
                value,
                Some(element),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            expect_module_type(
                module,
                element,
                &actual,
                expression_span(value),
                diagnostics,
            );
        }
        return expected.cloned().expect("array context");
    }
    if let ScalarExpression::StructLiteral { fields, span } = expression {
        let Some(ScalarType::Struct(id)) = expected else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "struct literal requires a struct context",
                *span,
            ));
            return ScalarType::Error;
        };
        let Some(structure) = modules
            .iter()
            .flat_map(|module| module.structs.iter())
            .find(|structure| structure.id == *id)
        else {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "invalid struct type",
                *span,
            ));
            return ScalarType::Error;
        };
        let mut seen = BTreeSet::new();
        let mut initialized = BTreeSet::new();
        for field in fields {
            if !seen.insert(field.name.clone()) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0002",
                    "duplicate struct literal field",
                    field.name_span,
                ));
                continue;
            }
            let Some(declared) = structure.fields.iter().find(|item| item.name == field.name)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "M0002",
                    "unknown struct field",
                    field.name_span,
                ));
                continue;
            };
            initialized.insert(field.name.clone());
            let actual = expression_type_in_module_expected(
                &field.value,
                Some(&declared.ty),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            expect_module_type(module, &declared.ty, &actual, field.span, diagnostics);
        }
        if initialized.len() != structure.fields.len() {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "struct literal must initialize every field",
                *span,
            ));
        }
        return ScalarType::Struct(id.clone());
    }
    if let ScalarExpression::Integer { value, span } = expression {
        if let Some(
            expected @ (ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128),
        ) = expected
        {
            validate_integer_range_for_type(module, value, *span, expected, diagnostics);
            return expected.clone();
        }
    }
    if let ScalarExpression::Float { .. } = expression {
        if matches!(expected, Some(ScalarType::F32 | ScalarType::F64)) {
            if let ScalarExpression::Float { value, span, .. } = expression {
                if matches!(expected, Some(ScalarType::F32)) && !(*value as f32).is_finite() {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0010",
                        "floating-point literal is outside the f32 range",
                        *span,
                    ));
                    return ScalarType::Error;
                }
            }
            return expected.cloned().expect("float context");
        }
    }
    if let ScalarExpression::Block(block) = expression {
        return block_type_in_module_expected(
            block,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
    }
    if let ScalarExpression::Binary {
        operator,
        left,
        right,
        span,
    } = expression
    {
        let comparison = matches!(
            operator,
            BinaryOperator::Equal
                | BinaryOperator::NotEqual
                | BinaryOperator::Less
                | BinaryOperator::LessEqual
                | BinaryOperator::Greater
                | BinaryOperator::GreaterEqual
        );
        let boolean = matches!(operator, BinaryOperator::And | BinaryOperator::Or);
        let bitwise = matches!(
            operator,
            BinaryOperator::BitAnd
                | BinaryOperator::BitOr
                | BinaryOperator::BitXor
                | BinaryOperator::ShiftLeft
                | BinaryOperator::ShiftRight
        );
        let arithmetic_context = expected.filter(|ty| {
            matches!(
                ty,
                ScalarType::I8
                    | ScalarType::I16
                    | ScalarType::I32
                    | ScalarType::I64
                    | ScalarType::I128
                    | ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
                    | ScalarType::F32
                    | ScalarType::F64
            )
        });
        let bool_context = ScalarType::Bool;
        let (left_type, right_type) = if comparison && is_null_expression(left) {
            let right_type = expression_type_in_module_expected(
                right,
                None,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let left_type = expression_type_in_module_expected(
                left,
                Some(&right_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        } else if comparison && matches!(left.as_ref(), ScalarExpression::Float { .. }) {
            let right_type = expression_type_in_module(
                right,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let left_type = expression_type_in_module_expected(
                left,
                Some(&right_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        } else if comparison && matches!(right.as_ref(), ScalarExpression::Float { .. }) {
            let left_type = expression_type_in_module(
                left,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let right_type = expression_type_in_module_expected(
                right,
                Some(&left_type),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        } else {
            let left_type = expression_type_in_module_expected(
                left,
                if boolean {
                    Some(&bool_context)
                } else {
                    arithmetic_context
                },
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let right_type = expression_type_in_module_expected(
                right,
                if boolean {
                    Some(&bool_context)
                } else if comparison {
                    Some(&left_type)
                } else {
                    arithmetic_context
                },
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            (left_type, right_type)
        };
        if boolean {
            expect_module_type(module, &ScalarType::Bool, &left_type, *span, diagnostics);
            expect_module_type(module, &ScalarType::Bool, &right_type, *span, diagnostics);
            return ScalarType::Bool;
        }
        if is_error_type(&left_type) || is_error_type(&right_type) {
            return ScalarType::Error;
        }
        if comparison {
            expect_module_type(module, &left_type, &right_type, *span, diagnostics);
            return ScalarType::Bool;
        }
        if bitwise {
            expect_module_integer(module, &left_type, *span, diagnostics);
            expect_module_integer(module, &right_type, *span, diagnostics);
            expect_module_type(module, &left_type, &right_type, *span, diagnostics);
            return left_type;
        }
        expect_module_type(
            module,
            arithmetic_context.unwrap_or(&ScalarType::I32),
            &left_type,
            *span,
            diagnostics,
        );
        expect_module_type(module, &left_type, &right_type, *span, diagnostics);
        if let Some(expected) = arithmetic_context {
            return expected.clone();
        }
        return ScalarType::I32;
    }
    expression_type_in_module(
        expression,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
    )
}

fn utf8_type_from_namespace_target(
    module: &ScalarModule,
    modules: &[ScalarModule],
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let structure = module
        .namespace_bindings
        .iter()
        .filter_map(|binding| {
            modules
                .iter()
                .find(|candidate| candidate.source == binding.target)
        })
        .flat_map(|target| target.structs.iter())
        .find(|structure| structure.name == "utf8");
    match structure {
        Some(structure) => ScalarType::Struct(structure.id.clone()),
        None => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "std.utf8 is unavailable",
                span,
            ));
            ScalarType::Error
        }
    }
}

fn utf8_type_from_context(
    module: &ScalarModule,
    modules: &[ScalarModule],
    expected: &ScalarType,
) -> Option<ScalarType> {
    let ScalarType::Struct(expected_id) = expected else {
        return None;
    };
    module
        .namespace_bindings
        .iter()
        .filter(|binding| binding.target == expected_id.source)
        .filter_map(|binding| {
            modules
                .iter()
                .find(|candidate| candidate.source == binding.target)
        })
        .flat_map(|target| target.structs.iter())
        .find(|structure| structure.id == *expected_id && structure.name == "utf8")
        .map(|structure| ScalarType::Struct(structure.id.clone()))
}

fn field_type_in_module(
    receiver: &ScalarType,
    name: &str,
    name_span: ByteSpan,
    span: ByteSpan,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let structure_id = match receiver {
        ScalarType::Struct(id) => id,
        ScalarType::RawPointer(inner) => match inner.as_ref() {
            ScalarType::Struct(id) => id,
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    span,
                ));
                return ScalarType::Error;
            }
        },
        _ => unreachable!("field receiver must be a struct or pointer to a struct"),
    };
    let Some(structure) = modules
        .iter()
        .flat_map(|module| module.structs.iter())
        .find(|structure| structure.id == *structure_id)
    else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            span,
        ));
        return ScalarType::Error;
    };
    let Some(field) = structure.fields.iter().find(|field| field.name == name) else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            name_span,
        ));
        return ScalarType::Error;
    };
    field.ty.clone()
}

fn is_supported_checked_address_place(place: &ScalarPlace) -> bool {
    match place {
        ScalarPlace::Name { .. } => true,
        ScalarPlace::Field { base, .. } => matches!(base.as_ref(), ScalarPlace::Name { .. }),
        ScalarPlace::Dereference { .. } => true,
        ScalarPlace::Index { base, .. } => is_supported_checked_address_place(base),
    }
}

fn is_addressable_checked_reference_type(ty: &ScalarType) -> bool {
    !matches!(
        ty,
        ScalarType::Unit | ScalarType::Callable { .. } | ScalarType::Error
    )
}

fn checked_address_type(
    mutability: ScalarReferenceMutability,
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if !is_supported_checked_address_place(place) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    let inner = if let ScalarPlace::Dereference { pointer, .. } = place {
        match expression_type(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            false,
        ) {
            ScalarType::CheckedReference {
                mutability: parent_mode,
                inner,
            } if mutability == ScalarReferenceMutability::Shared
                || parent_mode == ScalarReferenceMutability::Mutable =>
            {
                *inner
            }
            ScalarType::RawPointer(_) => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "checked address requires a storage name or direct storage field",
                    span,
                ));
                return ScalarType::Error;
            }
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "checked reborrow requires a compatible checked reference",
                    span,
                ));
                return ScalarType::Error;
            }
        }
    } else {
        place_type(place, scope, program, diagnostics)
    };
    if !is_addressable_checked_reference_type(&inner) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    ScalarType::CheckedReference {
        mutability,
        inner: Box::new(inner),
    }
}

fn checked_address_type_in_module(
    mutability: ScalarReferenceMutability,
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if !is_supported_checked_address_place(place) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    let inner = if let ScalarPlace::Dereference { pointer, .. } = place {
        match expression_type_in_module(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            module,
            modules,
            diagnostics,
            false,
        ) {
            ScalarType::CheckedReference {
                mutability: parent_mode,
                inner,
            } if mutability == ScalarReferenceMutability::Shared
                || parent_mode == ScalarReferenceMutability::Mutable =>
            {
                *inner
            }
            ScalarType::RawPointer(_) => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "checked address requires a storage name or direct storage field",
                    span,
                ));
                return ScalarType::Error;
            }
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "checked reborrow requires a compatible checked reference",
                    span,
                ));
                return ScalarType::Error;
            }
        }
    } else {
        place_type_in_module(place, scope, module, modules, diagnostics)
    };
    if !is_addressable_checked_reference_type(&inner) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "checked address requires a storage name or direct storage field",
            span,
        ));
        return ScalarType::Error;
    }
    ScalarType::CheckedReference {
        mutability,
        inner: Box::new(inner),
    }
}

fn writable_place_type(
    place: &ScalarPlace,
    target_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if writable_place_contains_raw_dereference(place, scope, program, diagnostics) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "assignment through raw pointer dereference is not supported",
            target_span,
        ));
        return ScalarType::Error;
    }
    let ty = writable_path_type(place, scope, program, diagnostics);
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "assignment place requires a scalar leaf type",
            target_span,
        ));
        return ScalarType::Error;
    }
    ty
}

fn writable_place_type_in_module(
    place: &ScalarPlace,
    target_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    if writable_place_contains_raw_dereference_in_module(place, scope, module, modules, diagnostics)
    {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "assignment through raw pointer dereference is not supported",
            target_span,
        ));
        return ScalarType::Error;
    }
    let ty = writable_path_type_in_module(place, scope, module, modules, diagnostics);
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. }) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "assignment place requires a scalar leaf type",
            target_span,
        ));
        return ScalarType::Error;
    }
    ty
}

fn writable_place_contains_raw_dereference(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    match place {
        ScalarPlace::Name { .. } => false,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => {
            writable_place_contains_raw_dereference(base, scope, program, diagnostics)
        }
        ScalarPlace::Dereference { pointer, .. } => matches!(
            expression_type(
                pointer,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                true,
            ),
            ScalarType::RawPointer(_)
        ),
    }
}

fn writable_place_contains_raw_dereference_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    match place {
        ScalarPlace::Name { .. } => false,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => {
            writable_place_contains_raw_dereference_in_module(
                base,
                scope,
                module,
                modules,
                diagnostics,
            )
        }
        ScalarPlace::Dereference { pointer, .. } => matches!(
            expression_type_in_module(
                pointer,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                true,
            ),
            ScalarType::RawPointer(_)
        ),
    }
}

fn writable_path_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { .. } => place_type(place, scope, program, diagnostics),
        ScalarPlace::Dereference { pointer, span } => match expression_type(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            false,
        ) {
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable,
                inner,
            } => *inner,
            ScalarType::Error => ScalarType::Error,
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "assignment dereference requires a mutable checked reference",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Field { base, field, span } => {
            let ScalarType::Struct(structure) =
                writable_path_type(base, scope, program, diagnostics)
            else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let Some(field) = program
                .structs
                .iter()
                .find(|candidate| candidate.id == structure && field.structure == structure)
                .and_then(|candidate| candidate.fields.get(field.index))
                .filter(|candidate| candidate.id == *field)
            else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            field.ty.clone()
        }
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = writable_path_type(base, scope, program, diagnostics);
            let index_type = expression_type_expected(
                index,
                &ScalarType::U64,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                false,
            );
            expect_type(
                program,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(diagnostic(
                        program,
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

fn writable_path_type_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { .. } => {
            place_type_in_module(place, scope, module, modules, diagnostics)
        }
        ScalarPlace::Dereference { pointer, span } => match expression_type_in_module(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            module,
            modules,
            diagnostics,
            false,
        ) {
            ScalarType::CheckedReference {
                mutability: ScalarReferenceMutability::Mutable,
                inner,
            } => *inner,
            ScalarType::Error => ScalarType::Error,
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "assignment dereference requires a mutable checked reference",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Field { base, field, span } => {
            let ScalarType::Struct(structure) =
                writable_path_type_in_module(base, scope, module, modules, diagnostics)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            };
            let Some(field) = modules
                .iter()
                .flat_map(|candidate| candidate.structs.iter())
                .find(|candidate| candidate.id == structure && field.structure == structure)
                .and_then(|candidate| candidate.fields.get(field.index))
                .filter(|candidate| candidate.id == *field)
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            };
            field.ty.clone()
        }
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = writable_path_type_in_module(base, scope, module, modules, diagnostics);
            let index_type = expression_type_in_module_expected(
                index,
                Some(&ScalarType::U64),
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                false,
            );
            expect_module_type(
                module,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(module_diagnostic(
                        module,
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

pub(super) fn place_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarPlace::Field {
            base, field, span, ..
        } => {
            let base_type = place_type(base, scope, program, diagnostics);
            let id = match base_type {
                ScalarType::Struct(id) => id,
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => id.clone(),
                    _ => {
                        diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                        return ScalarType::Error;
                    }
                },
                _ => {
                    diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                    return ScalarType::Error;
                }
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let Some(resolved_field) = program
                .structs
                .get(id.index)
                .filter(|structure| field.structure == structure.id)
                .and_then(|structure| structure.fields.get(field.index))
                .filter(|resolved_field| resolved_field.id == *field)
            else {
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            resolved_field.ty.clone()
        }
        ScalarPlace::Dereference { pointer, span } => match expression_type(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            true,
        ) {
            ScalarType::RawPointer(inner) => *inner,
            _ => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "cannot dereference value",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = place_type(base, scope, program, diagnostics);
            let index_type = expression_type_expected(
                index,
                &ScalarType::U64,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                false,
            );
            expect_type(
                program,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(diagnostic(
                        program,
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

pub(super) fn place_type_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    match place {
        ScalarPlace::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(module_diagnostic(module, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarPlace::Dereference { pointer, span } => match expression_type_in_module(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            module,
            modules,
            diagnostics,
            true,
        ) {
            ScalarType::RawPointer(inner) => *inner,
            _ => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "cannot dereference value",
                    *span,
                ));
                ScalarType::Error
            }
        },
        ScalarPlace::Field {
            base, field, span, ..
        } => {
            let base_type = place_type_in_module(base, scope, module, modules, diagnostics);
            let id = match base_type {
                ScalarType::Struct(id) => id,
                ScalarType::RawPointer(inner) => match inner.as_ref() {
                    ScalarType::Struct(id) => id.clone(),
                    _ => {
                        diagnostics.push(module_diagnostic(
                            module,
                            "B0003",
                            "value has no field",
                            *span,
                        ));
                        return ScalarType::Error;
                    }
                },
                _ => {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "value has no field",
                        *span,
                    ));
                    return ScalarType::Error;
                }
            };
            let ScalarFieldReference::Resolved(field) = field else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            };
            if field.structure != id {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "value has no field",
                    *span,
                ));
                return ScalarType::Error;
            }
            let resolved = modules
                .iter()
                .flat_map(|module| module.structs.iter())
                .find(|structure| field.structure == structure.id)
                .and_then(|structure| structure.fields.get(field.index))
                .filter(|resolved_field| resolved_field.id == *field)
                .map(|field| field.ty.clone());
            match resolved {
                Some(ty) => ty,
                None => {
                    diagnostics.push(module_diagnostic(
                        module,
                        "B0003",
                        "value has no field",
                        *span,
                    ));
                    ScalarType::Error
                }
            }
        }
        ScalarPlace::Index {
            base,
            index,
            span,
            index_span,
        } => {
            let base_type = place_type_in_module(base, scope, module, modules, diagnostics);
            let index_type = expression_type_in_module_expected(
                index,
                Some(&ScalarType::U64),
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                false,
            );
            expect_module_type(
                module,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
                    *element
                }
                ScalarType::Error => ScalarType::Error,
                _ => {
                    diagnostics.push(module_diagnostic(
                        module,
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

fn checked_dereference_target_type(
    pointer: &ScalarExpression,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match expression_type(
        pointer,
        scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        program,
        diagnostics,
        unsafe_context,
    ) {
        ScalarType::CheckedReference { inner, .. } => match inner.as_ref() {
            ScalarType::Unit => {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "cannot read unit through checked reference",
                    span,
                ));
                ScalarType::Error
            }
            _ => *inner,
        },
        ScalarType::RawPointer(_) => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "checked dereference requires a checked reference",
                span,
            ));
            ScalarType::Error
        }
        _ => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "cannot dereference value",
                span,
            ));
            ScalarType::Error
        }
    }
}

fn checked_dereference_type(
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match place {
        ScalarPlace::Dereference { pointer, span } => checked_dereference_target_type(
            pointer,
            *span,
            scope,
            program,
            diagnostics,
            unsafe_context,
        ),
        ScalarPlace::Field {
            base,
            field,
            span: field_span,
        } => {
            let ScalarPlace::Dereference { pointer, span } = base.as_ref() else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "checked dereference requires a direct struct field",
                    span,
                ));
                return ScalarType::Error;
            };
            let base_type = checked_dereference_target_type(
                pointer,
                *span,
                scope,
                program,
                diagnostics,
                unsafe_context,
            );
            let ScalarFieldReference::Resolved(field) = field else {
                let ScalarFieldReference::Unresolved { span, .. } = field else {
                    unreachable!("field reference")
                };
                diagnostics.push(diagnostic(program, "B0003", "value has no field", *span));
                return ScalarType::Error;
            };
            let ScalarType::Struct(structure) = base_type else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "value has no field",
                    *field_span,
                ));
                return ScalarType::Error;
            };
            let candidate = program
                .structs
                .iter()
                .find(|candidate| candidate.id == structure)
                .and_then(|candidate| candidate.fields.get(field.index))
                .filter(|candidate| candidate.id == *field)
                .cloned();
            let Some(candidate) = candidate else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "value has no field",
                    *field_span,
                ));
                return ScalarType::Error;
            };
            if candidate.ty == ScalarType::Unit {
                let field_name_span =
                    ByteSpan::new(field_span.end - candidate.name.len() as u32, field_span.end);
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "cannot read unit through checked reference",
                    field_name_span,
                ));
                ScalarType::Error
            } else {
                candidate.ty
            }
        }
        ScalarPlace::Name { .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            ScalarType::Error
        }
        ScalarPlace::Index { .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            ScalarType::Error
        }
    }
}

fn checked_dereference_type_in_module(
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let (pointer, dereference_span, field) = match place {
        ScalarPlace::Dereference { pointer, span } => (pointer, *span, None),
        ScalarPlace::Field {
            base,
            field,
            span: field_span,
        } => {
            let ScalarPlace::Dereference {
                pointer,
                span: dereference_span,
            } = base.as_ref()
            else {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "checked dereference requires a direct struct field",
                    *field_span,
                ));
                return ScalarType::Error;
            };
            (pointer, *dereference_span, Some((field, *field_span)))
        }
        ScalarPlace::Name { .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            return ScalarType::Error;
        }
        ScalarPlace::Index { .. } => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "checked dereference requires a dereference place",
                span,
            ));
            return ScalarType::Error;
        }
    };
    let inner = match expression_type_in_module(
        pointer,
        scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        module,
        modules,
        diagnostics,
        unsafe_context,
    ) {
        ScalarType::CheckedReference { inner, .. } => match inner.as_ref() {
            ScalarType::Unit => {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    "cannot read unit through checked reference",
                    dereference_span,
                ));
                return ScalarType::Error;
            }
            _ => *inner,
        },
        ScalarType::RawPointer(_) => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "checked dereference requires a checked reference",
                dereference_span,
            ));
            return ScalarType::Error;
        }
        _ => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "cannot dereference value",
                dereference_span,
            ));
            return ScalarType::Error;
        }
    };
    let (field, field_span) = match field {
        None => return inner,
        Some((ScalarFieldReference::Resolved(field), field_span)) => (field, field_span),
        Some((ScalarFieldReference::Unresolved { span, .. }, _)) => {
            diagnostics.push(module_diagnostic(
                module,
                "B0003",
                "value has no field",
                *span,
            ));
            return ScalarType::Error;
        }
    };
    let ScalarType::Struct(structure) = inner else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            field_span,
        ));
        return ScalarType::Error;
    };
    let candidate = modules
        .iter()
        .flat_map(|candidate| candidate.structs.iter())
        .find(|candidate| candidate.id == structure)
        .and_then(|candidate| candidate.fields.get(field.index))
        .filter(|candidate| candidate.id == *field)
        .cloned();
    let Some(candidate) = candidate else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            field_span,
        ));
        return ScalarType::Error;
    };
    if candidate.ty == ScalarType::Unit {
        let field_name_span =
            ByteSpan::new(field_span.end - candidate.name.len() as u32, field_span.end);
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "cannot read unit through checked reference",
            field_name_span,
        ));
        ScalarType::Error
    } else {
        candidate.ty
    }
}

pub(super) fn expect_module_type(
    module: &ScalarModule,
    expected: &ScalarType,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(expected) && !is_error_type(actual) && !scalar_type_equal(expected, actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
}

pub(super) fn is_integer_type(ty: &ScalarType) -> bool {
    matches!(
        ty,
        ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128
    )
}

fn is_float_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::F32 | ScalarType::F64)
}

fn integer_width(ty: &ScalarType) -> Option<u32> {
    match ty {
        ScalarType::I8 | ScalarType::U8 => Some(8),
        ScalarType::I16 | ScalarType::U16 => Some(16),
        ScalarType::I32 | ScalarType::U32 => Some(32),
        ScalarType::I64 | ScalarType::U64 => Some(64),
        ScalarType::I128 | ScalarType::U128 => Some(128),
        _ => None,
    }
}

fn expect_module_integer(
    module: &ScalarModule,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(actual) && !is_integer_type(actual) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "integer operation requires integer operands",
            span,
        ));
    }
}

pub(super) fn module_diagnostic(
    module: &ScalarModule,
    code: &str,
    message: &str,
    span: ByteSpan,
) -> super::Diagnostic {
    super::Diagnostic {
        code: code.to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(module.source.clone(), span),
            message: message.to_owned(),
        }],
        notes: Vec::new(),
    }
}

fn fold_name(name: &str) -> String {
    name.bytes()
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

pub(super) fn declare_module_name(
    module: &ScalarModule,
    name: &str,
    span: ByteSpan,
    names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    if !names.insert(name.to_owned()) {
        diagnostics.push(module_diagnostic(
            module,
            "B0002",
            "duplicate declaration",
            span,
        ));
        return false;
    }
    let folded = fold_name(name);
    if let Some((first_name, first_span)) = folded_names.get(&folded) {
        if first_name != name {
            names.remove(name);
            diagnostics.push(module_collision_diagnostic(module, span, *first_span));
            return false;
        }
    } else {
        folded_names.insert(folded, (name.to_owned(), span));
    }
    true
}

pub(super) fn validate_identifier_style(
    module: &ScalarModule,
    name: &str,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_declared_identifier_style(name) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE",
            span,
        ));
    }
}

fn validate_program_identifier_style(
    program: &ScalarProgram,
    name: &str,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_declared_identifier_style(name) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE",
            span,
        ));
    }
}

fn is_declared_identifier_style(name: &str) -> bool {
    if name == "_" {
        return true;
    }
    let bytes = name.as_bytes();
    let snake_case = bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_');
    let pascal_case =
        bytes[0].is_ascii_uppercase() && bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric());
    let screaming_snake_case = bytes.iter().any(|byte| byte.is_ascii_uppercase())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_');
    snake_case || pascal_case || screaming_snake_case
}

fn module_collision_diagnostic(
    module: &ScalarModule,
    span: ByteSpan,
    first_span: ByteSpan,
) -> super::Diagnostic {
    let mut diagnostic = module_diagnostic(
        module,
        "B0008",
        "declaration collides with an existing name under ASCII case folding",
        span,
    );
    diagnostic.labels.push(super::DiagnosticLabel {
        kind: super::DiagnosticLabelKind::Secondary,
        span: SourceSpan::new(module.source.clone(), first_span),
        message: "first conflicting declaration".to_owned(),
    });
    diagnostic
}

fn unknown_member_diagnostic(
    importer: &ScalarModule,
    member_span: ByteSpan,
    target: &SourceIdentity,
) -> super::Diagnostic {
    super::Diagnostic {
        code: "M0002".to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: "unknown module member".to_owned(),
        labels: vec![
            super::DiagnosticLabel {
                kind: super::DiagnosticLabelKind::Primary,
                span: SourceSpan::new(importer.source.clone(), member_span),
                message: "unknown module member".to_owned(),
            },
            super::DiagnosticLabel {
                kind: super::DiagnosticLabelKind::Secondary,
                span: SourceSpan::new(target.clone(), ByteSpan::new(0, 0)),
                message: "module resolved here".to_owned(),
            },
        ],
        notes: Vec::new(),
    }
}

pub(super) fn is_module_private_name(name: &str) -> bool {
    name.starts_with('_')
}

fn validate_unit_if_positions(program: &ScalarProgram, diagnostics: &mut Vec<super::Diagnostic>) {
    validate_unit_if_positions_in_items(&program.items, &program.source, diagnostics);
}

pub(super) fn validate_unit_if_positions_in_module(
    module: &ScalarModule,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    validate_unit_if_positions_in_items(&module.items, &module.source, diagnostics);
}

fn validate_unit_if_positions_in_items(
    items: &[ScalarItem],
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for item in items {
        match item {
            ScalarItem::Binding(binding) => {
                validate_unit_if_position(&binding.value, false, source, diagnostics)
            }
            ScalarItem::Function(function) => {
                validate_unit_if_positions_in_block(&function.body, source, diagnostics)
            }
            ScalarItem::Executable(item) => {
                validate_unit_if_positions_in_item(item, true, source, diagnostics)
            }
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
        }
    }
}

fn validate_unit_if_positions_in_block(
    block: &ScalarBlock,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (index, item) in block.items.iter().enumerate() {
        let statement = index + 1 != block.items.len() || block.terminated_items[index];
        validate_unit_if_positions_in_item(item, statement, source, diagnostics);
    }
}

fn validate_unit_if_positions_in_item(
    item: &ScalarBlockItem,
    statement: bool,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            validate_unit_if_position(&binding.value, false, source, diagnostics)
        }
        ScalarBlockItem::Expression(expression) => {
            validate_unit_if_position(expression, statement, source, diagnostics)
        }
        ScalarBlockItem::Assignment(assignment) => {
            for value in &assignment.values {
                validate_unit_if_position(value, false, source, diagnostics);
            }
        }
        ScalarBlockItem::While(while_expression) => {
            validate_unit_if_position(&while_expression.condition, false, source, diagnostics);
            validate_unit_if_positions_in_block(&while_expression.body, source, diagnostics);
        }
    }
}

fn validate_unit_if_position(
    expression: &ScalarExpression,
    statement: bool,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match expression {
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            span,
        } => {
            if !statement {
                diagnostics.push(super::Diagnostic {
                    code: "B0003".to_owned(),
                    severity: super::DiagnosticSeverity::Error,
                    message: "if without else is only valid as a unit statement".to_owned(),
                    labels: vec![super::DiagnosticLabel {
                        kind: super::DiagnosticLabelKind::Primary,
                        span: SourceSpan::new(source.clone(), *span),
                        message: "if without else is only valid as a unit statement".to_owned(),
                    }],
                    notes: Vec::new(),
                });
            }
            validate_unit_if_position(condition, false, source, diagnostics);
            validate_unit_if_positions_in_block(then_branch, source, diagnostics);
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            validate_unit_if_position(condition, false, source, diagnostics);
            validate_unit_if_positions_in_block(then_branch, source, diagnostics);
            validate_unit_if_positions_in_block(else_branch, source, diagnostics);
        }
        ScalarExpression::Binary { left, right, .. } => {
            validate_unit_if_position(left, false, source, diagnostics);
            validate_unit_if_position(right, false, source, diagnostics);
        }
        ScalarExpression::Unary { operand, .. } => {
            validate_unit_if_position(operand, false, source, diagnostics);
        }
        ScalarExpression::Call { arguments, .. } => {
            for argument in arguments {
                validate_unit_if_position(argument, false, source, diagnostics);
            }
        }
        ScalarExpression::RawAddress { place, .. }
        | ScalarExpression::CheckedAddress { place, .. }
        | ScalarExpression::Dereference { place, .. } => {
            validate_unit_if_position_in_place(place, source, diagnostics)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            validate_unit_if_position_in_place(place, source, diagnostics)
        }
        ScalarExpression::StructLiteral { fields, .. } => {
            for field in fields {
                validate_unit_if_position(&field.value, false, source, diagnostics);
            }
        }
        ScalarExpression::ArrayLiteral { elements, .. } => {
            for element in elements {
                validate_unit_if_position(element, false, source, diagnostics);
            }
        }
        ScalarExpression::Block(block) => {
            validate_unit_if_positions_in_block(block, source, diagnostics)
        }
        ScalarExpression::Name { .. }
        | ScalarExpression::Member { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::InvalidInteger { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::InvalidFloat { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. }
        | ScalarExpression::Utf8 { .. } => {}
    }
}

fn validate_unit_if_position_in_place(
    place: &ScalarPlace,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match place {
        ScalarPlace::Dereference { pointer, .. } => {
            validate_unit_if_position(pointer, false, source, diagnostics)
        }
        ScalarPlace::Field { base, .. } => {
            validate_unit_if_position_in_place(base, source, diagnostics)
        }
        ScalarPlace::Index { base, index, .. } => {
            validate_unit_if_position_in_place(base, source, diagnostics);
            validate_unit_if_position(index, false, source, diagnostics);
        }
        ScalarPlace::Name { .. } => {}
    }
}

pub(super) fn operator(value: &str) -> BinaryOperator {
    match value {
        "+" => BinaryOperator::Add,
        "-" => BinaryOperator::Subtract,
        "*" => BinaryOperator::Multiply,
        "/" => BinaryOperator::Divide,
        "%" => BinaryOperator::Remainder,
        "==" => BinaryOperator::Equal,
        "!=" => BinaryOperator::NotEqual,
        "<" => BinaryOperator::Less,
        "<=" => BinaryOperator::LessEqual,
        ">" => BinaryOperator::Greater,
        ">=" => BinaryOperator::GreaterEqual,
        "&&" => BinaryOperator::And,
        "||" => BinaryOperator::Or,
        "&" => BinaryOperator::BitAnd,
        "|" => BinaryOperator::BitOr,
        "^" => BinaryOperator::BitXor,
        "<<" => BinaryOperator::ShiftLeft,
        ">>" => BinaryOperator::ShiftRight,
        _ => panic!("operator grammar"),
    }
}

pub(super) fn validate(program: &ScalarProgram) -> Vec<super::Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_unit_if_positions(program, &mut diagnostics);
    let mut declarations = BTreeMap::new();
    let mut declaration_names = BTreeSet::new();
    let mut folded_declarations = BTreeMap::new();
    let mut struct_names = BTreeSet::new();
    for structure in &program.structs {
        if !declare_program_name(
            program,
            &structure.name,
            structure.span,
            &mut struct_names,
            &mut folded_declarations,
            &mut diagnostics,
        ) {
            continue;
        }
        let mut field_names = BTreeSet::new();
        for field in &structure.fields {
            if field_names.insert(field.name.clone()) {
                validate_type(program, &field.ty, field.name_span, &mut diagnostics);
            } else {
                diagnostics.push(diagnostic(
                    program,
                    "B0002",
                    "duplicate declaration",
                    field.name_span,
                ));
            }
        }
    }
    for item in &program.items {
        let (name, span, ty) = match item {
            ScalarItem::Namespace(namespace) => {
                validate_program_identifier_style(
                    program,
                    &namespace.binding,
                    namespace.binding_span,
                    &mut diagnostics,
                );
                (&namespace.binding, namespace.span, None)
            }
            ScalarItem::Extern(extern_decl) => {
                validate_program_identifier_style(
                    program,
                    &extern_decl.binding,
                    extern_decl.binding_span,
                    &mut diagnostics,
                );
                (&extern_decl.binding, extern_decl.span, None)
            }
            ScalarItem::Binding(binding) => {
                for receiver in &binding.receivers {
                    validate_program_identifier_style(
                        program,
                        &receiver.name,
                        receiver.name_span,
                        &mut diagnostics,
                    );
                }
                (&binding.name, binding.span, Some(&binding.declared_type))
            }
            ScalarItem::Function(function) => {
                validate_program_identifier_style(
                    program,
                    &function.name,
                    function.name_span,
                    &mut diagnostics,
                );
                (&function.name, function.span, Some(&function.signature))
            }
            ScalarItem::Executable(_) => continue,
        };
        if declare_program_name(
            program,
            name,
            span,
            &mut declaration_names,
            &mut folded_declarations,
            &mut diagnostics,
        ) {
            if let Some(ty) = ty {
                declarations.insert(name.clone(), ty.clone());
                if let ScalarItem::Binding(binding) = item {
                    for receiver in &binding.receivers {
                        declarations.insert(receiver.name.clone(), receiver.ty.clone());
                    }
                }
                if let ScalarItem::Function(function) = item {
                    if !function.overload_arms.is_empty() {
                        for arm in &function.overload_arms {
                            if arm.generic_parameters.is_empty() {
                                validate_type(program, &arm.signature, arm.span, &mut diagnostics);
                            } else {
                                validate_generic_type(
                                    program,
                                    &arm.signature,
                                    arm.span,
                                    &arm.generic_parameters,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else if function.generic_parameters.is_empty() {
                        validate_type(program, ty, span, &mut diagnostics);
                    } else {
                        validate_generic_type(
                            program,
                            ty,
                            span,
                            &function.generic_parameters,
                            &mut diagnostics,
                        );
                    }
                } else {
                    validate_type(program, ty, span, &mut diagnostics);
                }
            }
        }
    }
    for item in &program.items {
        if let ScalarItem::Extern(extern_decl) = item {
            let module =
                ScalarModule::new(program.source.clone(), program.items.clone(), Vec::new());
            validate_extern(&module, extern_decl, &mut diagnostics);
        }
    }
    let mut scope = declarations.clone();
    for item in &program.items {
        match item {
            ScalarItem::Namespace(_) | ScalarItem::Extern(_) => {}
            ScalarItem::Binding(binding) => {
                if binding.is_allocation {
                    for receiver in &binding.receivers {
                        let Some(length) = &receiver.allocation_length else {
                            continue;
                        };
                        let actual = expression_type_expected(
                            length,
                            &ScalarType::U64,
                            &declarations,
                            &BTreeSet::new(),
                            &BTreeMap::new(),
                            program,
                            &mut diagnostics,
                            false,
                        );
                        expect_type(
                            program,
                            &ScalarType::U64,
                            &actual,
                            receiver.span,
                            &mut diagnostics,
                        );
                    }
                    continue;
                }
                let actual = expression_type_expected(
                    &binding.value,
                    &binding.declared_type,
                    &declarations,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    program,
                    &mut diagnostics,
                    false,
                );
                validate_output_receivers(
                    &binding.value,
                    &binding.receivers,
                    binding.span,
                    &declarations,
                    program,
                    &mut diagnostics,
                );
                expect_type(
                    program,
                    &binding.declared_type,
                    &actual,
                    binding.span,
                    &mut diagnostics,
                );
            }
            ScalarItem::Function(function) => {
                if !function.overload_arms.is_empty() {
                    continue;
                }
                let ScalarType::Callable {
                    outputs,
                    parameters,
                } = &function.signature
                else {
                    continue;
                };
                let mut scope = declarations.clone();
                let mut visible_names = declaration_names.clone();
                let mut folded_names = folded_declarations.clone();
                for (index, name) in function.parameters.iter().enumerate() {
                    validate_program_identifier_style(
                        program,
                        name,
                        function.parameter_spans[index],
                        &mut diagnostics,
                    );
                    if index < parameters.len()
                        && declare_program_name(
                            program,
                            name,
                            function.span,
                            &mut visible_names,
                            &mut folded_names,
                            &mut diagnostics,
                        )
                    {
                        scope.insert(name.clone(), parameters[index].clone());
                    }
                }
                if outputs.outputs.len() > 1 && !function.body.final_output_values.is_empty() {
                    let final_start =
                        function.body.items.len() - function.body.final_output_values.len();
                    for item in &function.body.items[..final_start] {
                        let _ = match item {
                            ScalarBlockItem::Expression(
                                expression @ (ScalarExpression::If { .. }
                                | ScalarExpression::UnitIf { .. }),
                            ) => expression_type_expected(
                                expression,
                                &ScalarType::Unit,
                                &scope,
                                &visible_names,
                                &folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            ),
                            _ => block_item_type(
                                item,
                                &mut scope,
                                &mut visible_names,
                                &mut folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            ),
                        };
                    }
                    if function.body.final_output_values.len() == 1
                        && matches!(
                            function.body.final_output_values[0].value,
                            ScalarExpression::If { .. }
                        )
                    {
                        validate_conditional_output_arity(
                            &function.body,
                            outputs.outputs.len(),
                            program,
                            &mut diagnostics,
                        );
                    }
                    if function.body.final_output_values.len() == 1
                        && matches!(
                            &function.body.final_output_values[0].value,
                            ScalarExpression::Call { .. }
                        )
                    {
                        let value = &function.body.final_output_values[0];
                        if let Some(actual_outputs) =
                            call_output_sequence(&value.value, &scope, program)
                        {
                            expression_type_expected(
                                &value.value,
                                &actual_outputs
                                    .outputs
                                    .first()
                                    .map(|output| &output.ty)
                                    .expect("multi-output final call has an output"),
                                &scope,
                                &visible_names,
                                &folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            );
                            if actual_outputs.outputs.len() != outputs.outputs.len() {
                                diagnostics.push(diagnostic(
                                    program,
                                    "B0004",
                                    "function output arity does not match its final output list",
                                    value.span,
                                ));
                            }
                            for (actual, expected) in
                                actual_outputs.outputs.iter().zip(&outputs.outputs)
                            {
                                expect_type(
                                    program,
                                    &expected.ty,
                                    &actual.ty,
                                    value.span,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else {
                        if function.body.final_output_values.len() > 1
                            && function.body.final_output_values.len() != outputs.outputs.len()
                        {
                            diagnostics.push(diagnostic(
                                program,
                                "B0004",
                                "function output arity does not match its final output list",
                                function.body.span,
                            ));
                        }
                        for (value, output) in function
                            .body
                            .final_output_values
                            .iter()
                            .zip(&outputs.outputs)
                        {
                            let actual = expression_type_expected(
                                &value.value,
                                &output.ty,
                                &scope,
                                &visible_names,
                                &folded_names,
                                program,
                                &mut diagnostics,
                                false,
                            );
                            expect_type(program, &output.ty, &actual, value.span, &mut diagnostics);
                        }
                    }
                } else if let Some(output) = outputs.outputs.first() {
                    let actual = block_type_expected(
                        &function.body,
                        &output.ty,
                        &scope,
                        &visible_names,
                        &folded_names,
                        program,
                        &mut diagnostics,
                        false,
                    );
                    expect_type(
                        program,
                        &output.ty,
                        &actual,
                        function.body.span,
                        &mut diagnostics,
                    );
                } else {
                    let actual = block_type(
                        &function.body,
                        &scope,
                        &visible_names,
                        &folded_names,
                        program,
                        &mut diagnostics,
                        false,
                    );
                    expect_type(
                        program,
                        &ScalarType::Unit,
                        &actual,
                        function.body.span,
                        &mut diagnostics,
                    );
                }
                if function.parameters.len() != parameters.len() {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "function parameter arity does not match its type",
                        function.span,
                    ));
                }
            }
            ScalarItem::Executable(executable) => {
                let mut visible_names = declaration_names.clone();
                let mut folded_names = folded_declarations.clone();
                let _ = block_item_type(
                    executable,
                    &mut scope,
                    &mut visible_names,
                    &mut folded_names,
                    program,
                    &mut diagnostics,
                    false,
                );
            }
        }
    }
    for span in unused_binding_spans(&program.items) {
        diagnostics.push(diagnostic(
            program,
            "B0009",
            "local binding or parameter has no required static use",
            span,
        ));
    }
    diagnostics
}

struct StaticUseAnalyzer {
    visible: BTreeMap<String, usize>,
    declarations: Vec<ByteSpan>,
    uses: BTreeSet<usize>,
}

impl StaticUseAnalyzer {
    fn new(function: &ScalarFunction) -> Self {
        let mut analyzer = Self {
            visible: BTreeMap::new(),
            declarations: Vec::new(),
            uses: BTreeSet::new(),
        };
        for (index, name) in function.parameters.iter().enumerate() {
            if let Some(span) = function.parameter_spans.get(index) {
                let id = analyzer.declarations.len();
                analyzer.visible.insert(name.clone(), id);
                analyzer.declarations.push(*span);
            }
        }
        analyzer
    }

    fn block(&mut self, block: &ScalarBlock, visible: &BTreeMap<String, usize>) {
        let mut visible = visible.clone();
        for item in &block.items {
            self.item(item, &mut visible);
        }
    }

    fn item(&mut self, item: &ScalarBlockItem, visible: &mut BTreeMap<String, usize>) {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                if binding.is_allocation {
                    for receiver in &binding.receivers {
                        if let Some(length) = &receiver.allocation_length {
                            self.expression(length, visible);
                        }
                    }
                } else {
                    self.expression(&binding.value, visible);
                }
                let id = self.declarations.len();
                visible.insert(binding.name.clone(), id);
                self.declarations.push(binding.span);
            }
            ScalarBlockItem::Expression(expression) => self.expression(expression, visible),
            ScalarBlockItem::Assignment(assignment) => {
                self.expression(&assignment.value, visible);
                for value in &assignment.values {
                    self.expression(value, visible);
                }
                if let Some(receiver) = &assignment.receiver {
                    self.use_name(receiver, visible);
                } else {
                    self.use_name(&assignment.target, visible);
                }
            }
            ScalarBlockItem::While(while_expression) => {
                self.expression(&while_expression.condition, visible);
                self.block(&while_expression.body, visible);
            }
        }
    }

    fn expression(&mut self, expression: &ScalarExpression, visible: &BTreeMap<String, usize>) {
        match expression {
            ScalarExpression::Name { name, .. } => self.use_name(name, visible),
            ScalarExpression::Member { receiver, .. } => self.use_name(receiver, visible),
            ScalarExpression::Unary { operand, .. } => self.expression(operand, visible),
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left, visible);
                self.expression(right, visible);
            }
            ScalarExpression::Call {
                receiver,
                name,
                arguments,
                ..
            } => {
                if let Some(receiver) = receiver {
                    self.use_name(receiver, visible);
                } else {
                    self.use_name(name, visible);
                }
                for argument in arguments {
                    self.expression(argument, visible);
                }
            }
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition, visible);
                self.block(then_branch, visible);
                self.block(else_branch, visible);
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition, visible);
                self.block(then_branch, visible);
            }
            ScalarExpression::Block(block) => self.block(block, visible),
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. } => self.place(place, visible),
            ScalarExpression::IndexedRead { place, .. } => self.place(place, visible),
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value, visible);
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element, visible);
                }
            }
            ScalarExpression::Integer { .. }
            | ScalarExpression::InvalidInteger { .. }
            | ScalarExpression::Float { .. }
            | ScalarExpression::InvalidFloat { .. }
            | ScalarExpression::Boolean { .. }
            | ScalarExpression::Char { .. }
            | ScalarExpression::Utf8 { .. } => {}
        }
    }

    fn place(&mut self, place: &ScalarPlace, visible: &BTreeMap<String, usize>) {
        match place {
            ScalarPlace::Name { name, .. } => self.use_name(name, visible),
            ScalarPlace::Field { base, .. } => self.place(base, visible),
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer, visible),
            ScalarPlace::Index { base, index, .. } => {
                self.place(base, visible);
                self.expression(index, visible);
            }
        }
    }

    fn use_name(&mut self, name: &str, visible: &BTreeMap<String, usize>) {
        if let Some(id) = visible.get(name) {
            self.uses.insert(*id);
        }
    }

    fn unused(self) -> Vec<ByteSpan> {
        self.declarations
            .iter()
            .enumerate()
            .filter_map(|(id, span)| {
                let used = self.uses.contains(&id)
                    || self
                        .declarations
                        .iter()
                        .enumerate()
                        .any(|(other_id, other)| other == span && self.uses.contains(&other_id));
                (!used).then_some(*span)
            })
            .collect()
    }
}

pub(super) fn unused_binding_spans(items: &[ScalarItem]) -> Vec<ByteSpan> {
    items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) => {
                let mut analyzer = StaticUseAnalyzer::new(function);
                let visible = analyzer.visible.clone();
                analyzer.block(&function.body, &visible);
                Some(analyzer.unused())
            }
            ScalarItem::Namespace(_)
            | ScalarItem::Extern(_)
            | ScalarItem::Binding(_)
            | ScalarItem::Executable(_) => None,
        })
        .flatten()
        .collect()
}

fn validate_type(
    program: &ScalarProgram,
    ty: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { name, span }
            if !program.structs.iter().any(|item| item.name == *name) =>
        {
            diagnostics.push(diagnostic(program, "B0003", "unknown scalar type", *span))
        }
        ScalarType::Named { .. } | ScalarType::Qualified { .. } => {}
        ScalarType::Struct(_) | ScalarType::Enum(_) => {}
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_type(program, &output.ty, output.span, diagnostics);
            }
            for parameter in parameters {
                validate_type(program, parameter, span, diagnostics);
            }
        }
        ScalarType::Unit
        | ScalarType::Bool
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::U128
        | ScalarType::F32
        | ScalarType::F64
        | ScalarType::Char
        | ScalarType::ArtifactId => {}
        ScalarType::RawPointer(inner) if matches!(inner.as_ref(), ScalarType::U8) => {}
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            validate_type(program, inner, span, diagnostics)
        }
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
            validate_type(program, element, span, diagnostics)
        }
        ScalarType::Error => {}
    }
}

fn validate_generic_type(
    program: &ScalarProgram,
    ty: &ScalarType,
    span: ByteSpan,
    generic_parameters: &[ScalarGenericParameter],
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { name, .. }
            if generic_parameters
                .iter()
                .any(|parameter| parameter.name == *name) => {}
        ScalarType::RawPointer(inner) | ScalarType::CheckedReference { inner, .. } => {
            validate_generic_type(program, inner, span, generic_parameters, diagnostics)
        }
        ScalarType::Array { element, .. } => {
            validate_generic_type(program, element, span, generic_parameters, diagnostics)
        }
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_generic_type(
                    program,
                    &output.ty,
                    output.span,
                    generic_parameters,
                    diagnostics,
                );
            }
            for parameter in parameters {
                validate_generic_type(program, parameter, span, generic_parameters, diagnostics);
            }
        }
        _ => validate_type(program, ty, span, diagnostics),
    }
}

pub(super) fn block_type(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = block_item_type(
            item,
            &mut scope,
            &mut visible_names,
            &mut folded_names,
            program,
            diagnostics,
            unsafe_context,
        );
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

fn block_type_expected(
    block: &ScalarBlock,
    expected: &ScalarType,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut scope = scope.clone();
    let mut visible_names = visible_names.clone();
    let mut folded_names = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = match item {
            ScalarBlockItem::Expression(expression)
                if index + 1 == block.items.len() && !block.terminated_items[index] =>
            {
                expression_type_expected(
                    expression,
                    expected,
                    &scope,
                    &visible_names,
                    &folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                )
            }
            _ => block_item_type(
                item,
                &mut scope,
                &mut visible_names,
                &mut folded_names,
                program,
                diagnostics,
                unsafe_context,
            ),
        };
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

fn block_item_type(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            for receiver in &binding.receivers {
                validate_program_identifier_style(
                    program,
                    &receiver.name,
                    receiver.name_span,
                    diagnostics,
                );
            }
            if binding.is_allocation {
                for receiver in &binding.receivers {
                    let Some(length) = &receiver.allocation_length else {
                        continue;
                    };
                    let actual = expression_type_expected(
                        length,
                        &ScalarType::U64,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                    expect_type(
                        program,
                        &ScalarType::U64,
                        &actual,
                        receiver.span,
                        diagnostics,
                    );
                }
                if declare_program_name(
                    program,
                    &binding.name,
                    binding.span,
                    visible_names,
                    folded_names,
                    diagnostics,
                ) {
                    for receiver in &binding.receivers {
                        scope.insert(receiver.name.clone(), receiver.ty.clone());
                    }
                }
                return ScalarType::Unit;
            }
            let actual = expression_type_expected(
                &binding.value,
                &binding.declared_type,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            validate_output_receivers(
                &binding.value,
                &binding.receivers,
                binding.span,
                scope,
                program,
                diagnostics,
            );
            expect_type(
                program,
                &binding.declared_type,
                &actual,
                binding.span,
                diagnostics,
            );
            if declare_program_name(
                program,
                &binding.name,
                binding.span,
                visible_names,
                folded_names,
                diagnostics,
            ) {
                scope.insert(binding.name.clone(), binding.declared_type.clone());
                for receiver in &binding.receivers {
                    scope.insert(receiver.name.clone(), receiver.ty.clone());
                }
            }
            ScalarType::Unit
        }
        ScalarBlockItem::Expression(expression) => expression_type(
            expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        ScalarBlockItem::Assignment(assignment) => assignment_type(
            assignment,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        ScalarBlockItem::While(while_expression) => while_type(
            while_expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
    }
}

fn assignment_type(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let mut inferred_names = visible_names.clone();
    let mut inferred_folded_names = folded_names.clone();
    let expected = assignment
        .targets
        .iter()
        .map(|target| assignment_target_type(target, scope, program, diagnostics))
        .collect::<Vec<_>>();
    validate_assignment_target_distinctness(assignment, program, diagnostics);
    let outputs = assignment
        .values
        .iter()
        .enumerate()
        .map(|(position, value)| {
            let actual = match expected.get(position).and_then(Option::as_ref) {
                Some(expected) => expression_type_expected(
                    value,
                    expected,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                ),
                None => expression_type(
                    value,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                ),
            };
            match call_output_sequence(value, scope, program) {
                Some(outputs) => Some(outputs.outputs),
                None if matches!(value, ScalarExpression::Call { .. })
                    && is_error_type(&actual) =>
                {
                    None
                }
                None => Some(vec![ScalarOutput {
                    ty: actual,
                    span: expression_span(value),
                }]),
            }
        })
        .collect::<Option<Vec<_>>>()
        .map(|outputs| ScalarOutputSequence {
            outputs: outputs.into_iter().flatten().collect(),
            span: assignment.span,
        });
    if let Some(outputs) = outputs {
        record_assignment_outputs(
            &program.resolved_assignment_outputs,
            &program.source,
            &program.items,
            assignment,
            &outputs,
        );
        if assignment.targets.len() > outputs.outputs.len() {
            diagnostics.push(diagnostic(
                program,
                "B0004",
                "call has fewer outputs than assignment targets",
                assignment.span,
            ));
        }
        for ((index, (target, expected)), output) in assignment
            .targets
            .iter()
            .zip(&expected)
            .enumerate()
            .zip(&outputs.outputs)
        {
            if let Some(expected) = expected {
                expect_type(
                    program,
                    expected,
                    &output.ty,
                    if index == 0 {
                        assignment.span
                    } else {
                        target.target_span
                    },
                    diagnostics,
                );
            } else if let ScalarPlace::Name { name, span } = &target.place {
                validate_program_identifier_style(program, name, *span, diagnostics);
                if declare_program_name(
                    program,
                    name,
                    *span,
                    &mut inferred_names,
                    &mut inferred_folded_names,
                    diagnostics,
                ) {
                    scope.insert(name.clone(), output.ty.clone());
                }
            }
        }
    }
    ScalarType::Unit
}

fn assignment_target_type(
    target: &ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> Option<ScalarType> {
    match &target.place {
        ScalarPlace::Name { name, span } => {
            let expected = scope.get(name).cloned();
            if expected.is_some() && is_const_binding_name(name) {
                diagnostics.push(diagnostic(
                    program,
                    "B0007",
                    "assignment targets a SCREAMING_SNAKE_CASE const binding",
                    *span,
                ));
            }
            expected
        }
        place => Some(writable_place_type(
            place,
            target.span,
            scope,
            program,
            diagnostics,
        )),
    }
}

fn validate_assignment_target_distinctness(
    assignment: &ScalarAssignment,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (index, target) in assignment.targets.iter().enumerate() {
        for previous in &assignment.targets[..index] {
            if scalar_place_identity(&target.place) == scalar_place_identity(&previous.place) {
                diagnostics.push(diagnostic(
                    program,
                    "B0002",
                    "duplicate assignment target",
                    target.target_span,
                ));
            } else if place_contains_dynamic_index(&target.place)
                || place_contains_dynamic_index(&previous.place)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0002",
                    "assignment targets require a distinctness proof",
                    target.target_span,
                ));
            }
        }
    }
}

fn place_contains_dynamic_index(place: &ScalarPlace) -> bool {
    match place {
        ScalarPlace::Name { .. } | ScalarPlace::Dereference { .. } => false,
        ScalarPlace::Field { base, .. } => place_contains_dynamic_index(base),
        ScalarPlace::Index { base, index, .. } => {
            place_contains_dynamic_index(base)
                || !matches!(index.as_ref(), ScalarExpression::Integer { .. })
        }
    }
}

fn while_type(
    while_expression: &ScalarWhile,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    let condition = expression_type(
        &while_expression.condition,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    if !is_error_type(&condition) && !scalar_type_equal(&condition, &ScalarType::Bool) {
        diagnostics.push(diagnostic(
            program,
            "B0005",
            "while expression requires bool",
            while_expression.span,
        ));
    }
    let _ = block_type(
        &while_expression.body,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
    );
    ScalarType::Unit
}

pub(super) fn expression_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    match expression {
        ScalarExpression::RawAddress { place, span } => {
            if !unsafe_context {
                diagnostics.push(diagnostic(
                    program,
                    "B0012",
                    "raw address requires an unsafe block",
                    *span,
                ));
            }
            ScalarType::RawPointer(Box::new(place_type(place, scope, program, diagnostics)))
        }
        ScalarExpression::CheckedAddress {
            mutability,
            place,
            span,
        } => checked_address_type(*mutability, place, *span, scope, program, diagnostics),
        ScalarExpression::Dereference { place, span } => {
            checked_dereference_type(place, *span, scope, program, diagnostics, unsafe_context)
        }
        ScalarExpression::IndexedRead { place, .. } => {
            place_type(place, scope, program, diagnostics)
        }
        ScalarExpression::StructLiteral { .. } => ScalarType::Error,
        ScalarExpression::ArrayLiteral { span, .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "array literal requires a fixed array context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Name { name, span } if name == "null" => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "null requires a pointer context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarExpression::Member {
            receiver,
            name,
            enum_tag,
            name_span,
            span,
            ..
        } => match program
            .enums
            .iter()
            .find(|enumeration| enumeration.name == *receiver)
        {
            Some(enumeration) => {
                if enum_tag.is_some() {
                    ScalarType::Enum(enumeration.id.clone())
                } else {
                    diagnostics.push(diagnostic(
                        program,
                        "M0002",
                        "unknown enum variant",
                        *name_span,
                    ));
                    ScalarType::Error
                }
            }
            None => field_type(
                scope.get(receiver),
                name,
                *name_span,
                *span,
                program,
                diagnostics,
            ),
        },
        ScalarExpression::Integer { value, span } => {
            validate_integer_range_program(program, value, *span, diagnostics);
            ScalarType::I32
        }
        ScalarExpression::Float { .. } => ScalarType::Error,
        ScalarExpression::InvalidFloat { .. } => ScalarType::Error,
        ScalarExpression::InvalidInteger {
            span: _,
            error_span,
        } => {
            if error_span.is_some() {
                ScalarType::Error
            } else {
                ScalarType::I32
            }
        }
        ScalarExpression::Boolean { .. } => ScalarType::Bool,
        ScalarExpression::Char { .. } => ScalarType::Char,
        ScalarExpression::Utf8 { span, .. } => {
            diagnostics.push(diagnostic(
                program,
                "B0003",
                "string literal requires a std.utf8 context",
                *span,
            ));
            ScalarType::Error
        }
        ScalarExpression::Unary {
            operator,
            operand,
            span,
        } => {
            if matches!(operator, UnaryOperator::Negate) {
                if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                    let value = -value;
                    validate_integer_range_program(program, &value, *span, diagnostics);
                    return ScalarType::I32;
                }
            }
            let operand_type = expression_type(
                operand,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            match operator {
                UnaryOperator::LogicalNot => {
                    expect_type(
                        program,
                        &ScalarType::Bool,
                        &operand_type,
                        *span,
                        diagnostics,
                    );
                    ScalarType::Bool
                }
                UnaryOperator::BitwiseNot => {
                    expect_integer(program, &operand_type, *span, diagnostics);
                    operand_type
                }
                UnaryOperator::Negate => {
                    expect_integer(program, &operand_type, *span, diagnostics);
                    operand_type
                }
            }
        }
        ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } => {
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            if comparison && (is_null_expression(left) || is_null_expression(right)) {
                return expression_type_expected(
                    expression,
                    &ScalarType::Bool,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            let left_type = expression_type(
                left,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let right_type = if comparison {
                expression_type_expected(
                    right,
                    &left_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                )
            } else {
                expression_type(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                )
            };
            if is_error_type(&left_type) || is_error_type(&right_type) {
                return ScalarType::Error;
            }
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            let boolean = matches!(operator, BinaryOperator::And | BinaryOperator::Or);
            let bitwise = matches!(
                operator,
                BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
            );
            if boolean {
                expect_type(program, &ScalarType::Bool, &left_type, *span, diagnostics);
                expect_type(program, &ScalarType::Bool, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if comparison {
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                ScalarType::Bool
            } else if bitwise {
                expect_integer(program, &left_type, *span, diagnostics);
                expect_integer(program, &right_type, *span, diagnostics);
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                left_type
            } else {
                expect_type(program, &ScalarType::I32, &left_type, *span, diagnostics);
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                ScalarType::I32
            }
        }
        ScalarExpression::Call {
            receiver,
            name,
            name_span,
            arguments,
            span,
            type_arguments,
            ..
        } => {
            if receiver.as_deref() == Some("core") && name == "this_artifact_id" {
                if !arguments.is_empty() {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                return ScalarType::ArtifactId;
            }
            if receiver.as_deref() == Some("core") && name == "declare_artifact" {
                if arguments.len() != 1 {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "call argument arity does not match callable type",
                        *span,
                    ));
                    return ScalarType::Error;
                }
                let actual = expression_type_expected(
                    &arguments[0],
                    &ScalarType::Error,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(program, &ScalarType::Error, &actual, *span, diagnostics);
                return if is_error_type(&actual) {
                    ScalarType::Error
                } else {
                    ScalarType::ArtifactId
                };
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "alloc" | "free" | "invalidate" | "rebind" | "system_panic"
                )
            {
                return type_core_memory(
                    name,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return type_core_int_conversion(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "uint_to_float"
                        | "sint_to_float"
                        | "float_to_sint_trunc"
                        | "float_to_uint_trunc"
                        | "float_trunc"
                        | "float_extend"
                )
            {
                return type_core_float_conversion(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return type_core_bitcast(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core") && matches!(name.as_str(), "offset" | "load") {
                return type_core_raw_memory(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return type_core_pointer_cast(
                    name,
                    type_arguments,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
            }
            if let Some(receiver_type) = receiver.as_ref().and_then(|binding| scope.get(binding)) {
                if matches!(receiver_type, ScalarType::Struct(_)) {
                    let Some(field) = resolved_struct_field(receiver_type, name, &program.structs)
                    else {
                        diagnostics.push(diagnostic(
                            program,
                            "B0001",
                            "unknown callable field",
                            *name_span,
                        ));
                        return ScalarType::Error;
                    };
                    let ScalarType::Callable {
                        outputs,
                        parameters,
                    } = &field.ty
                    else {
                        diagnostics.push(diagnostic(
                            program,
                            "B0003",
                            "struct field is not callable",
                            *name_span,
                        ));
                        return ScalarType::Error;
                    };
                    if !type_arguments.is_empty() || arguments.len() != parameters.len() {
                        diagnostics.push(diagnostic(
                            program,
                            "B0004",
                            "call argument arity does not match callable type",
                            *span,
                        ));
                        return ScalarType::Error;
                    }
                    let mut error_argument = false;
                    for (argument, parameter) in arguments.iter().zip(parameters) {
                        let actual = expression_type_expected(
                            argument,
                            parameter,
                            scope,
                            visible_names,
                            folded_names,
                            program,
                            diagnostics,
                            unsafe_context,
                        );
                        expect_type(
                            program,
                            parameter,
                            &actual,
                            expression_span(argument),
                            diagnostics,
                        );
                        error_argument |= is_error_type(&actual);
                    }
                    return if error_argument {
                        ScalarType::Error
                    } else {
                        scalar_call_result(outputs)
                    };
                }
            }
            if let Some(overload) = program.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if receiver.is_none()
                        && function.name == *name
                        && !function.overload_arms.is_empty() =>
                {
                    Some(function)
                }
                _ => None,
            }) {
                let argument_types = arguments
                    .iter()
                    .map(|argument| {
                        expression_type(
                            argument,
                            scope,
                            visible_names,
                            folded_names,
                            program,
                            diagnostics,
                            unsafe_context,
                        )
                    })
                    .collect::<Vec<_>>();
                let overload_arguments = arguments
                    .iter()
                    .zip(&argument_types)
                    .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                    .collect::<Vec<_>>();
                let callable = match resolve_overload_candidate(overload, &overload_arguments, None)
                {
                    Ok((callable, _)) => callable,
                    Err(message) => {
                        diagnostics.push(diagnostic(program, "B0004", message, *name_span));
                        return ScalarType::Error;
                    }
                };
                let ScalarType::Callable {
                    outputs,
                    parameters,
                } = callable
                else {
                    unreachable!("overload arms are callable")
                };
                for (argument, parameter) in arguments.iter().zip(&parameters) {
                    let actual = expression_type_expected(
                        argument,
                        parameter,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                    expect_type(
                        program,
                        parameter,
                        &actual,
                        expression_span(argument),
                        diagnostics,
                    );
                }
                return scalar_call_result(&outputs);
            }
            let lookup_name = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            let callable = scope.get(&lookup_name).cloned().or_else(|| {
                program.items.iter().find_map(|item| match item {
                    ScalarItem::Extern(extern_decl)
                        if receiver.as_deref() == Some(extern_decl.binding.as_str()) =>
                    {
                        extern_decl
                            .functions
                            .iter()
                            .find(|function| function.name == *name)
                            .map(|function| function.signature.clone())
                    }
                    _ => None,
                })
            });
            let unsafe_callable = program.items.iter().any(|item| {
                matches!(
                    item,
                    ScalarItem::Extern(extern_decl)
                        if receiver.as_deref() == Some(extern_decl.binding.as_str())
                            && extern_decl.functions.iter().any(|function| {
                                function.name == *name && function.unsafe_marker
                            })
                )
            });
            let Some(callable) = callable else {
                diagnostics.push(diagnostic(program, "B0001", "unknown callable name", *span));
                return ScalarType::Error;
            };
            let extern_callable = program.items.iter().any(|item| {
                matches!(
                    item,
                    ScalarItem::Extern(extern_decl)
                        if receiver.as_deref() == Some(extern_decl.binding.as_str())
                            && extern_decl.functions.iter().any(|function| function.name == *name)
                )
            });
            if extern_callable && !type_arguments.is_empty() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "generic argument arity does not match callable declaration",
                    *span,
                ));
                return ScalarType::Error;
            }
            let generic_parameters = (receiver.is_none()).then(|| {
                program.items.iter().find_map(|item| match item {
                    ScalarItem::Function(function) if function.name == *name => {
                        Some(&function.generic_parameters)
                    }
                    _ => None,
                })
            });
            let callable = if let Some(Some(generic_parameters)) = generic_parameters {
                let Some(callable) =
                    substitute_generic_callable(&callable, generic_parameters, type_arguments)
                else {
                    diagnostics.push(diagnostic(
                        program,
                        "B0004",
                        "generic argument arity does not match callable declaration",
                        *span,
                    ));
                    return ScalarType::Error;
                };
                callable
            } else {
                callable
            };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
            else {
                diagnostics.push(diagnostic(program, "B0001", "unknown callable name", *span));
                return ScalarType::Error;
            };
            if arguments.len() != parameters.len() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            let mut error_argument = false;
            for (argument, parameter) in arguments.iter().zip(parameters) {
                let actual = expression_type_expected(
                    argument,
                    &parameter,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(program, &parameter, &actual, *span, diagnostics);
                error_argument |= is_error_type(&actual);
            }
            if unsafe_callable && !unsafe_context {
                diagnostics.push(diagnostic(
                    program,
                    "B0012",
                    "unsafe extern call requires an unsafe block",
                    *span,
                ));
            }
            if error_argument {
                ScalarType::Error
            } else {
                scalar_call_result(&outputs)
            }
        }
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } => {
            let condition_type = expression_type(
                condition,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !scalar_type_equal(&condition_type, &ScalarType::Bool)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0005",
                    "conditional expression requires bool",
                    *span,
                ));
            }
            let then_type = block_type(
                then_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let else_type = block_type(
                else_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && !scalar_type_equal(&then_type, &else_type)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            if is_error_type(&condition_type) || is_error_type(&then_type) {
                ScalarType::Error
            } else if is_error_type(&else_type) {
                ScalarType::Error
            } else {
                then_type
            }
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => {
            let condition_type = expression_type(
                condition,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            expect_type(
                program,
                &ScalarType::Bool,
                &condition_type,
                expression_span(condition),
                diagnostics,
            );
            block_type(
                then_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            ScalarType::Unit
        }
        ScalarExpression::Block(block) => block_type(
            block,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
    }
}

fn validate_conditional_output_arity(
    block: &ScalarBlock,
    output_count: usize,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if block.final_output_values.len() == output_count {
        for output in &block.final_output_values {
            if let ScalarExpression::If {
                then_branch,
                else_branch,
                ..
            } = &output.value
            {
                validate_conditional_output_arity(then_branch, output_count, program, diagnostics);
                validate_conditional_output_arity(else_branch, output_count, program, diagnostics);
            }
        }
        return;
    }
    if let [output] = block.final_output_values.as_slice() {
        if let ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } = &output.value
        {
            validate_conditional_output_arity(then_branch, output_count, program, diagnostics);
            validate_conditional_output_arity(else_branch, output_count, program, diagnostics);
            return;
        }
    }
    diagnostics.push(diagnostic(
        program,
        "B0004",
        "function output arity does not match its final output list",
        block
            .final_output_values
            .first()
            .map_or(block.span, |output| output.span),
    ));
}

fn expect_type(
    program: &ScalarProgram,
    expected: &ScalarType,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(expected) && !is_error_type(actual) && !scalar_type_equal(expected, actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "expression type does not match expected type",
            span,
        ));
    }
}

fn expect_integer(
    program: &ScalarProgram,
    actual: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_error_type(actual) && !is_integer_type(actual) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "integer operation requires integer operands",
            span,
        ));
    }
}

fn expression_type_expected(
    expression: &ScalarExpression,
    expected: &ScalarType,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarType {
    if is_error_type(expected) {
        return ScalarType::Error;
    }
    if let ScalarExpression::Call {
        receiver: None,
        name,
        name_span,
        type_arguments,
        arguments,
        ..
    } = expression
    {
        if let Some(overload) = program.items.iter().find_map(|item| match item {
            ScalarItem::Function(function)
                if function.name == *name && !function.overload_arms.is_empty() =>
            {
                Some(function)
            }
            _ => None,
        }) {
            if let Some(type_argument) = type_arguments.first() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "overload calls do not accept explicit type arguments",
                    type_argument.span,
                ));
                return ScalarType::Error;
            }
            let argument_types = arguments
                .iter()
                .map(|argument| {
                    expression_type(
                        argument,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                })
                .collect::<Vec<_>>();
            let overload_arguments = arguments
                .iter()
                .zip(&argument_types)
                .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                .collect::<Vec<_>>();
            let callable =
                match resolve_overload_candidate(overload, &overload_arguments, Some(expected)) {
                    Ok((callable, _)) => callable,
                    Err(message) => {
                        diagnostics.push(diagnostic(program, "B0004", message, *name_span));
                        return ScalarType::Error;
                    }
                };
            let ScalarType::Callable {
                outputs,
                parameters,
            } = callable
            else {
                unreachable!("overload arms are callable")
            };
            for (argument, parameter) in arguments.iter().zip(&parameters) {
                let actual = expression_type_expected(
                    argument,
                    parameter,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    parameter,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            return scalar_call_result(&outputs);
        }
    }
    if let ScalarExpression::Unary {
        operator,
        operand,
        span,
    } = expression
    {
        if matches!(operator, UnaryOperator::Negate) {
            if let ScalarExpression::Integer { value, .. } = operand.as_ref() {
                let value = -value;
                if is_integer_type(expected) {
                    validate_integer_range_for_type_program(
                        program,
                        &value,
                        *span,
                        expected,
                        diagnostics,
                    );
                    return expected.clone();
                }
            }
        }
        let operand_type = expression_type_expected(
            operand,
            if matches!(operator, UnaryOperator::LogicalNot) {
                &ScalarType::Bool
            } else {
                expected
            },
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        );
        match operator {
            UnaryOperator::LogicalNot => {
                expect_type(
                    program,
                    &ScalarType::Bool,
                    &operand_type,
                    *span,
                    diagnostics,
                );
                ScalarType::Bool
            }
            UnaryOperator::BitwiseNot => {
                expect_integer(program, &operand_type, *span, diagnostics);
                expect_type(program, expected, &operand_type, *span, diagnostics);
                operand_type
            }
            UnaryOperator::Negate => {
                expect_integer(program, &operand_type, *span, diagnostics);
                expect_type(program, expected, &operand_type, *span, diagnostics);
                operand_type
            }
        }
    } else {
        if let ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            span,
        } = expression
        {
            let condition_type = expression_type(
                condition,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&condition_type)
                && !scalar_type_equal(&condition_type, &ScalarType::Bool)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0005",
                    "conditional expression requires bool",
                    *span,
                ));
            }
            let then_type = block_type_expected(
                then_branch,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let else_type = block_type_expected(
                else_branch,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !is_error_type(&then_type)
                && !is_error_type(&else_type)
                && !scalar_type_equal(&then_type, &else_type)
            {
                diagnostics.push(diagnostic(
                    program,
                    "B0006",
                    "conditional branches must have equal types",
                    *span,
                ));
            }
            return then_type;
        }
        if matches!(expression, ScalarExpression::Name { name, .. } if name == "null")
            && matches!(
                expected,
                ScalarType::RawPointer(_) | ScalarType::CheckedReference { .. }
            )
        {
            return expected.clone();
        }
        if matches!(expression, ScalarExpression::Integer { .. })
            && matches!(
                expected,
                ScalarType::I8
                    | ScalarType::I16
                    | ScalarType::I32
                    | ScalarType::I64
                    | ScalarType::I128
                    | ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
            )
        {
            if let ScalarExpression::Integer { value, span } = expression {
                validate_integer_range_for_type_program(
                    program,
                    value,
                    *span,
                    expected,
                    diagnostics,
                );
            }
            return expected.clone();
        }
        if matches!(expression, ScalarExpression::Float { .. })
            && matches!(expected, ScalarType::F32 | ScalarType::F64)
        {
            if let ScalarExpression::Float { value, span, .. } = expression {
                if *expected == ScalarType::F32 && !(*value as f32).is_finite() {
                    diagnostics.push(diagnostic(
                        program,
                        "B0010",
                        "floating-point literal is outside the f32 range",
                        *span,
                    ));
                    return ScalarType::Error;
                }
            }
            return expected.clone();
        }
        if let ScalarExpression::Block(block) = expression {
            return block_type_expected(
                block,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
        }
        if let ScalarExpression::Binary {
            operator,
            left,
            right,
            span,
        } = expression
        {
            let comparison = matches!(
                operator,
                BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEqual
            );
            let boolean = matches!(operator, BinaryOperator::And | BinaryOperator::Or);
            let arithmetic_context = matches!(
                expected,
                ScalarType::I8
                    | ScalarType::I16
                    | ScalarType::I32
                    | ScalarType::I64
                    | ScalarType::I128
                    | ScalarType::U8
                    | ScalarType::U16
                    | ScalarType::U32
                    | ScalarType::U64
                    | ScalarType::U128
                    | ScalarType::F32
                    | ScalarType::F64
            );
            let bitwise = matches!(
                operator,
                BinaryOperator::BitAnd
                    | BinaryOperator::BitOr
                    | BinaryOperator::BitXor
                    | BinaryOperator::ShiftLeft
                    | BinaryOperator::ShiftRight
            );
            let bool_context = ScalarType::Bool;
            let (left_type, right_type) = if comparison && is_null_expression(left) {
                let right_type = expression_type(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                let left_type = expression_type_expected(
                    left,
                    &right_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            } else if comparison && matches!(left.as_ref(), ScalarExpression::Float { .. }) {
                let right_type = expression_type(
                    right,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                let left_type = expression_type_expected(
                    left,
                    &right_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            } else if comparison && matches!(right.as_ref(), ScalarExpression::Float { .. }) {
                let left_type = expression_type(
                    left,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                let right_type = expression_type_expected(
                    right,
                    &left_type,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            } else {
                let left_type = if boolean {
                    expression_type_expected(
                        left,
                        &bool_context,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                } else if comparison {
                    expression_type(
                        left,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                } else {
                    expression_type_expected(
                        left,
                        expected,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    )
                };
                let right_type = expression_type_expected(
                    right,
                    if boolean {
                        &bool_context
                    } else if comparison {
                        &left_type
                    } else {
                        expected
                    },
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                (left_type, right_type)
            };
            if boolean {
                expect_type(program, &bool_context, &left_type, *span, diagnostics);
                expect_type(program, &bool_context, &right_type, *span, diagnostics);
                return ScalarType::Bool;
            }
            if is_error_type(&left_type) || is_error_type(&right_type) {
                return ScalarType::Error;
            }
            if comparison {
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                return ScalarType::Bool;
            }
            if bitwise {
                expect_integer(program, &left_type, *span, diagnostics);
                expect_integer(program, &right_type, *span, diagnostics);
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                return left_type;
            }
            expect_type(
                program,
                if arithmetic_context {
                    expected
                } else {
                    &ScalarType::I32
                },
                &left_type,
                *span,
                diagnostics,
            );
            expect_type(program, &left_type, &right_type, *span, diagnostics);
            if arithmetic_context {
                return expected.clone();
            }
            return ScalarType::I32;
        }
        if let ScalarExpression::ArrayLiteral { elements, span } = expression {
            let ScalarType::Array {
                element, length, ..
            } = expected
            else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "array literal requires a fixed array context",
                    *span,
                ));
                return ScalarType::Error;
            };
            if elements.len() as u64 != *length {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "array literal element count does not match fixed array length",
                    *span,
                ));
            }
            for value in elements {
                let actual = expression_type_expected(
                    value,
                    element,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(
                    program,
                    element,
                    &actual,
                    expression_span(value),
                    diagnostics,
                );
            }
            return expected.clone();
        }
        if let ScalarExpression::StructLiteral { fields, span } = expression {
            let ScalarType::Struct(id) = expected else {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "struct literal requires a struct context",
                    *span,
                ));
                return ScalarType::Error;
            };
            let Some(structure) = program
                .structs
                .get(id.index)
                .filter(|structure| structure.id == *id)
            else {
                diagnostics.push(diagnostic(program, "B0003", "invalid struct type", *span));
                return ScalarType::Error;
            };
            let mut seen = BTreeSet::new();
            let mut initialized = BTreeSet::new();
            for field in fields {
                if !seen.insert(field.name.clone()) {
                    diagnostics.push(diagnostic(
                        program,
                        "B0002",
                        "duplicate struct literal field",
                        field.name_span,
                    ));
                    continue;
                }
                let Some(declared) = structure.fields.iter().find(|item| item.name == field.name)
                else {
                    diagnostics.push(diagnostic(
                        program,
                        "M0002",
                        "unknown struct field",
                        field.name_span,
                    ));
                    continue;
                };
                initialized.insert(field.name.clone());
                let actual = expression_type_expected(
                    &field.value,
                    &declared.ty,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                );
                expect_type(program, &declared.ty, &actual, field.span, diagnostics);
            }
            if initialized.len() != structure.fields.len() {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    "struct literal must initialize every field",
                    *span,
                ));
            }
            return expected.clone();
        }
        expression_type(
            expression,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        )
    }
}

fn is_null_expression(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Name { name, .. } => name == "null",
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => is_null_block(then_branch) && is_null_block(else_branch),
        ScalarExpression::Block(block) => is_null_block(block),
        _ => false,
    }
}

fn is_null_block(block: &ScalarBlock) -> bool {
    match (block.items.last(), block.terminated_items.last()) {
        (Some(ScalarBlockItem::Expression(expression)), Some(false)) => {
            is_null_expression(expression)
        }
        _ => false,
    }
}

fn field_type(
    receiver: Option<&ScalarType>,
    name: &str,
    name_span: ByteSpan,
    span: ByteSpan,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let Some(receiver) = receiver else {
        diagnostics.push(diagnostic(program, "B0001", "unknown name", span));
        return ScalarType::Error;
    };
    let structure = match receiver {
        ScalarType::Struct(id) => program.structs.get(id.index),
        ScalarType::RawPointer(inner) => match inner.as_ref() {
            ScalarType::Struct(id) => program.structs.get(id.index),
            _ => None,
        },
        _ => None,
    };
    let Some(structure) = structure else {
        diagnostics.push(diagnostic(program, "B0001", "unknown field", name_span));
        return ScalarType::Error;
    };
    let Some(field) = structure.fields.iter().find(|field| field.name == name) else {
        diagnostics.push(diagnostic(program, "B0001", "unknown field", name_span));
        return ScalarType::Error;
    };
    field.ty.clone()
}

fn validate_integer_range(
    module: &ScalarModule,
    value: &BigInt,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if value > &BigInt::from(i32::MAX) {
        diagnostics.push(module_diagnostic(
            module,
            "B0010",
            "integer literal is outside the resolved target type range",
            span,
        ));
    }
}

fn validate_integer_range_for_type(
    module: &ScalarModule,
    value: &BigInt,
    span: ByteSpan,
    ty: &ScalarType,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (minimum, maximum) = match ty {
        ScalarType::I8 => (BigInt::from(i8::MIN), BigInt::from(i8::MAX)),
        ScalarType::I16 => (BigInt::from(i16::MIN), BigInt::from(i16::MAX)),
        ScalarType::I32 => (BigInt::from(i32::MIN), BigInt::from(i32::MAX)),
        ScalarType::I64 => (BigInt::from(i64::MIN), BigInt::from(i64::MAX)),
        ScalarType::I128 => (BigInt::from(i128::MIN), BigInt::from(i128::MAX)),
        ScalarType::U8 => (BigInt::from(0), BigInt::from(u8::MAX)),
        ScalarType::U16 => (BigInt::from(0), BigInt::from(u16::MAX)),
        ScalarType::U32 => (BigInt::from(0), BigInt::from(u32::MAX)),
        ScalarType::U64 => (BigInt::from(0), BigInt::from(u64::MAX)),
        ScalarType::U128 => (BigInt::from(0), BigInt::from(u128::MAX)),
        _ => return,
    };
    if value < &minimum || value > &maximum {
        diagnostics.push(module_diagnostic(
            module,
            "B0010",
            "integer literal is outside the resolved target type range",
            span,
        ));
    }
}

fn validate_integer_range_program(
    program: &ScalarProgram,
    value: &BigInt,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if value > &BigInt::from(i32::MAX) {
        diagnostics.push(diagnostic(
            program,
            "B0010",
            "integer literal is outside the resolved target type range",
            span,
        ));
    }
}

fn validate_integer_range_for_type_program(
    program: &ScalarProgram,
    value: &BigInt,
    span: ByteSpan,
    ty: &ScalarType,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let (minimum, maximum) = match ty {
        ScalarType::I8 => (BigInt::from(i8::MIN), BigInt::from(i8::MAX)),
        ScalarType::I16 => (BigInt::from(i16::MIN), BigInt::from(i16::MAX)),
        ScalarType::I32 => (BigInt::from(i32::MIN), BigInt::from(i32::MAX)),
        ScalarType::I64 => (BigInt::from(i64::MIN), BigInt::from(i64::MAX)),
        ScalarType::I128 => (BigInt::from(i128::MIN), BigInt::from(i128::MAX)),
        ScalarType::U8 => (BigInt::from(0), BigInt::from(u8::MAX)),
        ScalarType::U16 => (BigInt::from(0), BigInt::from(u16::MAX)),
        ScalarType::U32 => (BigInt::from(0), BigInt::from(u32::MAX)),
        ScalarType::U64 => (BigInt::from(0), BigInt::from(u64::MAX)),
        ScalarType::U128 => (BigInt::from(0), BigInt::from(u128::MAX)),
        _ => return,
    };
    if value < &minimum || value > &maximum {
        diagnostics.push(diagnostic(
            program,
            "B0010",
            "integer literal is outside the resolved target type range",
            span,
        ));
    }
}

fn is_const_binding_name(name: &str) -> bool {
    name != "_"
        && name.bytes().any(|byte| byte.is_ascii_uppercase())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'_' || byte.is_ascii_digit())
}

pub(super) fn diagnostic(
    program: &ScalarProgram,
    code: &str,
    message: &str,
    span: ByteSpan,
) -> super::Diagnostic {
    super::Diagnostic {
        code: code.to_owned(),
        severity: super::DiagnosticSeverity::Error,
        message: message.to_owned(),
        labels: vec![super::DiagnosticLabel {
            kind: super::DiagnosticLabelKind::Primary,
            span: SourceSpan::new(program.source.clone(), span),
            message: message.to_owned(),
        }],
        notes: Vec::new(),
    }
}

fn declare_program_name(
    program: &ScalarProgram,
    name: &str,
    span: ByteSpan,
    names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> bool {
    if !names.insert(name.to_owned()) {
        diagnostics.push(diagnostic(program, "B0002", "duplicate declaration", span));
        return false;
    }
    let folded = fold_name(name);
    if let Some((first_name, first_span)) = folded_names.get(&folded) {
        if first_name != name {
            names.remove(name);
            let mut collision = diagnostic(
                program,
                "B0008",
                "declaration collides with an existing name under ASCII case folding",
                span,
            );
            collision.labels.push(super::DiagnosticLabel {
                kind: super::DiagnosticLabelKind::Secondary,
                span: SourceSpan::new(program.source.clone(), *first_span),
                message: "first conflicting declaration".to_owned(),
            });
            diagnostics.push(collision);
            return false;
        }
    } else {
        folded_names.insert(folded, (name.to_owned(), span));
    }
    true
}
