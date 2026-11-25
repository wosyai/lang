use super::inference::CheckedMember;
use super::*;
use std::borrow::Cow;

pub(super) struct ScalarProgramTyping<'a> {
    pub source: &'a SourceIdentity,
    pub structs: &'a [ScalarStruct],
    pub enums: &'a [ScalarEnum],
    pub target_layout: ScalarTargetLayout,
}

impl ScalarProgramTyping<'_> {
    fn local_utf8_id(&self) -> Option<&ScalarStructId> {
        if self.source.package != "std" {
            return None;
        }
        self.structs
            .iter()
            .find(|structure| structure.name == "utf8" && &structure.id.source == self.source)
            .map(|structure| &structure.id)
    }
}

pub(super) struct ScalarModuleTyping<'a> {
    pub source: &'a SourceIdentity,
    pub members: &'a BTreeMap<String, ScalarType>,
    pub namespace_bindings: &'a [ScalarNamespaceBinding],
    pub structs: &'a [ScalarStruct],
    pub enums: &'a [ScalarEnum],
    pub target_layout: ScalarTargetLayout,
}

pub(super) fn split_module_typing(
    modules: &mut [ScalarModule],
) -> (Vec<ScalarModuleTyping<'_>>, Vec<&mut [ScalarItem]>) {
    modules
        .iter_mut()
        .map(|module| {
            (
                ScalarModuleTyping {
                    source: &module.source,
                    members: &module.members,
                    namespace_bindings: &module.namespace_bindings,
                    structs: &module.structs,
                    enums: &module.enums,
                    target_layout: module.target_layout,
                },
                module.items.as_mut_slice(),
            )
        })
        .unzip()
}

#[cfg(test)]
mod callable_context_tests;
#[cfg(test)]
mod source_context_tests;
#[cfg(test)]
mod tail_conditional_tests;

pub(super) fn validate_module_type(
    source: &SourceIdentity,
    ty: &ScalarType,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match ty {
        ScalarType::Named { span, .. } | ScalarType::Qualified { span, .. } => diagnostics.push(
            source_diagnostic(source, "B0003", "unknown scalar type", *span),
        ),
        ScalarType::Callable {
            outputs,
            parameters,
        } => {
            for output in &outputs.outputs {
                validate_module_type(source, &output.ty, output.span, diagnostics);
            }
            for parameter in parameters {
                validate_module_type(source, parameter, span, diagnostics);
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
            validate_module_type(source, inner, span, diagnostics)
        }
        ScalarType::RawPointer(inner)
            if matches!(inner.as_ref(), ScalarType::U8 | ScalarType::Struct(_)) => {}
        ScalarType::RawPointer(_) => diagnostics.push(source_diagnostic(
            source,
            "B0003",
            "unsupported raw pointer type",
            span,
        )),
        ScalarType::Array { element, .. } | ScalarType::RuntimeArray { element, .. } => {
            validate_module_type(source, element, span, diagnostics)
        }
        ScalarType::Struct(_) | ScalarType::Enum(_) => {}
        ScalarType::Error => {}
    }
}

pub(super) fn validate_module_generic_type(
    module: &ScalarModuleTyping<'_>,
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
        _ => validate_module_type(&module.source, ty, span, diagnostics),
    }
}

pub(super) fn validate_extern(
    source: &SourceIdentity,
    extern_decl: &ScalarExtern,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    let valid_binding = extern_decl.kind == "wasm"
        && matches!(extern_decl.actual_module, ScalarExternModule::Valid(_))
        && !extern_decl.functions.is_empty();
    if !valid_binding {
        diagnostics.push(source_diagnostic(
            source,
            "B0003",
            "unsupported extern declaration",
            extern_decl.span,
        ));
    }
    for function in &extern_decl.functions {
        validate_identifier_style(source, &function.name, function.name_span, diagnostics);
        validate_module_type(
            source,
            &function.signature,
            function.signature_span,
            diagnostics,
        );
        if contains_fixed_array(&function.signature) {
            diagnostics.push(source_diagnostic(
                source,
                "B0003",
                "direct FFI arrays are not supported",
                function.signature_span,
            ));
        }
        if contains_checked_reference(&function.signature) {
            diagnostics.push(source_diagnostic(
                source,
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

fn call_output_sequence_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    overloads: &ScalarTypingView<'_>,
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
    let (callable, target, target_view) = match receiver {
        None => (scope.get(name), module, *overloads),
        Some(binding) => {
            let namespace = module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *binding);
            if let Some(namespace) = namespace {
                let target = modules
                    .iter()
                    .find(|candidate| candidate.source == &namespace.target)?;
                (
                    Some(target.members.get(name)?),
                    target,
                    overloads.imported(&target.source),
                )
            } else {
                if !type_arguments.is_empty() {
                    return None;
                }
                let member = overloads.extern_member(Some(binding), name)?;
                let ScalarType::Callable { outputs, .. } = &member.signature else {
                    return None;
                };
                return Some(outputs.clone());
            }
        }
    };
    if let Some(overload_header) = overloads.first(&target.source, name) {
        let argument_types = arguments
            .iter()
            .map(|argument| overload_argument_type(argument, scope))
            .collect::<Option<Vec<_>>>()?;
        let ScalarType::Callable { outputs, .. } =
            resolve_overload_candidate(overload_header, &argument_types, None)
                .ok()?
                .0
        else {
            return None;
        };
        return Some(outputs);
    }
    let callable = callable?;
    if let Some(parameters) = target_view.generics(name) {
        let ScalarType::Callable { outputs, .. } =
            substitute_generic_callable(callable, parameters, type_arguments)?
        else {
            return None;
        };
        return Some(outputs);
    }
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs.clone())
}

fn call_output_sequence(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    overloads: &ScalarTypingView<'_>,
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
        overloads
            .extern_member(receiver.as_deref(), name)
            .map(|member| &member.signature)
    }?;
    let extern_callable = overloads.extern_member(receiver.as_deref(), name).is_some();
    if extern_callable && !type_arguments.is_empty() {
        return None;
    }
    if let Some(overload_header) = receiver
        .is_none()
        .then(|| overloads.first(&program.source, name))
        .flatten()
    {
        let argument_types = arguments
            .iter()
            .map(|argument| overload_argument_type(argument, scope))
            .collect::<Option<Vec<_>>>()?;
        let ScalarType::Callable { outputs, .. } =
            resolve_overload_candidate(overload_header, &argument_types, None)
                .ok()?
                .0
        else {
            return None;
        };
        return Some(outputs);
    }
    let generic_parameters = receiver.is_none().then(|| overloads.generics(name));
    if let Some(Some(parameters)) = generic_parameters {
        let ScalarType::Callable { outputs, .. } =
            substitute_generic_callable(callable, parameters, type_arguments)?
        else {
            return None;
        };
        return Some(outputs);
    }
    let ScalarType::Callable { outputs, .. } = callable else {
        return None;
    };
    Some(outputs.clone())
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
        | ScalarExpression::ValueField { span, .. }
        | ScalarExpression::StructLiteral { span, .. }
        | ScalarExpression::ArrayLiteral { span, .. }
        | ScalarExpression::AnonymousCallable { span, .. }
        | ScalarExpression::GeneratedArray { span, .. }
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
) {
    let Some(outputs) =
        call_output_sequence_in_module(expression, scope, module, modules, overloads)
    else {
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
) {
    let Some(outputs) = call_output_sequence(expression, scope, program, overloads) else {
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

struct BlockLexicalScope<'a> {
    types: Cow<'a, BTreeMap<String, ScalarType>>,
    names: Cow<'a, BTreeSet<String>>,
    folded_names: Cow<'a, BTreeMap<String, (String, ByteSpan)>>,
}

impl<'a> BlockLexicalScope<'a> {
    fn new(
        types: &'a BTreeMap<String, ScalarType>,
        names: &'a BTreeSet<String>,
        folded_names: &'a BTreeMap<String, (String, ByteSpan)>,
    ) -> Self {
        Self {
            types: Cow::Borrowed(types),
            names: Cow::Borrowed(names),
            folded_names: Cow::Borrowed(folded_names),
        }
    }

    fn item_type(
        &mut self,
        item: &ScalarBlockItem,
        typing: TailConditionalTyping<'_>,
        diagnostics: &mut Vec<super::Diagnostic>,
        unsafe_context: bool,
        overloads: &ScalarTypingView<'_>,
        inference: &mut ScalarInferenceBuilder<'_>,
    ) -> ScalarType {
        match (item, typing) {
            (ScalarBlockItem::Expression(expression), _) => typing.expression_type(
                expression,
                None,
                &self.types,
                &self.names,
                &self.folded_names,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            (ScalarBlockItem::Assignment(assignment), TailConditionalTyping::Program(program)) => {
                assignment_type(
                    assignment,
                    self.types.to_mut(),
                    &self.names,
                    &self.folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                )
            }
            (
                ScalarBlockItem::Assignment(assignment),
                TailConditionalTyping::Module { module, modules },
            ) => assignment_type_in_module(
                assignment,
                self.types.to_mut(),
                &self.names,
                &self.folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            (ScalarBlockItem::LocalBinding(_), _) => typing.block_item_type(
                item,
                self.types.to_mut(),
                self.names.to_mut(),
                self.folded_names.to_mut(),
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
        }
    }
}

pub(super) fn block_type_in_module(
    block: &ScalarBlock,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let mut scope = BlockLexicalScope::new(scope, visible_names, folded_names);
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = scope.item_type(
            item,
            TailConditionalTyping::Module { module, modules },
            diagnostics,
            unsafe_context,
            overloads,
            inference,
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let mut scope = BlockLexicalScope::new(scope, visible_names, folded_names);
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
                    &scope.types,
                    &scope.names,
                    &scope.folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                )
            }
            _ => scope.item_type(
                item,
                TailConditionalTyping::Module { module, modules },
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
        };
        if index + 1 == block.items.len() && block.terminated_items[index] {
            result = ScalarType::Unit;
        }
    }
    result
}

#[derive(Clone, Copy)]
pub(super) enum TailConditionalTyping<'a> {
    Program(&'a ScalarProgramTyping<'a>),
    Module {
        module: &'a ScalarModuleTyping<'a>,
        modules: &'a [ScalarModuleTyping<'a>],
    },
}

impl TailConditionalTyping<'_> {
    pub(super) fn validate_sole_final_call(
        self,
        expression: &ScalarExpression,
        span: ByteSpan,
        outputs: &ScalarOutputSequence,
        scope: &BTreeMap<String, ScalarType>,
        visible_names: &BTreeSet<String>,
        folded_names: &BTreeMap<String, (String, ByteSpan)>,
        diagnostics: &mut Vec<super::Diagnostic>,
        overloads: &ScalarTypingView<'_>,
        inference: &mut ScalarInferenceBuilder<'_>,
    ) {
        let preliminary_outputs = match self {
            Self::Program(program) => call_output_sequence(expression, scope, program, overloads),
            Self::Module { module, modules } => {
                call_output_sequence_in_module(expression, scope, module, modules, overloads)
            }
        };
        let expected = match &preliminary_outputs {
            Some(actual) => actual.outputs.first().map(|output| &output.ty),
            None => Some(&outputs.outputs[0].ty),
        };
        let diagnostics_before_call = diagnostics.len();
        let actual = match self {
            Self::Program(_) => self.expression_type(
                expression,
                expected,
                scope,
                visible_names,
                folded_names,
                diagnostics,
                false,
                overloads,
                inference,
            ),
            Self::Module { module, modules } => expression_type_in_module_expected(
                expression,
                expected,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                false,
                overloads,
                inference,
            ),
        };
        let actual_outputs = match &preliminary_outputs {
            Some(actual) => actual,
            None => {
                if is_error_type(&actual) || diagnostics.len() != diagnostics_before_call {
                    return;
                }
                let Some(checked) = inference.checked_non_core_call_outputs(expression) else {
                    return;
                };
                checked
            }
        };
        if actual_outputs.outputs.len() != outputs.outputs.len() {
            diagnostics.push(self.diagnostic(
                "B0004",
                "function output arity does not match its final output list",
                span,
            ));
        }
        for (actual, expected) in actual_outputs.outputs.iter().zip(&outputs.outputs) {
            match self {
                Self::Program(program) => {
                    expect_type(program, &expected.ty, &actual.ty, span, diagnostics);
                }
                Self::Module { module, .. } => {
                    expect_module_type(module, &expected.ty, &actual.ty, span, diagnostics);
                }
            }
        }
    }

    pub(super) fn validate_secondary_binding_initializers(
        self,
        binding: &ScalarBinding,
        scope: &BTreeMap<String, ScalarType>,
        visible_names: &BTreeSet<String>,
        folded_names: &BTreeMap<String, (String, ByteSpan)>,
        diagnostics: &mut Vec<super::Diagnostic>,
        unsafe_context: bool,
        overloads: &ScalarTypingView<'_>,
        inference: &mut ScalarInferenceBuilder<'_>,
    ) {
        if binding.output_origin != ScalarBindingOutputOrigin::IndependentExpressions {
            return;
        }
        for (position, receiver) in binding.receivers.iter().enumerate().skip(1) {
            let ScalarBindingOutput::Evaluate(expression) = binding.output_source(position) else {
                unreachable!("independent binding output evaluates its source")
            };
            let actual = self.expression_type(
                expression,
                Some(&receiver.ty),
                scope,
                visible_names,
                folded_names,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            );
            let span = expression_span(expression);
            match self {
                Self::Program(program) => {
                    expect_type(program, &receiver.ty, &actual, span, diagnostics);
                }
                Self::Module { module, .. } => {
                    expect_module_type(module, &receiver.ty, &actual, span, diagnostics);
                }
            }
        }
    }

    pub(super) fn declare_parameters(
        self,
        function: &ScalarFunction,
        parameters: &[ScalarType],
        scope: &mut BTreeMap<String, ScalarType>,
        visible_names: &mut BTreeSet<String>,
        folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
        diagnostics: &mut Vec<super::Diagnostic>,
    ) {
        for (index, name) in function.parameters.iter().enumerate() {
            let span = function.parameter_spans[index];
            match self {
                Self::Program(program) => {
                    validate_program_identifier_style(program, name, span, diagnostics);
                }
                Self::Module { module, .. } => {
                    validate_identifier_style(module.source, name, span, diagnostics);
                }
            }
            let Some(ty) = parameters.get(index) else {
                continue;
            };
            let declared = match self {
                Self::Program(program) => declare_program_name(
                    program,
                    name,
                    function.span,
                    visible_names,
                    folded_names,
                    diagnostics,
                ),
                Self::Module { module, .. } => declare_module_name(
                    module,
                    name,
                    function.span,
                    visible_names,
                    folded_names,
                    diagnostics,
                ),
            };
            if declared {
                scope.insert(name.clone(), ty.clone());
            }
        }
    }

    fn expression_type(
        self,
        expression: &ScalarExpression,
        expected: Option<&ScalarType>,
        scope: &BTreeMap<String, ScalarType>,
        visible_names: &BTreeSet<String>,
        folded_names: &BTreeMap<String, (String, ByteSpan)>,
        diagnostics: &mut Vec<super::Diagnostic>,
        unsafe_context: bool,
        overloads: &ScalarTypingView<'_>,
        inference: &mut ScalarInferenceBuilder<'_>,
    ) -> ScalarType {
        match (self, expected) {
            (Self::Program(program), None) => expression_type(
                expression,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            (Self::Program(program), Some(expected)) => expression_type_expected(
                expression,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            (Self::Module { module, modules }, None) => expression_type_in_module(
                expression,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            (Self::Module { module, modules }, Some(expected)) => {
                expression_type_in_module_expected(
                    expression,
                    Some(expected),
                    scope,
                    visible_names,
                    folded_names,
                    module,
                    modules,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                )
            }
        }
    }

    fn block_item_type(
        self,
        item: &ScalarBlockItem,
        scope: &mut BTreeMap<String, ScalarType>,
        visible_names: &mut BTreeSet<String>,
        folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
        diagnostics: &mut Vec<super::Diagnostic>,
        unsafe_context: bool,
        overloads: &ScalarTypingView<'_>,
        inference: &mut ScalarInferenceBuilder<'_>,
    ) -> ScalarType {
        match self {
            Self::Program(program) => block_item_type(
                item,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            Self::Module { module, modules } => block_item_type_in_module(
                item,
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
        }
    }

    fn diagnostic(self, code: &str, message: &str, span: ByteSpan) -> super::Diagnostic {
        match self {
            Self::Program(program) => diagnostic(program, code, message, span),
            Self::Module { module, .. } => module_diagnostic(module, code, message, span),
        }
    }
}

pub(super) fn validate_tail_conditional_return(
    tail: ScalarTailConditional<'_>,
    expected: &[ScalarOutput],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    typing: TailConditionalTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) {
    let span = tail.span;
    let actual = tail_conditional_types(
        tail,
        expected,
        scope,
        visible_names,
        folded_names,
        typing,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
    );
    for (output, actual) in expected.iter().zip(&actual) {
        match typing {
            TailConditionalTyping::Program(program) => {
                expect_type(program, &output.ty, actual, span, diagnostics);
            }
            TailConditionalTyping::Module { module, .. } => {
                expect_module_type(module, &output.ty, actual, span, diagnostics);
            }
        }
    }
}

pub(super) fn tail_conditional_types(
    tail: ScalarTailConditional<'_>,
    expected: &[ScalarOutput],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    typing: TailConditionalTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> Vec<ScalarType> {
    debug_assert!(expected.len() > 1);
    let diagnostics_before = diagnostics.len();
    let condition_type = typing.expression_type(
        tail.condition,
        None,
        scope,
        visible_names,
        folded_names,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
    );
    if !is_error_type(&condition_type) && !scalar_type_equal(&condition_type, &ScalarType::Bool) {
        diagnostics.push(typing.diagnostic(
            "B0005",
            "conditional expression requires bool",
            tail.span,
        ));
    }
    let mut check_arm = |block| {
        conditional_arm_types(
            block,
            expected,
            scope,
            visible_names,
            folded_names,
            typing,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
        )
    };
    let mut actual = check_arm(tail.then_branch);
    let else_types = check_arm(tail.else_branch);
    for (then_type, else_type) in actual.iter_mut().zip(else_types) {
        if is_error_type(then_type) || is_error_type(&else_type) {
            *then_type = ScalarType::Error;
        } else if !scalar_type_equal(then_type, &else_type) {
            diagnostics.push(typing.diagnostic(
                "B0006",
                "conditional branches must have equal types",
                tail.span,
            ));
        }
    }
    inference.record_expression_type(tail.expression, &actual[0], true);
    if diagnostics.len() == diagnostics_before
        && scalar_type_equal(&condition_type, &ScalarType::Bool)
    {
        inference.record_tail_conditional_outputs(tail.expression, &actual);
    }
    actual
}

pub(super) fn check_final_output_prefix(
    block: &ScalarBlock,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    typing: TailConditionalTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) {
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        match item {
            ScalarBlockItem::Expression(
                expression @ (ScalarExpression::If { .. } | ScalarExpression::UnitIf { .. }),
            ) => typing.expression_type(
                expression,
                Some(&ScalarType::Unit),
                scope,
                visible_names,
                folded_names,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            _ => typing.block_item_type(
                item,
                scope,
                visible_names,
                folded_names,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
        };
    }
}

fn conditional_arm_types(
    block: &ScalarBlock,
    expected: &[ScalarOutput],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    typing: TailConditionalTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> Vec<ScalarType> {
    let mut scope = BlockLexicalScope::new(scope, visible_names, folded_names);
    let unsafe_context = unsafe_context || block.unsafe_context;
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        match item {
            ScalarBlockItem::Expression(
                expression @ (ScalarExpression::If { .. } | ScalarExpression::UnitIf { .. }),
            ) => typing.expression_type(
                expression,
                Some(&ScalarType::Unit),
                &scope.types,
                &scope.names,
                &scope.folded_names,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
            _ => scope.item_type(
                item,
                typing,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ),
        };
    }
    if let Some(tail) = block.tail_conditional() {
        return tail_conditional_types(
            tail,
            expected,
            &scope.types,
            &scope.names,
            &scope.folded_names,
            typing,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
        );
    }
    if block.final_output_values.len() != expected.len() {
        diagnostics.push(
            typing.diagnostic(
                "B0004",
                "function output arity does not match its final output list",
                block
                    .final_output_values
                    .first()
                    .map_or(block.span, |output| output.span),
            ),
        );
    }
    let mut actual = Vec::with_capacity(expected.len());
    for ((expression, _), output) in block.final_expressions().zip(expected) {
        actual.push(typing.expression_type(
            expression,
            Some(&output.ty),
            &scope.types,
            &scope.names,
            &scope.folded_names,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
        ));
    }
    actual.resize(expected.len(), ScalarType::Error);
    actual
}

pub(super) fn block_item_type_in_module(
    item: &ScalarBlockItem,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &mut BTreeSet<String>,
    folded_names: &mut BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            if let Some(operator) = pending_fallible_operator(binding, module.structs, scope) {
                diagnostics.push(fallible_assignment_diagnostic(module.source, operator));
                scope.insert(binding.name.clone(), ScalarType::Error);
                return ScalarType::Unit;
            }
            for receiver in &binding.receivers {
                validate_identifier_style(
                    &module.source,
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
                        overloads,
                        inference,
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
            let widening = widened_local_literal_type(binding);
            if !unsafe_context {
                inference.nominate_inline_elements(binding);
            }
            let actual = if let ScalarExpression::GeneratedArray { factory, count, span } = &binding.value {
                let supported = !unsafe_context && binding.receivers.len() == 1
                    && binding.fallible_assignment.is_none() && matches!(factory.as_ref(), ScalarExpression::Name { .. });
                let expected = match (&binding.declared_type, count.as_ref()) {
                    (ScalarType::Array { element, length, .. }, ScalarExpression::Integer { value, .. })
                        if supported && u64::try_from(value).ok() == Some(*length) => Some(element.as_ref()),
                    _ => None,
                };
                let signature = expected.and_then(|element| {
                    let signature = expression_type_in_module(
                        factory, scope, visible_names, folded_names, module, modules, diagnostics,
                        unsafe_context, overloads, inference,
                    );
                    match &signature {
                        ScalarType::Callable { parameters, outputs }
                            if parameters.as_slice() == [ScalarType::U64]
                                && matches!(outputs.outputs.as_slice(), [ScalarOutput { ty, .. }] if ty == element)
                                && super::anonymous_callable::closed_scalar_signature(&signature) => Some(signature),
                        _ => None,
                    }
                });
                if signature.is_none() {
                    diagnostics.push(module_diagnostic(module, "B0003",
                        "generated fixed array requires a closed matching scalar u64 factory and exact fixed count", *span));
                    ScalarType::Error
                } else {
                    inference.record_expression_type(count, &ScalarType::U64, true);
                    inference.record_expression_type(&binding.value, &binding.declared_type, true);
                    binding.declared_type.clone()
                }
            } else { expression_type_in_module_expected(
                &binding.value,
                Some(widening.as_ref().unwrap_or(&binding.declared_type)),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            ) };
            validate_output_receivers_in_module(
                &binding.value,
                &binding.receivers,
                binding.span,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
            );
            expect_module_type(
                module,
                widening.as_ref().or_else(|| super::widened_fixed_copy(binding, &actual).map(|_| &actual))
                    .unwrap_or(&binding.declared_type),
                &actual,
                binding.span,
                diagnostics,
            );
            TailConditionalTyping::Module { module, modules }
                .validate_secondary_binding_initializers(
                    binding,
                    scope,
                    visible_names,
                    folded_names,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
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
            overloads,
            inference,
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
            overloads,
            inference,
        ),
    }
}

fn assignment_type_in_module(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let expected = assignment
        .targets
        .iter()
        .map(|target| {
            assignment_target_type_in_module(
                target,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
                inference,
            )
        })
        .collect::<Vec<_>>();
    validate_assignment_target_distinctness_in_module(assignment, scope, module, diagnostics);
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
                overloads,
                inference,
            );
            match call_output_sequence_in_module(value, scope, module, modules, overloads) {
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
        inference.record_assignment_outputs(&module.source, assignment, &outputs);
        if assignment.targets.len() > outputs.outputs.len() {
            diagnostics.push(module_diagnostic(
                module,
                "B0004",
                "call has fewer outputs than assignment targets",
                assignment.span,
            ));
        }
        let mut inferred_names = None;
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
                validate_identifier_style(&module.source, name, *span, diagnostics);
                let (names, folded) = inferred_names
                    .get_or_insert_with(|| (visible_names.clone(), folded_names.clone()));
                if declare_module_name(
                    module,
                    name,
                    *span,
                    names,
                    folded,
                    diagnostics,
                ) {
                    scope.insert(name.clone(), output.ty.clone());
                    inference.record_place_type(&target.place, &output.ty);
                }
            }
        }
    }
    ScalarType::Unit
}

fn assignment_target_type_in_module(
    target: &ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> Option<ScalarType> {
    if let Some(receiver) = &target.receiver {
        if let Some(namespace) = module
            .namespace_bindings
            .iter()
            .find(|namespace| namespace.binding == *receiver)
        {
            let Some(target_module) = modules
                .iter()
                .find(|candidate| candidate.source == &namespace.target)
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
            inference.record_place_type(&target.place, &expected);
            if let ScalarPlace::Field { base, .. } = &target.place {
                if matches!(base.as_ref(), ScalarPlace::Name { .. }) {
                    inference.record_resolved_namespace_place(base, target_module.source);
                }
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
            if let Some(ty) = &expected {
                inference.record_place_type(&target.place, ty);
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
            overloads,
            inference,
        )),
    }
}

fn validate_assignment_target_distinctness_in_module(
    assignment: &ScalarAssignment,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (span, message) in assignment_target_distinctness(assignment, scope) {
        diagnostics.push(module_diagnostic(module, "B0002", message, span));
    }
}

fn widened_local_literal_type(binding: &ScalarBinding) -> Option<ScalarType> {
    let (element, values, span) = super::widened_local_literal(binding).or_else(|| super::guarded_runtime_literal(binding))?;
    Some(ScalarType::Array {
        element: Box::new(element.clone()), length: values.len() as u64,
        length_span: span, span,
    })
}

fn array_len_address<'a>(
    expression: &ScalarExpression,
    arguments: &'a [ScalarExpression],
    scope: &BTreeMap<String, ScalarType>,
) -> Option<&'a ScalarExpression> {
    let ScalarExpression::Call { type_arguments, .. } = expression else { return None };
    let [address @ ScalarExpression::CheckedAddress {
        mutability: ScalarReferenceMutability::Shared, place, ..
    }] = arguments else { return None };
    if !type_arguments.is_empty() { return None; }
    match place {
        ScalarPlace::Name { .. } => Some(address),
        ScalarPlace::Dereference { pointer, .. } => {
            let ScalarExpression::Name { name, .. } = pointer.as_ref() else { return None };
            // Runtime views must acquire an exact extent transport after normal
            // checked reference analysis; a source pointer alone proves no count.
            matches!(scope.get(name), Some(ScalarType::CheckedReference { inner, .. })
                if matches!(inner.as_ref(), ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }))
                .then_some(address)
        }
        _ => None,
    }
}

fn is_array_len_operand(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::CheckedReference { inner, .. }
        if matches!(inner.as_ref(), ScalarType::Array { .. } | ScalarType::RuntimeArray { .. }))
}

fn type_core_memory_in_module(
    expression: &ScalarExpression,
    operation: &str,
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let diagnostics_before_call = diagnostics.len();
    if matches!(operation, "alloc" | "free" | "invalidate" | "rebind") && !unsafe_context {
        diagnostics.push(module_diagnostic(
            module,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    let (parameters, result) = match operation {
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
            let mut parameters = Vec::with_capacity(2);
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
                    overloads,
                    inference,
                );
                expect_module_type(
                    module,
                    &ScalarType::U64,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
                parameters.push(actual);
            }
            (parameters, ScalarType::RawPointer(Box::new(ScalarType::U8)))
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
                overloads,
                inference,
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
            (vec![actual], ScalarType::Unit)
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
                overloads,
                inference,
            );
            if !is_error_type(&actual) && !matches!(actual, ScalarType::CheckedReference { .. }) {
                diagnostics.push(module_diagnostic(
                    module,
                    "B0003",
                    &format!("core.{operation} requires a checked reference to an allocation"),
                    expression_span(address),
                ));
            }
            let parameters = if operation == "rebind" {
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
                );
                expect_module_type(
                    module,
                    &ScalarType::U64,
                    &length_type,
                    expression_span(length),
                    diagnostics,
                );
                vec![actual, raw_type, length_type]
            } else {
                vec![actual]
            };
            (parameters, ScalarType::Unit)
        }
        "array_len" => {
            let Some(address) = array_len_address(expression, arguments, scope) else {
                diagnostics.push(module_diagnostic(module, "B0003",
                    "core.array_len requires the shared address of a named array place", span));
                return ScalarType::Error;
            };
            let actual = expression_type_in_module(address, scope, visible_names, folded_names, module, modules, diagnostics, unsafe_context, overloads, inference);
            if !is_array_len_operand(&actual) {
                if !is_error_type(&actual) {
                    diagnostics.push(module_diagnostic(module, "B0003",
                        "core.array_len requires the shared address of a named array place", span));
                }
                return ScalarType::Error;
            }
            (vec![actual], ScalarType::U64)
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
            (Vec::new(), ScalarType::Unit)
        }
        _ => unreachable!(),
    };
    if diagnostics.len() == diagnostics_before_call {
        inference.record_core_call_signature(expression, parameters, &result, span);
    }
    result
}

fn type_core_memory(
    expression: &ScalarExpression,
    operation: &str,
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let diagnostics_before_call = diagnostics.len();
    if matches!(operation, "alloc" | "free" | "invalidate" | "rebind") && !unsafe_context {
        diagnostics.push(diagnostic(
            program,
            "B0012",
            &format!("core.{operation} requires an unsafe block"),
            span,
        ));
    }
    let (parameters, result) = match operation {
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
            let mut parameters = Vec::with_capacity(2);
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
                    overloads,
                    inference,
                );
                expect_type(
                    program,
                    &ScalarType::U64,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
                parameters.push(actual);
            }
            (parameters, ScalarType::RawPointer(Box::new(ScalarType::U8)))
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
                overloads,
                inference,
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
            (vec![actual], ScalarType::Unit)
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
                overloads,
                inference,
            );
            if !is_error_type(&actual) && !matches!(actual, ScalarType::CheckedReference { .. }) {
                diagnostics.push(diagnostic(
                    program,
                    "B0003",
                    &format!("core.{operation} requires a checked reference to an allocation"),
                    expression_span(address),
                ));
            }
            let parameters = if operation == "rebind" {
                let raw = &arguments[1];
                let raw_type = expression_type(
                    raw,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
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
                    overloads,
                    inference,
                );
                expect_type(
                    program,
                    &ScalarType::U64,
                    &length_type,
                    expression_span(length),
                    diagnostics,
                );
                vec![actual, raw_type, length_type]
            } else {
                vec![actual]
            };
            (parameters, ScalarType::Unit)
        }
        "array_len" => {
            let Some(address) = array_len_address(expression, arguments, scope) else {
                diagnostics.push(diagnostic(program, "B0003",
                    "core.array_len requires the shared address of a named array place", span));
                return ScalarType::Error;
            };
            let actual = expression_type(address, scope, visible_names, folded_names, program, diagnostics, unsafe_context, overloads, inference);
            if !is_array_len_operand(&actual) {
                if !is_error_type(&actual) {
                    diagnostics.push(diagnostic(program, "B0003",
                        "core.array_len requires the shared address of a named array place", span));
                }
                return ScalarType::Error;
            }
            (vec![actual], ScalarType::U64)
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
            (Vec::new(), ScalarType::Unit)
        }
        _ => unreachable!(),
    };
    if diagnostics.len() == diagnostics_before_call {
        inference.record_core_call_signature(expression, parameters, &result, span);
    }
    result
}

fn type_core_int_conversion_in_module(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
        if diagnostics.len() == diagnostics_before_call {
            inference.record_core_call_signature(expression, vec![actual], destination, span);
        }
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
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
    if diagnostics.len() == diagnostics_before_call {
        inference.record_core_call_signature(expression, vec![actual], destination, span);
    }
    destination.clone()
}

fn type_core_pointer_cast_in_module(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
    if diagnostics.len() == diagnostics_before_call {
        inference.record_core_call_signature(expression, vec![actual], destination, span);
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
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
        if diagnostics.len() == diagnostics_before_call {
            inference.record_core_call_signature(expression, vec![actual], destination, span);
        }
        destination.clone()
    }
}

fn type_core_float_conversion(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
    let diagnostics_before_call = diagnostics.len();
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
            overloads,
            inference,
        ),
        None => expression_type(
            &arguments[0],
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
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
        if diagnostics.len() == diagnostics_before_call {
            inference.record_core_call_signature(expression, vec![actual], destination, span);
        }
        destination.clone()
    }
}

fn type_core_int_conversion(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
    let diagnostics_before_call = diagnostics.len();
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
            overloads,
            inference,
        ),
        None => expression_type(
            &arguments[0],
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
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
        if diagnostics.len() == diagnostics_before_call {
            inference.record_core_call_signature(expression, vec![actual], destination, span);
        }
        destination.clone()
    }
}

fn type_core_bitcast(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
    let diagnostics_before_call = diagnostics.len();
    let actual = expression_type(
        &arguments[0],
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
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
    if diagnostics.len() == diagnostics_before_call {
        inference.record_core_call_signature(expression, vec![actual], destination, span);
    }
    destination.clone()
}

fn type_core_pointer_cast(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
    if diagnostics.len() == diagnostics_before_call {
        inference.record_core_call_signature(expression, vec![actual], destination, span);
    }
    destination.clone()
}
fn type_core_raw_memory_in_module(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
    let count = if operation == "offset" {
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
            overloads,
            inference,
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
        Some(count)
    } else {
        None
    };
    if !valid || is_error_type(&pointer) || is_error_type(pointee) {
        return ScalarType::Error;
    }
    let result = if operation == "offset" {
        ScalarType::RawPointer(Box::new(pointee.clone()))
    } else {
        pointee.clone()
    };
    if diagnostics.len() == diagnostics_before_call {
        let parameters = match count {
            Some(count) => vec![pointer, count],
            None => vec![pointer],
        };
        inference.record_core_call_signature(expression, parameters, &result, span);
    }
    result
}

fn type_core_raw_memory(
    expression: &ScalarExpression,
    operation: &str,
    type_arguments: &[ScalarTypeArgument],
    arguments: &[ScalarExpression],
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let diagnostics_before_call = diagnostics.len();
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
        overloads,
        inference,
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
    let count = if operation == "offset" {
        let count = expression_type_expected(
            &arguments[1],
            &ScalarType::I64,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
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
        Some(count)
    } else {
        None
    };
    if !valid || is_error_type(&pointer) || is_error_type(pointee) {
        return ScalarType::Error;
    }
    let result = if operation == "offset" {
        ScalarType::RawPointer(Box::new(pointee.clone()))
    } else {
        pointee.clone()
    };
    if diagnostics.len() == diagnostics_before_call {
        let parameters = match count {
            Some(count) => vec![pointer, count],
            None => vec![pointer],
        };
        inference.record_core_call_signature(expression, parameters, &result, span);
    }
    result
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

fn is_float_arithmetic_operator(operator: &BinaryOperator) -> bool {
    matches!(operator, BinaryOperator::Add | BinaryOperator::Subtract | BinaryOperator::Multiply
        | BinaryOperator::Divide | BinaryOperator::Remainder)
}

// Leaves and obvious non-float shapes need no cache access. Check both
// children shallowly before following a composite, so ordinary arithmetic
// spines with a named or integer operand have no recursive shape walk.
fn is_homogeneous_bitwise(operator: &BinaryOperator) -> bool {
    matches!(
        operator,
        BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
    )
}

fn is_integer_literal_operand(expression: &ScalarExpression) -> bool {
    match expression {
        ScalarExpression::Integer { .. } => true,
        ScalarExpression::Unary {
            operator: UnaryOperator::Negate | UnaryOperator::BitwiseNot,
            operand,
            ..
        } => matches!(operand.as_ref(), ScalarExpression::Integer { .. }),
        _ => false,
    }
}

fn infer_bitwise_operands(
    left: &ScalarExpression,
    right: &ScalarExpression,
    mut check: impl FnMut(&ScalarExpression, Option<&ScalarType>) -> ScalarType,
) -> (ScalarType, ScalarType) {
    let left_literal = is_integer_literal_operand(left);
    let right_literal = is_integer_literal_operand(right);
    if left_literal && !right_literal {
        let right_type = check(right, None);
        let left_type = check(left, is_integer_type(&right_type).then_some(&right_type));
        (left_type, right_type)
    } else {
        let left_type = check(left, None);
        let right_type = check(
            right,
            (!left_literal && right_literal && is_integer_type(&left_type)).then_some(&left_type),
        );
        (left_type, right_type)
    }
}

fn float_literal_leaf(expression: &ScalarExpression) -> Option<bool> {
    match expression {
        ScalarExpression::Float { .. } => Some(true),
        ScalarExpression::Unary { operator: UnaryOperator::Negate, .. } => None,
        ScalarExpression::Binary { operator, .. } if is_float_arithmetic_operator(operator) => None,
        _ => Some(false),
    }
}

fn is_float_literal_arithmetic(
    expression: &ScalarExpression,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> bool {
    if let Some(shape) = float_literal_leaf(expression) {
        return shape;
    }
    match expression {
        ScalarExpression::Unary { operand, .. } => {
            if let Some(shape) = float_literal_leaf(operand) {
                return shape;
            }
            inference.memoized_float_literal_shape(expression, |inference| {
                is_float_literal_arithmetic(operand, inference)
            })
        }
        ScalarExpression::Binary { left, right, .. } => {
            match (float_literal_leaf(left), float_literal_leaf(right)) {
                (Some(false), _) | (_, Some(false)) => return false,
                (Some(true), Some(true)) => return true,
                _ => {}
            }
            inference.memoized_float_literal_shape(expression, |inference| {
                is_float_literal_arithmetic(left, inference)
                    && is_float_literal_arithmetic(right, inference)
            })
        }
        _ => unreachable!("only composite float literal candidates reach the shape walk"),
    }
}

fn float_literal_arithmetic_operands(
    operator: &BinaryOperator,
    left: &ScalarExpression,
    right: &ScalarExpression,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> (bool, bool) {
    if !is_float_arithmetic_operator(operator) {
        return (false, false);
    }
    (
        is_float_literal_arithmetic(left, inference),
        is_float_literal_arithmetic(right, inference),
    )
}

// The sibling fixes the width: contextual literals do not choose a numeric
// type or convert a typed operand. Check each operand only once.
fn infer_float_literal_arithmetic(
    literal_operands: (bool, bool),
    left: &ScalarExpression,
    right: &ScalarExpression,
    mut check: impl FnMut(&ScalarExpression, Option<&ScalarType>) -> ScalarType,
) -> Option<Result<ScalarType, ()>> {
    let (literal, sibling) = match literal_operands {
        (true, false) => (left, right),
        (false, true) => (right, left),
        (true, true) => return Some(Err(())),
        (false, false) => return None,
    };
    let ty = check(sibling, None);
    Some(if is_error_type(&ty) {
        Ok(ty)
    } else if is_float_type(&ty) {
        Ok(check(literal, Some(&ty)))
    } else {
        Err(())
    })
}

fn required_binary_arithmetic_type<'a>(
    expected: Option<&'a ScalarType>,
    left: &ScalarType,
) -> Option<&'a ScalarType> {
    expected.or_else(|| (!is_float_type(left)).then_some(&ScalarType::I32))
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = check_expression_type_in_module(
        expression,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
    );
    inference.record_expression_type(expression, &ty, false);
    ty
}

fn check_expression_type_in_module(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    match expression {
        ScalarExpression::GeneratedArray { span, .. } => {
            diagnostics.push(module_diagnostic(module, "B0003", "generated array requires an exact prepared fixed array owner", *span));
            ScalarType::Error
        }
        ScalarExpression::AnonymousCallable { span, .. } => {
            diagnostics.push(module_diagnostic(module, "B0003", "inline callable lacks its prepared fixed array owner", *span));
            ScalarType::Error
        }
        ScalarExpression::ValueField { receiver, name, name_span, span } => {
            if !matches!(receiver.as_ref(), ScalarExpression::Call { .. }) {
                diagnostics.push(module_diagnostic(module, "B0003", "field value receiver must be a call result", *span));
                return ScalarType::Error;
            }
            let ty = expression_type_in_module(
                receiver, scope, visible_names, folded_names,
                module, modules, diagnostics, unsafe_context, overloads, inference,
            );
            if is_error_type(&ty) {
                return ScalarType::Error;
            }
            match checked_value_field(&ty, name, |id| modules.get(overloads.module_ordinal_for_source(&id.source)?)?.structs.get(id.index)) {
                Ok(field) => {
                    inference.record_value_field(expression, &field.id);
                    field.ty.clone()
                }
                Err(message) => {
                    diagnostics.push(module_diagnostic(module, "B0003", message, *name_span));
                    ScalarType::Error
                }
            }
        }
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
            let ty = check_place_type_in_module(
                place,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
                inference,
                PlaceTypeMode::RawAddress,
            );
            inference.record_place_type(place, &ty);
            ScalarType::RawPointer(Box::new(ty))
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
            overloads,
            inference,
        ),
        ScalarExpression::Dereference { place, span } => checked_dereference_type_in_module(
            place,
            *span,
            scope,
            module,
            modules,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
        ),
        ScalarExpression::IndexedRead { place, .. } => {
            let ty = check_place_type_in_module(
                place, scope, module, modules, diagnostics, overloads, inference,
                PlaceTypeMode::IndexedRead { unsafe_context },
            );
            inference.record_place_type(place, &ty);
            ty
        },
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
                let Some(tag) = *enum_tag else {
                    diagnostics.push(module_diagnostic(
                        module,
                        "M0002",
                        "unknown enum variant",
                        *name_span,
                    ));
                    return ScalarType::Error;
                };
                inference.record_member(
                    expression,
                    CheckedMember::Enum {
                        enumeration: enum_id.clone(),
                        tag,
                    },
                );
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
                    let Some(field) = member_field_in_module(
                        receiver_type,
                        name,
                        *name_span,
                        *span,
                        module,
                        modules,
                        diagnostics,
                    ) else {
                        return ScalarType::Error;
                    };
                    inference.record_member(
                        expression,
                        if matches!(receiver_type, ScalarType::Struct(_)) {
                            CheckedMember::DirectField(field.id.clone())
                        } else {
                            CheckedMember::RawPointerField(field.id.clone())
                        },
                    );
                    return field.ty.clone();
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
                .find(|candidate| candidate.source == &namespace.target)
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
            inference.record_member(expression, CheckedMember::Module(target.source.clone()));
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
                    let diagnostics_before_range = diagnostics.len();
                    validate_integer_range(module, &value, *span, diagnostics);
                    if diagnostics.len() == diagnostics_before_range {
                        inference.record_folded_signed_integer_magnitude(operand);
                    }
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
                overloads,
                inference,
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
                    if !is_float_type(&operand_type) {
                        expect_module_integer(module, &operand_type, *span, diagnostics);
                    }
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
            if is_homogeneous_bitwise(operator) {
                return check_expression_type_in_module_expected(
                    expression, None, scope, visible_names, folded_names, module, modules,
                    diagnostics, unsafe_context, overloads, inference,
                );
            }
            let literal_operands = float_literal_arithmetic_operands(operator, left, right, inference);
            if let Some(inferred) = infer_float_literal_arithmetic(literal_operands, left, right, |operand, expected| {
                expression_type_in_module_expected(
                    operand, expected, scope, visible_names, folded_names, module, modules, diagnostics, unsafe_context, overloads, inference,
                )
            }) {
                return inferred.unwrap_or_else(|()| {
                    diagnostics.push(module_diagnostic(
                        module, "B0003", "floating-point arithmetic requires a matching f32 or f64 context", *span,
                    ));
                    ScalarType::Error
                });
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
                    overloads,
                    inference,
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
                overloads,
                inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                let required = required_binary_arithmetic_type(None, &left_type);
                if let Some(required) = required {
                    expect_module_type(module, required, &left_type, *span, diagnostics);
                }
                expect_module_type(module, &left_type, &right_type, *span, diagnostics);
                required.cloned().unwrap_or(left_type)
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
                let result = ScalarType::ArtifactId;
                inference.record_core_call_signature(expression, Vec::new(), &result, *span);
                return result;
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
                let diagnostics_before_call = diagnostics.len();
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
                    overloads,
                    inference,
                );
                let expected = utf8_type_from_namespace_target(module, modules, *span, diagnostics);
                expect_module_type(module, &expected, &actual, *span, diagnostics);
                return if is_error_type(&actual) {
                    ScalarType::Error
                } else {
                    let result = ScalarType::ArtifactId;
                    if diagnostics.len() == diagnostics_before_call {
                        inference.record_core_call_signature(
                            expression,
                            vec![actual],
                            &result,
                            *span,
                        );
                    }
                    result
                };
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "alloc" | "free" | "invalidate" | "rebind" | "array_len" | "system_panic"
                )
            {
                return type_core_memory_in_module(
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return type_core_int_conversion_in_module(
                    expression,
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
                    overloads,
                    inference,
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
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return type_core_bitcast_in_module(
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core") && matches!(name.as_str(), "offset" | "load") {
                return type_core_raw_memory_in_module(
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return type_core_pointer_cast_in_module(
                    expression,
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
                    overloads,
                    inference,
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
                    let diagnostics_before_arguments = diagnostics.len();
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
                            overloads,
                            inference,
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
                        if diagnostics.len() == diagnostics_before_arguments {
                            inference.record_call_signature(expression, parameters, outputs, None);
                        }
                        scalar_call_result_in_module(outputs)
                    };
                }
            }
            let overload = overloads.for_call(&module.source, receiver.as_deref(), name);
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
                    overloads,
                    inference,
                );
            }
            let (callable, target, unsafe_callable, extern_callable, target_view) = match receiver {
                None => (scope.get(name), module, false, false, *overloads),
                Some(binding) => {
                    let namespace = module
                        .namespace_bindings
                        .iter()
                        .find(|namespace| namespace.binding == *binding);
                    if let Some(namespace) = namespace {
                        let Some(target) = modules
                            .iter()
                            .find(|module| module.source == &namespace.target)
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
                            false,
                            overloads.imported(&target.source),
                        )
                    } else {
                        let Some(function) = overloads
                            .extern_member(Some(binding), name)
                            .filter(|member| member.first_declaration)
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
                            *overloads,
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
            let generic_parameters = if extern_callable {
                None
            } else {
                target_view.generics(name)
            };
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
            let diagnostics_before_arguments = diagnostics.len();
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
                    overloads,
                    inference,
                );
                let fixed_copy = !unsafe_callable && parameters.len() == 1 && type_arguments.is_empty()
                    && matches!(argument, ScalarExpression::Name { .. })
                    && super::fixed_copy_owning_signature(&parameters, &outputs)
                    && super::fixed_copy_to_runtime(parameter, &actual).is_some();
                if !fixed_copy {
                    expect_module_type(target, parameter, &actual, *span, diagnostics);
                }
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
                if diagnostics.len() == diagnostics_before_arguments {
                    inference.record_call_signature(expression, parameters, outputs, None);
                }
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
            overloads,
            inference,
        ),
    }
}

pub(super) fn expression_type_in_module_expected(
    expression: &ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = check_expression_type_in_module_expected(
        expression,
        expected,
        scope,
        visible_names,
        folded_names,
        module,
        modules,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
    );
    inference.record_expression_type(expression, &ty, expected.is_some());
    ty
}

fn check_expression_type_in_module_expected(
    expression: &ScalarExpression,
    expected: Option<&ScalarType>,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
        let overload = overloads.for_call(&module.source, receiver.as_deref(), name);
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
            let diagnostics_before_arguments = diagnostics.len();
            let argument_types = inference.without_checked_occurrences(|inference| {
                arguments
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
                            overloads,
                            inference,
                        )
                    })
                    .collect()
            });
            let overload_arguments = arguments
                .iter()
                .zip(&argument_types)
                .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                .collect::<Vec<_>>();
            let (callable, selection) =
                match resolve_overload_candidate(overload, &overload_arguments, expected) {
                    Ok(resolved) => resolved,
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
                    overloads,
                    inference,
                );
                expect_module_type(
                    module,
                    parameter,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            if diagnostics.len() == diagnostics_before_arguments {
                inference.record_call_signature(
                    expression,
                    &parameters,
                    &outputs,
                    Some((selection, overload.origin)),
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
        if let Some(expected) = expected.filter(|ty| is_float_type(ty)) {
            let actual = expression_type_in_module_expected(
                operand,
                Some(expected),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            );
            expect_module_type(module, expected, &actual, *span, diagnostics);
            return actual;
        }
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
                let diagnostics_before_range = diagnostics.len();
                validate_integer_range_for_type(module, &value, *span, expected, diagnostics);
                if diagnostics.len() == diagnostics_before_range {
                    inference.record_folded_signed_integer_magnitude(operand);
                }
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
            overloads,
            inference,
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
            overloads,
            inference,
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
            overloads,
            inference,
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
    if let ScalarExpression::AnonymousCallable { .. } = expression {
        if inference.is_nominated_inline_element(expression) {
            if let Some(signature) = expected.filter(|ty| super::anonymous_callable::closed_scalar_signature(ty)) {
                return signature.clone();
            }
        }
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
                overloads,
                inference,
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
        if structure.copy_policy == CopyPolicy::Copy {
            inference.record_copy_struct_literal(expression);
        }
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
            let prior_diagnostics = diagnostics.len();
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
                overloads,
                inference,
            );
            expect_module_type(module, &declared.ty, &actual, field.span, diagnostics);
            if diagnostics.len() == prior_diagnostics && scalar_type_equal(&declared.ty, &actual) {
                inference.record_literal_field(&field.value, &declared.id);
            }
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
            overloads,
            inference,
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
        if arithmetic_context.is_none()
            && float_literal_arithmetic_operands(operator, left, right, inference) != (false, false)
        {
            return expression_type_in_module(
                expression, scope, visible_names, folded_names, module, modules, diagnostics,
                unsafe_context, overloads, inference,
            );
        }
        let bool_context = ScalarType::Bool;
        let (left_type, right_type) = if bitwise && arithmetic_context.is_none() {
            infer_bitwise_operands(left, right, |operand, expected| {
                expression_type_in_module_expected(
                    operand, expected, scope, visible_names, folded_names, module, modules,
                    diagnostics, unsafe_context, overloads, inference,
                )
            })
        } else if comparison && is_null_expression(left) {
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
        let required = required_binary_arithmetic_type(arithmetic_context, &left_type);
        if let Some(required) = required {
            expect_module_type(module, required, &left_type, *span, diagnostics);
        }
        expect_module_type(module, &left_type, &right_type, *span, diagnostics);
        return required.cloned().unwrap_or(left_type);
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
        overloads,
        inference,
    )
}

fn utf8_type_from_namespace_target(
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> ScalarType {
    let structure = module
        .namespace_bindings
        .iter()
        .filter_map(|binding| {
            modules
                .iter()
                .find(|candidate| candidate.source == &binding.target)
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
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
                .find(|candidate| candidate.source == &binding.target)
        })
        .flat_map(|target| target.structs.iter())
        .find(|structure| structure.id == *expected_id && structure.name == "utf8")
        .map(|structure| ScalarType::Struct(structure.id.clone()))
}

fn checked_value_field<'a>(
    receiver: &ScalarType,
    name: &str,
    lookup: impl FnOnce(&ScalarStructId) -> Option<&'a ScalarStruct>,
) -> Result<&'a ScalarStructField, &'static str> {
    let ScalarType::Struct(id) = receiver else {
        return Err("field value receiver must be a copy struct");
    };
    let structure = lookup(id)
        .filter(|structure| &structure.id == id && structure.copy_policy == CopyPolicy::Copy)
        .ok_or("field value receiver must be a copy struct")?;
    let field = structure.fields.iter().find(|field| field.name == name)
        .ok_or("unknown struct field")?;
    if super::reference::aggregate::ordinary_value(&field.ty) {
        Ok(field)
    } else {
        Err("field value must be an ordinary copy scalar")
    }
}

fn member_field_in_module<'a>(
    receiver: &ScalarType,
    name: &str,
    name_span: ByteSpan,
    span: ByteSpan,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'a>],
    diagnostics: &mut Vec<super::Diagnostic>,
) -> Option<&'a ScalarStructField> {
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
                return None;
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
        return None;
    };
    let Some(field) = structure.fields.iter().find(|field| field.name == name) else {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "value has no field",
            name_span,
        ));
        return None;
    };
    Some(field)
}

fn is_supported_checked_address_place(place: &ScalarPlace) -> bool {
    match place {
        ScalarPlace::Name { .. } => true,
        ScalarPlace::Field { base, .. } => matches!(base.as_ref(), ScalarPlace::Name { .. }),
        ScalarPlace::Dereference { .. } => true,
        ScalarPlace::Index { base, .. } => is_supported_checked_address_place(base),
    }
}

fn permits_named_checked_field_borrow(
    place: &ScalarPlace,
    mode: ScalarReferenceMutability,
    scope: &BTreeMap<String, ScalarType>,
) -> bool {
    let ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(_), .. } = place else {
        return false;
    };
    let ScalarPlace::Dereference { pointer, .. } = base.as_ref() else { return false };
    let ScalarExpression::Name { name, .. } = pointer.as_ref() else { return false };
    matches!(scope.get(name), Some(ScalarType::CheckedReference {
        mutability, inner,
    }) if (mode == ScalarReferenceMutability::Shared || *mutability == ScalarReferenceMutability::Mutable)
        && matches!(inner.as_ref(), ScalarType::Struct(_)))
}

fn named_checked_fixed_projection_index<'a>(
    place: &'a ScalarPlace,
    mode: ScalarReferenceMutability,
    scope: &BTreeMap<String, ScalarType>,
) -> Option<(&'a ScalarPlace, &'a ScalarExpression)> {
    let ScalarPlace::Index { base, index, .. } = place else { return None };
    let direct = matches!(base.as_ref(), ScalarPlace::Dereference { pointer, .. }
        if matches!(pointer.as_ref(), ScalarExpression::Name { name, .. }
            if matches!(scope.get(name), Some(ScalarType::CheckedReference { mutability, inner })
                if (mode == ScalarReferenceMutability::Shared || *mutability == ScalarReferenceMutability::Mutable)
                    && matches!(inner.as_ref(), ScalarType::Array { .. }))));
    (direct || permits_named_checked_field_borrow(base, mode, scope)).then_some((base, index))
}

fn named_checked_runtime_projection_index<'a>(
    place: &'a ScalarPlace,
    mode: ScalarReferenceMutability,
    scope: &BTreeMap<String, ScalarType>,
) -> Option<(&'a ScalarPlace, &'a ScalarExpression)> {
    let ScalarPlace::Index { base, index, .. } = place else { return None; };
    let ScalarPlace::Dereference { pointer, .. } = base.as_ref() else { return None; };
    let ScalarExpression::Name { name, .. } = pointer.as_ref() else { return None; };
    let ScalarType::CheckedReference { mutability, inner } = scope.get(name)? else { return None; };
    ((mode == ScalarReferenceMutability::Shared || *mutability == ScalarReferenceMutability::Mutable)
        && matches!(inner.as_ref(), ScalarType::RuntimeArray { element, .. }
            if super::reference::aggregate::ordinary_value(element)))
        .then_some((base, index))
}

fn checked_projected_element(
    place: &ScalarPlace,
    index: &ScalarExpression,
    index_type: ScalarType,
    base: ScalarType,
    mutability: ScalarReferenceMutability,
    inference: &mut ScalarInferenceBuilder<'_>,
    diagnose: impl FnOnce(&str),
) -> ScalarType {
    match base {
        ScalarType::Array { element, length, .. }
            if super::reference::aggregate::ordinary_value(&element)
                && index_type == ScalarType::U64
                && match index {
                    ScalarExpression::Integer { value, .. } => value >= &0.into() && value < &length.into(),
                    _ => true, // The prepared borrow checker must prove a concrete in-range value.
                } =>
        {
            inference.record_place_type(place, &element);
            ScalarType::CheckedReference { mutability, inner: element }
        }
        ScalarType::Error => ScalarType::Error,
        _ => {
            diagnose("checked projected borrow requires an in-range index into a fixed ordinary scalar array");
            ScalarType::Error
        }
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    if let Some((base, index)) = named_checked_fixed_projection_index(place, mutability, scope) {
        let array = checked_dereference_type(base, span, scope, program, diagnostics, false, overloads, inference);
        let index_type = expression_type_expected(index, &ScalarType::U64, scope,
            &BTreeSet::new(), &BTreeMap::new(), program, diagnostics, false, overloads, inference);
        return checked_projected_element(place, index, index_type, array, mutability, inference,
            |message| diagnostics.push(diagnostic(program, "B0003", message, span)));
    }
    if permits_named_checked_field_borrow(place, mutability, scope) {
        let inner = checked_dereference_type(place, span, scope, program, diagnostics, false, overloads, inference);
        if super::reference::aggregate::ordinary_value(&inner) {
            return ScalarType::CheckedReference { mutability, inner: Box::new(inner) };
        }
        if inner != ScalarType::Error {
            diagnostics.push(diagnostic(program, "B0003",
                "checked projected borrow requires an ordinary Copy field", span));
        }
        return ScalarType::Error;
    }
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
            overloads,
            inference,
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
        place_type(place, scope, program, diagnostics, overloads, inference)
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
    if matches!(place, ScalarPlace::Dereference { .. }) {
        inference.record_place_type(place, &inner);
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    if let Some((base, index)) = named_checked_runtime_projection_index(place, mutability, scope) {
        let array = checked_dereference_type_in_module(base, span, scope, module, modules,
            diagnostics, false, overloads, inference);
        let selector = expression_type_in_module_expected(index, Some(&ScalarType::U64), scope,
            &BTreeSet::new(), &BTreeMap::new(), module, modules, diagnostics, false, overloads, inference);
        if let ScalarType::RuntimeArray { element, .. } = array {
            if selector == ScalarType::U64 && matches!(index, ScalarExpression::Name { .. }) {
                inference.record_place_type(place, &element);
                return ScalarType::CheckedReference { mutability, inner: element };
            }
        }
        diagnostics.push(module_diagnostic(module, "B0003",
            "checked runtime element borrow requires an exact lexical u64 selector", span));
        return ScalarType::Error;
    }
    if let Some((base, index)) = named_checked_fixed_projection_index(place, mutability, scope) {
        let array = checked_dereference_type_in_module(base, span, scope, module, modules,
            diagnostics, false, overloads, inference);
        let index_type = expression_type_in_module_expected(index, Some(&ScalarType::U64), scope,
            &BTreeSet::new(), &BTreeMap::new(), module, modules, diagnostics, false, overloads, inference);
        return checked_projected_element(place, index, index_type, array, mutability, inference,
            |message| diagnostics.push(module_diagnostic(module, "B0003", message, span)));
    }
    if permits_named_checked_field_borrow(place, mutability, scope) {
        let inner = checked_dereference_type_in_module(place, span, scope, module, modules, diagnostics, false, overloads, inference);
        if super::reference::aggregate::ordinary_value(&inner) {
            return ScalarType::CheckedReference { mutability, inner: Box::new(inner) };
        }
        if inner != ScalarType::Error {
            diagnostics.push(module_diagnostic(module, "B0003",
                "checked projected borrow requires an ordinary Copy field", span));
        }
        return ScalarType::Error;
    }
    if let ScalarPlace::Field { base, field: ScalarFieldReference::Resolved(field), .. } = place {
        if matches!(base.as_ref(), ScalarPlace::Index { base, .. }
            if matches!(base.as_ref(), ScalarPlace::Name { name, .. }
                if matches!(scope.get(name), Some(ScalarType::RuntimeArray { element, .. })
                    if matches!(element.as_ref(), ScalarType::Struct(id) if id == &field.structure))))
        {
            let inner = place_type_in_module(place, scope, module, modules, diagnostics, overloads, inference);
            if super::reference::aggregate::ordinary_value(&inner) {
                return ScalarType::CheckedReference { mutability, inner: Box::new(inner) };
            }
            if inner != ScalarType::Error {
                diagnostics.push(module_diagnostic(module, "B0003",
                    "checked runtime field borrow requires an ordinary Copy field", span));
            }
            return ScalarType::Error;
        }
    }
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
            overloads,
            inference,
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
        place_type_in_module(
            place,
            scope,
            module,
            modules,
            diagnostics,
            overloads,
            inference,
        )
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
    if matches!(place, ScalarPlace::Dereference { .. }) {
        inference.record_place_type(place, &inner);
    }
    ScalarType::CheckedReference {
        mutability,
        inner: Box::new(inner),
    }
}

fn is_direct_fixed_array_element(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
) -> bool {
    let ScalarPlace::Index { base, .. } = place else {
        return false;
    };
    let ScalarPlace::Name { name, .. } = base.as_ref() else {
        return false;
    };
    !is_const_binding_name(name) && matches!(scope.get(name), Some(ScalarType::Array { .. }))
}

fn writable_place_type(
    place: &ScalarPlace,
    target_span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    if writable_place_contains_raw_dereference(
        place,
        scope,
        program,
        diagnostics,
        overloads,
        inference,
    ) {
        diagnostics.push(diagnostic(
            program,
            "B0003",
            "assignment through raw pointer dereference is not supported",
            target_span,
        ));
        return ScalarType::Error;
    }
    let ty = writable_path_type(place, scope, program, diagnostics, overloads, inference);
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. })
        && !(is_direct_fixed_array_element(place, scope)
            && copy_policy(&ty, program.structs, program.enums, &BTreeSet::new())
                == Ok(CopyPolicy::Copy))
    {
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    if writable_place_contains_raw_dereference_in_module(
        place,
        scope,
        module,
        modules,
        diagnostics,
        overloads,
        inference,
    ) {
        diagnostics.push(module_diagnostic(
            module,
            "B0003",
            "assignment through raw pointer dereference is not supported",
            target_span,
        ));
        return ScalarType::Error;
    }
    let ty = writable_path_type_in_module(
        place,
        scope,
        module,
        modules,
        diagnostics,
        overloads,
        inference,
    );
    if matches!(ty, ScalarType::Struct(_) | ScalarType::Array { .. })
        && !(is_direct_fixed_array_element(place, scope)
            && copy_policy_with(
                &ty,
                &BTreeSet::new(),
                &mut |id| modules.iter().find(|candidate| candidate.source == &id.source)
                    .and_then(|owner| owner.structs.iter().find(|structure| structure.id == *id))
                    .map(|structure| structure.copy_policy),
                &mut |id| modules.iter().find(|candidate| candidate.source == &id.source)
                    .and_then(|owner| owner.enums.iter().find(|enumeration| enumeration.id == *id))
                    .map(|enumeration| enumeration.copy_policy),
            ) == Ok(CopyPolicy::Copy))
    {
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> bool {
    match place {
        ScalarPlace::Name { .. } => false,
        ScalarPlace::Field { base, .. } | ScalarPlace::Index { base, .. } => {
            writable_place_contains_raw_dereference(
                base,
                scope,
                program,
                diagnostics,
                overloads,
                inference,
            )
        }
        ScalarPlace::Dereference { pointer, .. } => inference.probe_raw_dereference(|inference| {
            expression_type(
                pointer,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                true,
                overloads,
                inference,
            )
        }),
    }
}

fn writable_place_contains_raw_dereference_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
                overloads,
                inference,
            )
        }
        ScalarPlace::Dereference { pointer, .. } => inference.probe_raw_dereference(|inference| {
            expression_type_in_module(
                pointer,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                module,
                modules,
                diagnostics,
                true,
                overloads,
                inference,
            )
        }),
    }
}

fn writable_path_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = match place {
        ScalarPlace::Name { .. } => check_place_type(
            place,
            scope,
            program,
            diagnostics,
            overloads,
            inference,
            PlaceTypeMode::Ordinary,
        ),
        ScalarPlace::Dereference { pointer, span } => match expression_type(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            program,
            diagnostics,
            false,
            overloads,
            inference,
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
                writable_path_type(base, scope, program, diagnostics, overloads, inference)
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
            let base_type =
                writable_path_type(base, scope, program, diagnostics, overloads, inference);
            let index_type = expression_type_expected(
                index,
                &ScalarType::U64,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                false,
                overloads,
                inference,
            );
            expect_type(
                program,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::RuntimeArray { .. } if matches!(base.as_ref(), ScalarPlace::Dereference { .. }) => {
                    diagnostics.push(diagnostic(program, "B0003",
                        "checked runtime array write requires the extent of the dereferenced backing", *span));
                    ScalarType::Error
                }
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
    };
    inference.record_place_type(place, &ty);
    ty
}

fn writable_path_type_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = match place {
        ScalarPlace::Name { .. } => check_place_type_in_module(
            place,
            scope,
            module,
            modules,
            diagnostics,
            overloads,
            inference,
            PlaceTypeMode::Ordinary,
        ),
        ScalarPlace::Dereference { pointer, span } => match expression_type_in_module(
            pointer,
            scope,
            &BTreeSet::new(),
            &BTreeMap::new(),
            module,
            modules,
            diagnostics,
            false,
            overloads,
            inference,
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
            let ScalarType::Struct(structure) = writable_path_type_in_module(
                base,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
                inference,
            ) else {
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
            let base_type = writable_path_type_in_module(
                base,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
                inference,
            );
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
                overloads,
                inference,
            );
            expect_module_type(
                module,
                &ScalarType::U64,
                &index_type,
                *index_span,
                diagnostics,
            );
            match base_type {
                ScalarType::RuntimeArray { ref element, .. } if matches!(base.as_ref(), ScalarPlace::Dereference { .. })
                    && !inference.allows_private_runtime_write_candidate(base.as_ref())
                    && !inference.nominates_local_runtime_write(base, index, &index_type, element, scope) => {
                    diagnostics.push(module_diagnostic(module, "B0003",
                        "checked runtime array write requires the extent of the dereferenced backing", *span));
                    ScalarType::Error
                }
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
    };
    inference.record_place_type(place, &ty);
    ty
}

#[derive(Clone, Copy)]
enum PlaceTypeMode {
    Ordinary,
    RawAddress,
    IndexedRead { unsafe_context: bool },
}

impl PlaceTypeMode {
    fn projected(self) -> Self {
        // RawAddress historically authorizes only the outer dereference.
        match self {
            Self::RawAddress => Self::Ordinary,
            _ => self,
        }
    }

    fn unsafe_pointer_expression(self) -> bool {
        match self {
            Self::IndexedRead { unsafe_context } => unsafe_context,
            _ => true,
        }
    }
}

pub(super) fn place_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = check_place_type(
        place,
        scope,
        program,
        diagnostics,
        overloads,
        inference,
        PlaceTypeMode::Ordinary,
    );
    inference.record_place_type(place, &ty);
    ty
}

fn check_place_type(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
    mode: PlaceTypeMode,
) -> ScalarType {
    match place {
        ScalarPlace::Name { name, span } => scope.get(name).cloned().unwrap_or_else(|| {
            diagnostics.push(diagnostic(program, "B0001", "unknown name", *span));
            ScalarType::Error
        }),
        ScalarPlace::Field {
            base, field, span, ..
        } => {
            let base_type = check_place_type(
                base, scope, program, diagnostics, overloads, inference, mode.projected(),
            );
            inference.record_place_type(base, &base_type);
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
            mode.unsafe_pointer_expression(),
            overloads,
            inference,
        ) {
            ScalarType::RawPointer(_) if matches!(mode, PlaceTypeMode::IndexedRead { unsafe_context: false }) => {
                diagnostics.push(diagnostic(program, "B0012", "raw dereference requires an unsafe block", *span));
                ScalarType::Error
            }
            ScalarType::RawPointer(inner) => *inner,
            ScalarType::CheckedReference { inner, .. } if !matches!(mode, PlaceTypeMode::Ordinary) => *inner,
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
            let base_type = check_place_type(
                base, scope, program, diagnostics, overloads, inference, mode.projected(),
            );
            inference.record_place_type(base, &base_type);
            let index_type = expression_type_expected(
                index,
                &ScalarType::U64,
                scope,
                &BTreeSet::new(),
                &BTreeMap::new(),
                program,
                diagnostics,
                false,
                overloads,
                inference,
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
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = check_place_type_in_module(
        place,
        scope,
        module,
        modules,
        diagnostics,
        overloads,
        inference,
        PlaceTypeMode::Ordinary,
    );
    inference.record_place_type(place, &ty);
    ty
}

fn check_place_type_in_module(
    place: &ScalarPlace,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
    mode: PlaceTypeMode,
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
            mode.unsafe_pointer_expression(),
            overloads,
            inference,
        ) {
            ScalarType::RawPointer(_) if matches!(mode, PlaceTypeMode::IndexedRead { unsafe_context: false }) => {
                diagnostics.push(module_diagnostic(module, "B0012", "raw dereference requires an unsafe block", *span));
                ScalarType::Error
            }
            ScalarType::RawPointer(inner) => *inner,
            ScalarType::CheckedReference { inner, .. } if !matches!(mode, PlaceTypeMode::Ordinary) => *inner,
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
            let base_type = check_place_type_in_module(
                base,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
                inference,
                mode.projected(),
            );
            inference.record_place_type(base, &base_type);
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
            let base_type = check_place_type_in_module(
                base,
                scope,
                module,
                modules,
                diagnostics,
                overloads,
                inference,
                mode.projected(),
            );
            inference.record_place_type(base, &base_type);
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
                overloads,
                inference,
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    match expression_type(
        pointer,
        scope,
        &BTreeSet::new(),
        &BTreeMap::new(),
        program,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = match place {
        ScalarPlace::Dereference { pointer, span } => checked_dereference_target_type(
            pointer,
            *span,
            scope,
            program,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
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
                overloads,
                inference,
            );
            inference.record_place_type(base, &base_type);
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
    };
    inference.record_place_type(place, &ty);
    ty
}

fn checked_dereference_type_in_module(
    place: &ScalarPlace,
    span: ByteSpan,
    scope: &BTreeMap<String, ScalarType>,
    module: &ScalarModuleTyping<'_>,
    modules: &[ScalarModuleTyping<'_>],
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let (dereference, pointer, dereference_span, field) = match place {
        ScalarPlace::Dereference { pointer, span } => (place, pointer, *span, None),
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
            (
                base.as_ref(),
                pointer,
                *dereference_span,
                Some((field, *field_span)),
            )
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
        overloads,
        inference,
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
    inference.record_place_type(dereference, &inner);
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
        inference.record_place_type(place, &candidate.ty);
        candidate.ty
    }
}

pub(super) fn expect_module_type(
    module: &ScalarModuleTyping<'_>,
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
    module: &ScalarModuleTyping<'_>,
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
    module: &ScalarModuleTyping<'_>,
    code: &str,
    message: &str,
    span: ByteSpan,
) -> super::Diagnostic {
    source_diagnostic(&module.source, code, message, span)
}

fn is_guarded_struct_field_type(ty: &ScalarType) -> bool {
    matches!(ty, ScalarType::Bool | ScalarType::Char) || is_integer_type(ty) || is_float_type(ty)
}

fn guarded_numeric_rhs(ty: &ScalarType, expression: &ScalarExpression) -> bool {
    if !matches!(ty, ScalarType::U64 | ScalarType::I32 | ScalarType::Bool)
        || !super::fallible_composite_expression(expression)
    { return false; }
    let mut stack = vec![(expression, ty)];
    let mut calls = false;
    while let Some((node, ty)) = stack.pop() {
        match node {
            ScalarExpression::Name { .. } => {}
            ScalarExpression::Integer { .. } if *ty != ScalarType::Bool => {}
            ScalarExpression::Boolean { .. } if *ty == ScalarType::Bool => {}
            ScalarExpression::Call { type_arguments, .. } if type_arguments.is_empty() => calls = true,
            ScalarExpression::Unary { operator, operand, .. }
                if super::fallible_total_unary_bound(ty, operator).is_some() => stack.push((operand, ty)),
            ScalarExpression::Binary { operator, left, right, .. }
                if (*ty == ScalarType::Bool && matches!(operator, BinaryOperator::And | BinaryOperator::Or))
                    || (*ty != ScalarType::Bool && matches!(operator, BinaryOperator::BitAnd | BinaryOperator::BitOr | BinaryOperator::BitXor))
                    || *ty == ScalarType::U64 && *operator == BinaryOperator::Add =>
            {
                stack.push((right, ty));
                stack.push((left, ty));
            }
            ScalarExpression::If { condition, then_branch, else_branch, .. } => {
                for arm in [else_branch, then_branch] {
                    let [ScalarBlockItem::Expression(value)] = arm.items.as_slice() else { return false; };
                    if arm.unsafe_context { return false; }
                    stack.push((value, ty));
                }
                stack.push((condition, &ScalarType::Bool));
            }
            _ => return false,
        }
    }
    calls
}

pub(super) fn guarded_runtime_predicate(expression: &ScalarExpression) -> bool {
    let atom = |value: &ScalarExpression| matches!(value,
        ScalarExpression::Name { .. } | ScalarExpression::Integer { .. });
    match expression {
        ScalarExpression::Name { .. } | ScalarExpression::Boolean { .. } => true,
        ScalarExpression::Binary { operator: BinaryOperator::Equal | BinaryOperator::NotEqual,
            left, right, .. } => atom(left) && atom(right),
        _ => false,
    }
}

pub(super) fn guarded_runtime_rhs(expression: &ScalarExpression) -> bool {
    let mut nodes = vec![expression];
    while let Some(node) = nodes.pop() {
        match node {
            ScalarExpression::Call { arguments, type_arguments, .. }
                if type_arguments.is_empty()
                    && matches!(arguments.as_slice(), [ScalarExpression::Name { .. } | ScalarExpression::Integer { .. }]) => {},
            ScalarExpression::If { condition, then_branch, else_branch, .. }
                if guarded_runtime_predicate(condition) =>
            {
                for arm in [else_branch, then_branch] {
                    let [ScalarBlockItem::Expression(value)] = arm.items.as_slice() else { return false; };
                    if arm.unsafe_context { return false; }
                    nodes.push(value);
                }
            }
            _ => return false,
        }
    }
    true
}

fn guarded_inline_struct_literal(
    id: &ScalarStructId,
    fields: &[ScalarStructLiteralField],
    structs: &[ScalarStruct],
    scope: &BTreeMap<String, ScalarType>,
) -> bool {
    let mut pending = vec![(id, fields, false)];
    while let Some((id, fields, nested)) = pending.pop() {
        let Some(structure) = structs.get(id.index).filter(|structure| &structure.id == id) else { return false; };
        if fields.is_empty() || structure.fields.is_empty()
            || (nested && structure.copy_policy != CopyPolicy::Copy)
        { return false; }
        let member_types = structure.fields.iter().map(|field| (field.name.as_str(), &field.ty))
            .collect::<BTreeMap<_, _>>();
        for field in fields {
            let Some(ty) = member_types.get(field.name.as_str()).copied() else { return false; };
            match (ty, &field.value) {
                (ScalarType::Struct(inner), ScalarExpression::StructLiteral { fields, .. }) =>
                    pending.push((inner, fields, true)),
                (ScalarType::Array { element, length, .. }, ScalarExpression::ArrayLiteral { elements, .. })
                    if matches!(element.as_ref(), ScalarType::U64 | ScalarType::I32)
                        && *length != 0 && *length == elements.len() as u64 =>
                {
                    if !elements.iter().all(|value| guarded_inline_leaf(element, value, scope)) { return false; }
                }
                (_, value) if guarded_inline_leaf(ty, value, scope) => {}
                _ => return false,
            }
        }
    }
    true
}

fn guarded_inline_leaf(ty: &ScalarType, value: &ScalarExpression, scope: &BTreeMap<String, ScalarType>) -> bool {
    if !is_guarded_struct_field_type(ty) { return false; }
    match value {
        ScalarExpression::Integer { .. } | ScalarExpression::Boolean { .. } |
        ScalarExpression::Char { .. } | ScalarExpression::Float { .. } => true,
        ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } =>
            matches!(operand.as_ref(), ScalarExpression::Float { .. }),
        ScalarExpression::Name { name, .. } => scope.get(name) == Some(ty),
        expression => matches!(ty, ScalarType::U64 | ScalarType::I32 | ScalarType::Bool)
            && (matches!(expression, ScalarExpression::Call { type_arguments, .. } if type_arguments.is_empty())
                || guarded_numeric_rhs(ty, expression)),
    }
}

fn pending_fallible_operator(
    binding: &ScalarBinding,
    structs: &[ScalarStruct],
    scope: &BTreeMap<String, ScalarType>,
) -> Option<ByteSpan> {
    let operator = binding.fallible_assignment?;
    let [receiver] = binding.receivers.as_slice() else { return Some(operator) };
    let array_element = |element: &ScalarType, value: &ScalarExpression| match value {
        ScalarExpression::Integer { .. } => true,
        ScalarExpression::Name { name, .. } => scope.get(name) == Some(element),
        ScalarExpression::Call { type_arguments, .. } => type_arguments.is_empty(),
        _ => guarded_numeric_rhs(element, value),
    };
    if let Some((element, values, _)) = super::guarded_runtime_literal(binding) {
        return (!values.iter().all(|value| array_element(element, value))).then_some(operator);
    }
    match (&receiver.ty, &binding.value) {
        (ty, ScalarExpression::Integer { .. } | ScalarExpression::Name { .. })
            if is_integer_type(ty) => None,
        (ty, ScalarExpression::Call { type_arguments, .. })
            if (is_integer_type(ty) || *ty == ScalarType::Bool) && type_arguments.is_empty() => None,
        (ty, expression) if guarded_numeric_rhs(ty, expression) => None,
        (ScalarType::RuntimeArray { element, .. }, expression)
            if super::reference::aggregate::ordinary_value(element) && guarded_runtime_rhs(expression) => None,
        (ScalarType::Array { element, length, .. }, ScalarExpression::ArrayLiteral { elements, .. })
            if matches!(element.as_ref(), ScalarType::U64 | ScalarType::I32)
                && *length != 0 && *length == elements.len() as u64
                && elements.iter().all(|value| array_element(element, value)) => None,
        (ScalarType::Struct(id), ScalarExpression::StructLiteral { fields, .. }) => {
            if super::has_inline_aggregate_field(fields) {
                return (!guarded_inline_struct_literal(id, fields, structs, scope)).then_some(operator);
            }
            let Some(structure) = structs.iter().find(|structure| &structure.id == id) else { return Some(operator); };
            if fields.is_empty() || structure.fields.is_empty()
                || !structure.fields.iter().all(|field| is_guarded_struct_field_type(&field.ty))
            { return Some(operator); }
            let member_types = structure.fields.iter().map(|member| (member.name.as_str(), &member.ty))
                .collect::<BTreeMap<_, _>>();
            if fields.iter().all(|field| match &field.value {
                ScalarExpression::Integer { .. }
                | ScalarExpression::Boolean { .. }
                | ScalarExpression::Char { .. }
                | ScalarExpression::Float { .. } => true,
                ScalarExpression::Unary { operator: UnaryOperator::Negate, operand, .. } =>
                    matches!(operand.as_ref(), ScalarExpression::Float { .. }),
                ScalarExpression::Name { name, .. } => scope.get(name).is_some_and(is_guarded_struct_field_type),
                expression => member_types.get(field.name.as_str()).is_some_and(|ty|
                    matches!(ty, ScalarType::U64 | ScalarType::I32 | ScalarType::Bool)
                    && (matches!(expression, ScalarExpression::Call { type_arguments, .. } if type_arguments.is_empty())
                        || guarded_numeric_rhs(ty, expression))),
            }) { None } else { Some(operator) }
        }
        _ => Some(operator),
    }
}

fn fallible_assignment_diagnostic(source: &SourceIdentity, span: ByteSpan) -> super::Diagnostic {
    source_diagnostic(
        source,
        "B0003",
        "fallible allocation requires a local integer initialized by a literal or a same-type integer binding, or a local struct of integer, bool, char, or float fields initialized by literals or same-type lexical bindings",
        span,
    )
}

fn source_diagnostic(
    source: &SourceIdentity,
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
            span: SourceSpan::new(source.clone(), span),
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
    module: &ScalarModuleTyping<'_>,
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
    source: &SourceIdentity,
    name: &str,
    span: ByteSpan,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    if !is_declared_identifier_style(name) {
        diagnostics.push(source_diagnostic(
            source,
            "B0003",
            "identifier must use snake_case, PascalCase, or SCREAMING_SNAKE_CASE",
            span,
        ));
    }
}

fn validate_program_identifier_style(
    program: &ScalarProgramTyping<'_>,
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
    module: &ScalarModuleTyping<'_>,
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
        span: SourceSpan::new((*module.source).clone(), first_span),
        message: "first conflicting declaration".to_owned(),
    });
    diagnostic
}

fn unknown_member_diagnostic(
    importer: &ScalarModuleTyping<'_>,
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
                span: SourceSpan::new((*importer.source).clone(), member_span),
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
    items: &[ScalarItem],
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    validate_unit_if_positions_in_items(items, source, diagnostics);
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
    }
}

fn validate_unit_if_position(
    expression: &ScalarExpression,
    statement: bool,
    source: &SourceIdentity,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    match expression {
        ScalarExpression::AnonymousCallable { .. } | ScalarExpression::GeneratedArray { .. } => {}
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
        ScalarExpression::Unary { operand, .. } | ScalarExpression::ValueField { receiver: operand, .. } => {
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

pub(super) fn validate(
    program: &ScalarProgram,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> Vec<super::Diagnostic> {
    let mut diagnostics = Vec::new();
    validate_unit_if_positions(program, &mut diagnostics);
    let items = &program.items;
    let typing = ScalarProgramTyping {
        source: &program.source,
        structs: &program.structs,
        enums: &program.enums,
        target_layout: program.target_layout,
    };
    let program = &typing;
    let mut declarations = BTreeMap::new();
    let mut declaration_names = BTreeSet::new();
    let mut folded_declarations = BTreeMap::new();
    let mut struct_names = BTreeSet::new();
    for structure in program.structs {
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
    for item in items {
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
    for item in items {
        if let ScalarItem::Extern(extern_decl) = item {
            validate_extern(&program.source, extern_decl, &mut diagnostics);
        }
    }
    let mut scope = declarations.clone();
    for item in items {
        inference.set_validation_root(item);
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
                            overloads,
                            inference,
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
                    overloads,
                    inference,
                );
                validate_output_receivers(
                    &binding.value,
                    &binding.receivers,
                    binding.span,
                    &declarations,
                    program,
                    &mut diagnostics,
                    overloads,
                );
                expect_type(
                    program,
                    &binding.declared_type,
                    &actual,
                    binding.span,
                    &mut diagnostics,
                );
                TailConditionalTyping::Program(program).validate_secondary_binding_initializers(
                    binding,
                    &declarations,
                    &BTreeSet::new(),
                    &BTreeMap::new(),
                    &mut diagnostics,
                    false,
                    overloads,
                    inference,
                );
            }
            ScalarItem::Function(function) => {
                let typing = TailConditionalTyping::Program(program);
                let is_overload = !function.overload_arms.is_empty();
                let bodies = if is_overload {
                    function.overload_arms.as_slice()
                } else {
                    std::slice::from_ref(function)
                };
                for function in bodies {
                    let ScalarType::Callable {
                        outputs,
                        parameters,
                    } = &function.signature
                    else {
                        continue;
                    };
                    let multi_result_tail =
                        outputs.outputs.len() > 1 && function.body.tail_conditional().is_some();
                    let arity_mismatch = function.parameters.len() != parameters.len();
                    let inherited_overload_tail_order = is_overload && multi_result_tail;
                    if arity_mismatch && inherited_overload_tail_order {
                        diagnostics.push(diagnostic(
                            program,
                            "B0004",
                            "function parameter arity does not match its type",
                            function.span,
                        ));
                    }
                    let mut scope = BlockLexicalScope::new(
                        &declarations,
                        &declaration_names,
                        &folded_declarations,
                    );
                    if !function.parameters.is_empty() {
                        typing.declare_parameters(
                            function,
                            parameters,
                            scope.types.to_mut(),
                            scope.names.to_mut(),
                            scope.folded_names.to_mut(),
                            &mut diagnostics,
                        );
                    }
                    if outputs.outputs.len() > 1 && !function.body.final_output_values.is_empty() {
                        let tail = function.body.tail_conditional();
                        let unsafe_context = tail.is_some() && function.body.unsafe_context;
                        if function.body.items.len() != function.body.final_output_values.len() {
                            check_final_output_prefix(
                                &function.body,
                                scope.types.to_mut(),
                                scope.names.to_mut(),
                                scope.folded_names.to_mut(),
                                typing,
                                &mut diagnostics,
                                unsafe_context,
                                overloads,
                                inference,
                            );
                        }
                        if let Some(tail) = tail {
                            validate_tail_conditional_return(
                                tail,
                                &outputs.outputs,
                                &scope.types,
                                &scope.names,
                                &scope.folded_names,
                                typing,
                                &mut diagnostics,
                                unsafe_context,
                                overloads,
                                inference,
                            );
                        } else if function.body.final_output_values.len() == 1
                            && matches!(
                                function
                                    .body
                                    .final_expressions()
                                    .next()
                                    .expect("final output")
                                    .0,
                                ScalarExpression::Call { .. }
                            )
                        {
                            let (expression, value) = function
                                .body
                                .final_expressions()
                                .next()
                                .expect("final output");
                            typing.validate_sole_final_call(
                                expression,
                                value.span,
                                outputs,
                                &scope.types,
                                &scope.names,
                                &scope.folded_names,
                                &mut diagnostics,
                                overloads,
                                inference,
                            );
                        } else {
                            if (is_overload || function.body.final_output_values.len() > 1)
                                && function.body.final_output_values.len() != outputs.outputs.len()
                            {
                                diagnostics.push(diagnostic(
                                    program,
                                    "B0004",
                                    "function output arity does not match its final output list",
                                    function.body.span,
                                ));
                            }
                            for ((expression, value), output) in
                                function.body.final_expressions().zip(&outputs.outputs)
                            {
                                let actual = expression_type_expected(
                                    expression,
                                    &output.ty,
                                    &scope.types,
                                    &scope.names,
                                    &scope.folded_names,
                                    program,
                                    &mut diagnostics,
                                    false,
                                    overloads,
                                    inference,
                                );
                                expect_type(
                                    program,
                                    &output.ty,
                                    &actual,
                                    value.span,
                                    &mut diagnostics,
                                );
                            }
                        }
                    } else if let Some(output) = outputs.outputs.first() {
                        let actual = block_type_expected(
                            &function.body,
                            &output.ty,
                            &scope.types,
                            &scope.names,
                            &scope.folded_names,
                            program,
                            &mut diagnostics,
                            false,
                            overloads,
                            inference,
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
                            &scope.types,
                            &scope.names,
                            &scope.folded_names,
                            program,
                            &mut diagnostics,
                            false,
                            overloads,
                            inference,
                        );
                        expect_type(
                            program,
                            &ScalarType::Unit,
                            &actual,
                            function.body.span,
                            &mut diagnostics,
                        );
                    }
                    if arity_mismatch && !inherited_overload_tail_order {
                        diagnostics.push(diagnostic(
                            program,
                            "B0004",
                            "function parameter arity does not match its type",
                            function.span,
                        ));
                    }
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
                    overloads,
                    inference,
                );
            }
        }
    }
    for span in unused_binding_spans(items) {
        diagnostics.push(diagnostic(
            program,
            "B0009",
            "local binding or parameter has no required static use",
            span,
        ));
    }
    diagnostics
}

struct StaticUseAnalyzer<'tree> {
    visible: BTreeMap<&'tree str, ByteSpan>,
    scope_changes: Vec<(&'tree str, Option<ByteSpan>)>,
    declarations: Vec<ByteSpan>,
    used_spans: HashSet<ByteSpan>,
}

impl<'tree> StaticUseAnalyzer<'tree> {
    fn new(function: &'tree ScalarFunction) -> Self {
        let mut analyzer = Self {
            visible: BTreeMap::new(),
            scope_changes: Vec::new(),
            declarations: Vec::new(),
            used_spans: HashSet::new(),
        };
        for (index, name) in function.parameters.iter().enumerate() {
            if let Some(span) = function.parameter_spans.get(index) {
                analyzer.visible.insert(name.as_str(), *span);
                analyzer.declarations.push(*span);
            }
        }
        analyzer
    }

    fn block(&mut self, block: &'tree ScalarBlock) {
        let checkpoint = self.scope_changes.len();
        for item in &block.items {
            self.item(item, true);
        }
        while self.scope_changes.len() > checkpoint {
            let (name, previous) = self.scope_changes.pop().unwrap();
            if let Some(span) = previous {
                self.visible.insert(name, span);
            } else {
                self.visible.remove(name);
            }
        }
    }

    fn item(&mut self, item: &'tree ScalarBlockItem, nested: bool) {
        match item {
            ScalarBlockItem::LocalBinding(binding) => {
                if binding.is_allocation {
                    for receiver in &binding.receivers {
                        if let Some(length) = &receiver.allocation_length {
                            self.expression(length);
                        }
                    }
                } else {
                    self.expression(&binding.value);
                }
                let name = binding.name.as_str();
                let previous = self.visible.insert(name, binding.span);
                if nested {
                    self.scope_changes.push((name, previous));
                }
                self.declarations.push(binding.span);
            }
            ScalarBlockItem::Expression(expression) => self.expression(expression),
            ScalarBlockItem::Assignment(assignment) => {
                for (_, value) in assignment.rhs().evaluations() {
                    self.expression(value);
                }
                for (position, target) in assignment.targets.iter().enumerate() {
                    if position == 0 || !matches!(&target.place, ScalarPlace::Name { .. }) {
                        self.place(&target.place);
                    }
                }
            }
        }
    }

    fn expression(&mut self, expression: &'tree ScalarExpression) {
        match expression {
            ScalarExpression::AnonymousCallable { .. } | ScalarExpression::GeneratedArray { .. } => {}
            ScalarExpression::Name { name, .. } => self.use_name(name),
            ScalarExpression::Member { receiver, .. } => self.use_name(receiver),
            ScalarExpression::Unary { operand, .. } | ScalarExpression::ValueField { receiver: operand, .. } => self.expression(operand),
            ScalarExpression::Binary { left, right, .. } => {
                self.expression(left);
                self.expression(right);
            }
            ScalarExpression::Call {
                receiver,
                name,
                arguments,
                ..
            } => {
                if let Some(receiver) = receiver {
                    self.use_name(receiver);
                } else {
                    self.use_name(name);
                }
                for argument in arguments {
                    self.expression(argument);
                }
            }
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.expression(condition);
                self.block(then_branch);
                self.block(else_branch);
            }
            ScalarExpression::UnitIf {
                condition,
                then_branch,
                ..
            } => {
                self.expression(condition);
                self.block(then_branch);
            }
            ScalarExpression::Block(block) => self.block(block),
            ScalarExpression::RawAddress { place, .. }
            | ScalarExpression::CheckedAddress { place, .. }
            | ScalarExpression::Dereference { place, .. } => self.place(place),
            ScalarExpression::IndexedRead { place, .. } => self.place(place),
            ScalarExpression::StructLiteral { fields, .. } => {
                for field in fields {
                    self.expression(&field.value);
                }
            }
            ScalarExpression::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.expression(element);
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

    fn place(&mut self, place: &'tree ScalarPlace) {
        match place {
            ScalarPlace::Name { name, .. } => self.use_name(name),
            ScalarPlace::Field { base, .. } => self.place(base),
            ScalarPlace::Dereference { pointer, .. } => self.expression(pointer),
            ScalarPlace::Index { base, index, .. } => {
                self.place(base);
                self.expression(index);
            }
        }
    }

    fn use_name(&mut self, name: &str) {
        if let Some(span) = self.visible.get(name) {
            self.used_spans.insert(*span);
        }
    }

    fn unused(self) -> Vec<ByteSpan> {
        self.declarations
            .into_iter()
            .filter(|span| !self.used_spans.contains(span))
            .collect()
    }
}

pub(super) fn unused_binding_spans(items: &[ScalarItem]) -> Vec<ByteSpan> {
    items
        .iter()
        .filter_map(|item| match item {
            ScalarItem::Function(function) => {
                let mut analyzer = StaticUseAnalyzer::new(function);
                for item in &function.body.items {
                    analyzer.item(item, false);
                }
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
    program: &ScalarProgramTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let mut scope = BlockLexicalScope::new(scope, visible_names, folded_names);
    let unsafe_context = unsafe_context || block.unsafe_context;
    let mut result = ScalarType::Unit;
    for (index, item) in block.items.iter().enumerate() {
        result = scope.item_type(
            item,
            TailConditionalTyping::Program(program),
            diagnostics,
            unsafe_context,
            overloads,
            inference,
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let mut scope = BlockLexicalScope::new(scope, visible_names, folded_names);
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
                    &scope.types,
                    &scope.names,
                    &scope.folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                )
            }
            _ => scope.item_type(
                item,
                TailConditionalTyping::Program(program),
                diagnostics,
                unsafe_context,
                overloads,
                inference,
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    match item {
        ScalarBlockItem::LocalBinding(binding) => {
            if let Some(operator) = pending_fallible_operator(binding, program.structs, scope) {
                diagnostics.push(fallible_assignment_diagnostic(program.source, operator));
                scope.insert(binding.name.clone(), ScalarType::Error);
                return ScalarType::Unit;
            }
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
                        overloads,
                        inference,
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
            let widening = widened_local_literal_type(binding);
            let actual = expression_type_expected(
                &binding.value,
                widening.as_ref().unwrap_or(&binding.declared_type),
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
            );
            validate_output_receivers(
                &binding.value,
                &binding.receivers,
                binding.span,
                scope,
                program,
                diagnostics,
                overloads,
            );
            expect_type(
                program,
                widening.as_ref().or_else(|| super::widened_fixed_copy(binding, &actual).map(|_| &actual))
                    .unwrap_or(&binding.declared_type),
                &actual,
                binding.span,
                diagnostics,
            );
            TailConditionalTyping::Program(program).validate_secondary_binding_initializers(
                binding,
                scope,
                visible_names,
                folded_names,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
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
            overloads,
            inference,
        ),
        ScalarBlockItem::Assignment(assignment) => assignment_type(
            assignment,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
        ),
    }
}

fn assignment_type(
    assignment: &ScalarAssignment,
    scope: &mut BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let expected = assignment
        .targets
        .iter()
        .map(|target| {
            assignment_target_type(target, scope, program, diagnostics, overloads, inference)
        })
        .collect::<Vec<_>>();
    validate_assignment_target_distinctness(assignment, scope, program, diagnostics);
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
                    overloads,
                    inference,
                ),
                None => expression_type(
                    value,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                ),
            };
            match call_output_sequence(value, scope, program, overloads) {
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
        inference.record_assignment_outputs(&program.source, assignment, &outputs);
        if assignment.targets.len() > outputs.outputs.len() {
            diagnostics.push(diagnostic(
                program,
                "B0004",
                "call has fewer outputs than assignment targets",
                assignment.span,
            ));
        }
        let mut inferred_names = None;
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
                let (names, folded) = inferred_names
                    .get_or_insert_with(|| (visible_names.clone(), folded_names.clone()));
                if declare_program_name(
                    program,
                    name,
                    *span,
                    names,
                    folded,
                    diagnostics,
                ) {
                    scope.insert(name.clone(), output.ty.clone());
                    inference.record_place_type(&target.place, &output.ty);
                }
            }
        }
    }
    ScalarType::Unit
}

fn assignment_target_type(
    target: &ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
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
            if let Some(ty) = &expected {
                inference.record_place_type(&target.place, ty);
            }
            expected
        }
        place => Some(writable_place_type(
            place,
            target.span,
            scope,
            program,
            diagnostics,
            overloads,
            inference,
        )),
    }
}

fn validate_assignment_target_distinctness(
    assignment: &ScalarAssignment,
    scope: &BTreeMap<String, ScalarType>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
) {
    for (span, message) in assignment_target_distinctness(assignment, scope) {
        diagnostics.push(diagnostic(program, "B0002", message, span));
    }
}

// Only inline value storage contributes an independent root here. This does
// not infer anything from the spelling of a pointer, namespace or projection;
// those continue to need separate prepared alias/relationship evidence.
fn direct_inline_assignment_root<'a>(
    target: &'a ScalarAssignmentTarget,
    scope: &BTreeMap<String, ScalarType>,
) -> Option<&'a str> {
    use super::reference::aggregate::ordinary_value;
    match &target.place {
        ScalarPlace::Name { name, .. } if scope.get(name).is_some_and(ordinary_value) => Some(name),
        ScalarPlace::Index { base, .. } => {
            let ScalarPlace::Name { name, .. } = base.as_ref() else { return None; };
            matches!(scope.get(name), Some(ScalarType::Array { element, .. }) if ordinary_value(element))
                .then_some(name.as_str())
        }
        _ => None,
    }
}

fn assignment_target_distinctness<'a>(
    assignment: &'a ScalarAssignment,
    scope: &BTreeMap<String, ScalarType>,
) -> impl Iterator<Item = (ByteSpan, &'static str)> + 'a {
    let targets = if assignment.targets.len() > 1 { assignment.targets.as_slice() } else { &[] };
    let facts = targets.iter().map(|target| (
        scalar_place_identity(&target.place),
        place_contains_dynamic_index(&target.place),
        direct_inline_assignment_root(target, scope),
    )).collect::<Vec<_>>();
    facts.into_iter()
        .scan(Vec::new(), move |previous, (identity, dynamic, root)| {
            let span = targets[previous.len()].target_span;
            let failures = previous.iter().filter_map(|(previous_identity, previous_dynamic, previous_root)| {
                if &identity == previous_identity {
                    Some((span, "duplicate assignment target"))
                } else if (dynamic || *previous_dynamic)
                    && !matches!((root, *previous_root), (Some(left), Some(right)) if left != right)
                {
                    Some((span, "assignment targets require a distinctness proof"))
                } else {
                    None
                }
            }).collect::<Vec<_>>();
            previous.push((identity, dynamic, root));
            Some(failures)
        })
        .flatten()
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

pub(super) fn expression_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = check_expression_type(
        expression,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
    );
    inference.record_expression_type(expression, &ty, false);
    ty
}

fn check_expression_type(
    expression: &ScalarExpression,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    match expression {
        ScalarExpression::GeneratedArray { span, .. } => {
            diagnostics.push(diagnostic(program, "B0003", "generated array requires an exact prepared fixed array owner", *span));
            ScalarType::Error
        }
        ScalarExpression::AnonymousCallable { span, .. } => {
            diagnostics.push(diagnostic(program, "B0003", "inline callable lacks its prepared fixed array owner", *span));
            ScalarType::Error
        }
        ScalarExpression::ValueField { receiver, name, name_span, span } => {
            if !matches!(receiver.as_ref(), ScalarExpression::Call { .. }) {
                diagnostics.push(diagnostic(program, "B0003", "field value receiver must be a call result", *span));
                return ScalarType::Error;
            }
            let ty = expression_type(
                receiver, scope, visible_names, folded_names,
                program, diagnostics, unsafe_context, overloads, inference,
            );
            if is_error_type(&ty) {
                return ScalarType::Error;
            }
            match checked_value_field(&ty, name, |id| program.structs.get(id.index)) {
                Ok(field) => {
                    inference.record_value_field(expression, &field.id);
                    field.ty.clone()
                }
                Err(message) => {
                    diagnostics.push(diagnostic(program, "B0003", message, *name_span));
                    ScalarType::Error
                }
            }
        }
        ScalarExpression::RawAddress { place, span } => {
            if !unsafe_context {
                diagnostics.push(diagnostic(
                    program,
                    "B0012",
                    "raw address requires an unsafe block",
                    *span,
                ));
            }
            let ty = check_place_type(
                place,
                scope,
                program,
                diagnostics,
                overloads,
                inference,
                PlaceTypeMode::RawAddress,
            );
            inference.record_place_type(place, &ty);
            ScalarType::RawPointer(Box::new(ty))
        }
        ScalarExpression::CheckedAddress {
            mutability,
            place,
            span,
        } => checked_address_type(
            *mutability,
            place,
            *span,
            scope,
            program,
            diagnostics,
            overloads,
            inference,
        ),
        ScalarExpression::Dereference { place, span } => checked_dereference_type(
            place,
            *span,
            scope,
            program,
            diagnostics,
            unsafe_context,
            overloads,
            inference,
        ),
        ScalarExpression::IndexedRead { place, .. } => {
            let ty = check_place_type(
                place, scope, program, diagnostics, overloads, inference,
                PlaceTypeMode::IndexedRead { unsafe_context },
            );
            inference.record_place_type(place, &ty);
            ty
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
                if let Some(tag) = *enum_tag {
                    inference.record_member(
                        expression,
                        CheckedMember::Enum {
                            enumeration: enumeration.id.clone(),
                            tag,
                        },
                    );
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
            None => {
                let receiver_type = scope.get(receiver);
                let Some(field) =
                    member_field(receiver_type, name, *name_span, *span, program, diagnostics)
                else {
                    return ScalarType::Error;
                };
                inference.record_member(
                    expression,
                    if matches!(receiver_type, Some(ScalarType::Struct(_))) {
                        CheckedMember::DirectField(field.id.clone())
                    } else {
                        CheckedMember::RawPointerField(field.id.clone())
                    },
                );
                field.ty.clone()
            }
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
                    let diagnostics_before_range = diagnostics.len();
                    validate_integer_range_program(program, &value, *span, diagnostics);
                    if diagnostics.len() == diagnostics_before_range {
                        inference.record_folded_signed_integer_magnitude(operand);
                    }
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
                overloads,
                inference,
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
                    if !is_float_type(&operand_type) {
                        expect_integer(program, &operand_type, *span, diagnostics);
                    }
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
            let literal_operands = float_literal_arithmetic_operands(operator, left, right, inference);
            if let Some(inferred) = infer_float_literal_arithmetic(literal_operands, left, right, |operand, expected| {
                if let Some(expected) = expected {
                    expression_type_expected(
                        operand, expected, scope, visible_names, folded_names, program, diagnostics, unsafe_context, overloads, inference,
                    )
                } else {
                    expression_type(
                        operand, scope, visible_names, folded_names, program, diagnostics, unsafe_context, overloads, inference,
                    )
                }
            }) {
                return inferred.unwrap_or_else(|()| {
                    diagnostics.push(diagnostic(
                        program, "B0003", "floating-point arithmetic requires a matching f32 or f64 context", *span,
                    ));
                    ScalarType::Error
                });
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
                    overloads,
                    inference,
                );
            }
            let (left_type, right_type) = if is_homogeneous_bitwise(operator) {
                infer_bitwise_operands(left, right, |operand, expected| {
                    if let Some(expected) = expected {
                        expression_type_expected(
                            operand, expected, scope, visible_names, folded_names, program,
                            diagnostics, unsafe_context, overloads, inference,
                        )
                    } else {
                        expression_type(
                            operand, scope, visible_names, folded_names, program, diagnostics,
                            unsafe_context, overloads, inference,
                        )
                    }
                })
            } else {
                let left_type = expression_type(
                    left,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
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
                        overloads,
                        inference,
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
                        overloads,
                        inference,
                    )
                };
                (left_type, right_type)
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
                let required = required_binary_arithmetic_type(None, &left_type);
                if let Some(required) = required {
                    expect_type(program, required, &left_type, *span, diagnostics);
                }
                expect_type(program, &left_type, &right_type, *span, diagnostics);
                required.cloned().unwrap_or(left_type)
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
                let result = ScalarType::ArtifactId;
                inference.record_core_call_signature(expression, Vec::new(), &result, *span);
                return result;
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
                let Some(utf8_id) = program.local_utf8_id() else {
                    diagnostics.push(diagnostic(
                        program,
                        "B0003",
                        "std.utf8 is unavailable",
                        *span,
                    ));
                    return ScalarType::Error;
                };
                let expected = ScalarType::Struct(utf8_id.clone());
                let diagnostics_before_argument = diagnostics.len();
                let actual = expression_type_expected(
                    &arguments[0],
                    &expected,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                );
                expect_type(program, &expected, &actual, *span, diagnostics);
                if is_error_type(&actual) || diagnostics.len() != diagnostics_before_argument {
                    return ScalarType::Error;
                }
                let result = ScalarType::ArtifactId;
                inference.record_core_call_signature(expression, vec![expected], &result, *span);
                return result;
            }
            if receiver.as_deref() == Some("core")
                && matches!(
                    name.as_str(),
                    "alloc" | "free" | "invalidate" | "rebind" | "array_len" | "system_panic"
                )
            {
                return type_core_memory(
                    expression,
                    name,
                    arguments,
                    *span,
                    scope,
                    visible_names,
                    folded_names,
                    program,
                    diagnostics,
                    unsafe_context,
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core")
                && matches!(name.as_str(), "int_trunc" | "int_extend")
            {
                return type_core_int_conversion(
                    expression,
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
                    overloads,
                    inference,
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
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core") && name == "bitcast" {
                return type_core_bitcast(
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core") && matches!(name.as_str(), "offset" | "load") {
                return type_core_raw_memory(
                    expression,
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
                    overloads,
                    inference,
                );
            }
            if receiver.as_deref() == Some("core") && name == "pointer_cast" {
                return type_core_pointer_cast(
                    expression,
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
                    overloads,
                    inference,
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
                    let diagnostics_before_arguments = diagnostics.len();
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
                            overloads,
                            inference,
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
                        if diagnostics.len() == diagnostics_before_arguments {
                            inference.record_call_signature(expression, parameters, outputs, None);
                        }
                        scalar_call_result(outputs)
                    };
                }
            }
            if let Some(overload) = receiver
                .is_none()
                .then(|| overloads.first(&program.source, name))
                .flatten()
            {
                let diagnostics_before_arguments = diagnostics.len();
                let argument_types = inference.without_checked_occurrences(|inference| {
                    arguments
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
                                overloads,
                                inference,
                            )
                        })
                        .collect()
                });
                let overload_arguments = arguments
                    .iter()
                    .zip(&argument_types)
                    .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                    .collect::<Vec<_>>();
                let (callable, selection) =
                    match resolve_overload_candidate(overload, &overload_arguments, None) {
                        Ok(resolved) => resolved,
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
                        overloads,
                        inference,
                    );
                    expect_type(
                        program,
                        parameter,
                        &actual,
                        expression_span(argument),
                        diagnostics,
                    );
                }
                if diagnostics.len() == diagnostics_before_arguments {
                    inference.record_call_signature(
                        expression,
                        &parameters,
                        &outputs,
                        Some((selection, overload.origin)),
                    );
                }
                return scalar_call_result(&outputs);
            }
            let lookup_name = receiver
                .as_ref()
                .map_or_else(|| name.clone(), |receiver| format!("{receiver}.{name}"));
            let callable = scope.get(&lookup_name).cloned().or_else(|| {
                overloads
                    .extern_member(receiver.as_deref(), name)
                    .map(|member| member.signature.clone())
            });
            let unsafe_callable = overloads
                .extern_member(receiver.as_deref(), name)
                .is_some_and(|member| member.any_unsafe);
            let Some(callable) = callable else {
                diagnostics.push(diagnostic(program, "B0001", "unknown callable name", *span));
                return ScalarType::Error;
            };
            let extern_callable = overloads.extern_member(receiver.as_deref(), name).is_some();
            if extern_callable && !type_arguments.is_empty() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "generic argument arity does not match callable declaration",
                    *span,
                ));
                return ScalarType::Error;
            }
            let generic_parameters = (receiver.is_none()).then(|| overloads.generics(name));
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
            let diagnostics_before_arguments = diagnostics.len();
            if arguments.len() != parameters.len() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "call argument arity does not match callable type",
                    *span,
                ));
            }
            let mut error_argument = false;
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
                    overloads,
                    inference,
                );
                let fixed_copy = !unsafe_callable && parameters.len() == 1 && type_arguments.is_empty()
                    && matches!(argument, ScalarExpression::Name { .. })
                    && super::fixed_copy_owning_signature(&parameters, &outputs)
                    && super::fixed_copy_to_runtime(parameter, &actual).is_some();
                if !fixed_copy {
                    expect_type(program, parameter, &actual, *span, diagnostics);
                }
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
                if diagnostics.len() == diagnostics_before_arguments {
                    inference.record_call_signature(expression, &parameters, &outputs, None);
                }
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
                overloads,
                inference,
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
                overloads,
                inference,
            );
            let else_type = block_type(
                else_branch,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
            overloads,
            inference,
        ),
    }
}

fn expect_type(
    program: &ScalarProgramTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    let ty = check_expression_type_expected(
        expression,
        expected,
        scope,
        visible_names,
        folded_names,
        program,
        diagnostics,
        unsafe_context,
        overloads,
        inference,
    );
    inference.record_expression_type(expression, &ty, true);
    ty
}

fn check_expression_type_expected(
    expression: &ScalarExpression,
    expected: &ScalarType,
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgramTyping<'_>,
    diagnostics: &mut Vec<super::Diagnostic>,
    unsafe_context: bool,
    overloads: &ScalarTypingView<'_>,
    inference: &mut ScalarInferenceBuilder<'_>,
) -> ScalarType {
    if is_error_type(expected) {
        return ScalarType::Error;
    }
    if matches!(expression, ScalarExpression::Utf8 { .. })
        && matches!(expected, ScalarType::Struct(id) if program.local_utf8_id() == Some(id))
    {
        return expected.clone();
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
        if let Some(overload) = overloads.first(&program.source, name) {
            if let Some(type_argument) = type_arguments.first() {
                diagnostics.push(diagnostic(
                    program,
                    "B0004",
                    "overload calls do not accept explicit type arguments",
                    type_argument.span,
                ));
                return ScalarType::Error;
            }
            let diagnostics_before_arguments = diagnostics.len();
            let argument_types = inference.without_checked_occurrences(|inference| {
                arguments
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
                            overloads,
                            inference,
                        )
                    })
                    .collect()
            });
            let overload_arguments = arguments
                .iter()
                .zip(&argument_types)
                .map(|(argument, actual)| overload_argument_from_actual(argument, actual))
                .collect::<Vec<_>>();
            let (callable, selection) =
                match resolve_overload_candidate(overload, &overload_arguments, Some(expected)) {
                    Ok(resolved) => resolved,
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
                    overloads,
                    inference,
                );
                expect_type(
                    program,
                    parameter,
                    &actual,
                    expression_span(argument),
                    diagnostics,
                );
            }
            if diagnostics.len() == diagnostics_before_arguments {
                inference.record_call_signature(
                    expression,
                    &parameters,
                    &outputs,
                    Some((selection, overload.origin)),
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
                    let diagnostics_before_range = diagnostics.len();
                    validate_integer_range_for_type_program(
                        program,
                        &value,
                        *span,
                        expected,
                        diagnostics,
                    );
                    if diagnostics.len() == diagnostics_before_range {
                        inference.record_folded_signed_integer_magnitude(operand);
                    }
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
            overloads,
            inference,
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
                if !is_float_type(&operand_type) {
                    expect_integer(program, &operand_type, *span, diagnostics);
                }
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                overloads,
                inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                        overloads,
                        inference,
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
                        overloads,
                        inference,
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
                        overloads,
                        inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
                    overloads,
                    inference,
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
            overloads,
            inference,
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

fn member_field<'a>(
    receiver: Option<&ScalarType>,
    name: &str,
    name_span: ByteSpan,
    span: ByteSpan,
    program: &ScalarProgramTyping<'a>,
    diagnostics: &mut Vec<super::Diagnostic>,
) -> Option<&'a ScalarStructField> {
    let Some(receiver) = receiver else {
        diagnostics.push(diagnostic(program, "B0001", "unknown name", span));
        return None;
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
        return None;
    };
    let Some(field) = structure.fields.iter().find(|field| field.name == name) else {
        diagnostics.push(diagnostic(program, "B0001", "unknown field", name_span));
        return None;
    };
    Some(field)
}

fn validate_integer_range(
    module: &ScalarModuleTyping<'_>,
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
    module: &ScalarModuleTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
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
    program: &ScalarProgramTyping<'_>,
    code: &str,
    message: &str,
    span: ByteSpan,
) -> super::Diagnostic {
    source_diagnostic(&program.source, code, message, span)
}

fn declare_program_name(
    program: &ScalarProgramTyping<'_>,
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
                span: SourceSpan::new((*program.source).clone(), *first_span),
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
