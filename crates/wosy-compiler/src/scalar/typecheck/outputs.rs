use super::*;
#[cfg(test)]
mod canonical_source_tests;

pub(super) fn record(
    facts: &RefCell<Vec<ResolvedExpressionOutputFact>>,
    source: &SourceIdentity,
    span: ByteSpan,
    outputs: ScalarOutputSequence,
) {
    let fact = ResolvedExpressionOutputFact {
        source_span: SourceSpan::new(source.clone(), span),
        outputs,
    };
    let mut facts = facts.borrow_mut();
    if let Some(existing) = facts
        .iter_mut()
        .find(|existing| existing.source_span == fact.source_span)
    {
        *existing = fact;
    } else {
        facts.push(fact);
    }
}

pub(super) fn lookup(
    facts: &RefCell<Vec<ResolvedExpressionOutputFact>>,
    source: &SourceIdentity,
    span: ByteSpan,
) -> Option<ScalarOutputSequence> {
    let source_span = SourceSpan::new(source.clone(), span);
    facts
        .borrow()
        .iter()
        .find(|fact| fact.source_span == source_span)
        .map(|fact| fact.outputs.clone())
}

pub(super) fn call_result(
    facts: &RefCell<Vec<ResolvedExpressionOutputFact>>,
    source: &SourceIdentity,
    expression: &ScalarExpression,
    outputs: &ScalarOutputSequence,
) -> ScalarType {
    record(facts, source, expression_span(expression), outputs.clone());
    scalar_call_result(outputs)
}

fn block_outputs(
    facts: &RefCell<Vec<ResolvedExpressionOutputFact>>,
    source: &SourceIdentity,
    block: &ScalarBlock,
    result: &ScalarType,
) -> Result<ScalarOutputSequence, ReferenceAnalysisError> {
    let group = group_conditional_final_outputs(source, block)?;
    let outputs = if let Some(group) = group {
        lookup(facts, source, expression_span(&group.expression))
            .ok_or_else(|| ReferenceAnalysisError::MissingContext {
                source_span: SourceSpan::new(source.clone(), expression_span(&group.expression)),
                reason: ReferenceAnalysisErrorReason::InvalidAssignmentCorrespondence,
            })?
            .outputs
    } else if block.final_output_values.is_empty() {
        vec![ScalarOutput {
            ty: result.clone(),
            span: block.span,
        }]
    } else {
        block
            .final_output_values
            .iter()
            .flat_map(|output| {
                lookup(facts, source, expression_span(&output.value))
                    .expect("typed final expression output sequence")
                    .outputs
            })
            .collect()
    };
    Ok(ScalarOutputSequence {
        outputs,
        span: block.span,
    })
}

pub(super) fn finish(
    facts: &RefCell<Vec<ResolvedExpressionOutputFact>>,
    assignments: &RefCell<Vec<ResolvedAssignmentTypeFact>>,
    source: &SourceIdentity,
    expression: &ScalarExpression,
    result: &ScalarType,
) -> Result<Vec<(ScalarOutputSequence, ScalarOutputSequence)>, ReferenceAnalysisError> {
    if is_error_type(result) {
        record(
            facts,
            source,
            expression_span(expression),
            ScalarOutputSequence {
                outputs: vec![ScalarOutput {
                    ty: result.clone(),
                    span: expression_span(expression),
                }],
                span: expression_span(expression),
            },
        );
        return Ok(Vec::new());
    }
    let mut disagreements = Vec::new();
    let sequence = match expression {
        ScalarExpression::Assignment(assignment) => assignments
            .borrow()
            .iter()
            .find(|fact| fact.occurrence == *assignment.occurrence)
            .expect("typed assignment output sequence")
            .previous
            .clone(),
        ScalarExpression::Call { receiver, .. } if receiver.as_deref() == Some("core") => {
            ScalarOutputSequence {
                outputs: vec![ScalarOutput {
                    ty: result.clone(),
                    span: expression_span(expression),
                }],
                span: expression_span(expression),
            }
        }
        ScalarExpression::Call { .. } => lookup(facts, source, expression_span(expression))
            .expect("resolved callable output sequence"),
        ScalarExpression::Block(block) => block_outputs(facts, source, block, result)?,
        ScalarExpression::If {
            then_branch,
            else_branch,
            ..
        } => {
            let left = block_outputs(facts, source, then_branch, result)?;
            let right = block_outputs(facts, source, else_branch, result)?;
            if left.outputs.len() != right.outputs.len()
                || left
                    .outputs
                    .iter()
                    .zip(&right.outputs)
                    .any(|(left, right)| !scalar_type_equal(&left.ty, &right.ty))
            {
                disagreements.push((left.clone(), right));
            }
            left
        }
        _ => ScalarOutputSequence {
            outputs: vec![ScalarOutput {
                ty: result.clone(),
                span: expression_span(expression),
            }],
            span: expression_span(expression),
        },
    };
    record(facts, source, expression_span(expression), sequence);
    Ok(disagreements)
}

pub(super) fn arguments(
    expressions: &[ScalarExpression],
    parameters: &[ScalarType],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::super::Diagnostic>,
    unsafe_context: bool,
) -> Vec<ScalarOutput> {
    let mut resolved = Vec::new();
    for expression in expressions {
        let start = resolved.len().min(parameters.len());
        resolved.extend(
            sequence(
                expression,
                &parameters[start..],
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            )
            .outputs,
        );
    }
    resolved
}

pub(super) fn arguments_in_module(
    expressions: &[ScalarExpression],
    parameters: &[ScalarType],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::super::Diagnostic>,
    unsafe_context: bool,
) -> Vec<ScalarOutput> {
    let mut resolved = Vec::new();
    for expression in expressions {
        let start = resolved.len().min(parameters.len());
        resolved.extend(
            sequence_in_module(
                expression,
                &parameters[start..],
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            )
            .outputs,
        );
    }
    resolved
}

pub(in crate::scalar) fn sequence(
    expression: &ScalarExpression,
    expected: &[ScalarType],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarOutputSequence {
    if let ScalarExpression::Call {
        receiver: None,
        name,
        name_span,
        type_arguments,
        arguments: expressions,
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
            arguments(
                expressions,
                &[],
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let actual = overload_arguments(
                expressions,
                &program.resolved_expression_outputs,
                &program.source,
            );
            let callable = if type_arguments.is_empty() {
                resolve_overload_candidate_outputs(overload, &actual, expected, expressions.len())
            } else {
                Err("overload calls do not accept explicit type arguments")
            };
            match callable {
                Ok((
                    ScalarType::Callable {
                        outputs,
                        parameters,
                    },
                    _,
                )) => {
                    let inputs = arguments(
                        expressions,
                        &parameters,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                    for (input, parameter) in inputs.iter().zip(&parameters) {
                        expect_type(program, parameter, &input.ty, input.span, diagnostics);
                    }
                    record(
                        &program.resolved_expression_outputs,
                        &program.source,
                        expression_span(expression),
                        outputs.clone(),
                    );
                    return outputs;
                }
                Ok(_) => unreachable!("overload callable"),
                Err(message) => {
                    diagnostics.push(diagnostic(program, "B0004", message, *name_span));
                    let outputs = ScalarOutputSequence {
                        outputs: vec![ScalarOutput {
                            ty: ScalarType::Error,
                            span: expression_span(expression),
                        }],
                        span: expression_span(expression),
                    };
                    record(
                        &program.resolved_expression_outputs,
                        &program.source,
                        expression_span(expression),
                        outputs.clone(),
                    );
                    return outputs;
                }
            }
        }
    }
    let outputs = match expression {
        ScalarExpression::Block(block) => block_sequence(
            block,
            expected,
            scope,
            visible_names,
            folded_names,
            program,
            diagnostics,
            unsafe_context,
        ),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let condition_type = expression_type_expected(
                condition,
                &ScalarType::Bool,
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
            let left = block_sequence(
                then_branch,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            let right = block_sequence(
                else_branch,
                expected,
                scope,
                visible_names,
                folded_names,
                program,
                diagnostics,
                unsafe_context,
            );
            if !equal(&left, &right) {
                diagnostics.push(diagnostic(
                    program,
                    if left.outputs.len() == right.outputs.len() {
                        "B0006"
                    } else {
                        "B0004"
                    },
                    "conditional branches must have equal ordered output types and arity",
                    expression_span(expression),
                ));
            }
            left
        }
        _ => {
            match expected.first() {
                Some(expected) => {
                    expression_type_expected(
                        expression,
                        expected,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                }
                None => {
                    expression_type(
                        expression,
                        scope,
                        visible_names,
                        folded_names,
                        program,
                        diagnostics,
                        unsafe_context,
                    );
                }
            }
            lookup(
                &program.resolved_expression_outputs,
                &program.source,
                expression_span(expression),
            )
            .expect("typed expression output sequence")
        }
    };
    record(
        &program.resolved_expression_outputs,
        &program.source,
        expression_span(expression),
        outputs.clone(),
    );
    outputs
}

pub(in crate::scalar) fn sequence_in_module(
    expression: &ScalarExpression,
    expected: &[ScalarType],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarOutputSequence {
    if let ScalarExpression::Call {
        receiver,
        name,
        name_span,
        type_arguments,
        arguments: expressions,
        ..
    } = expression
    {
        let target = match receiver {
            None => Some(module),
            Some(receiver) => module
                .namespace_bindings
                .iter()
                .find(|namespace| namespace.binding == *receiver)
                .and_then(|namespace| {
                    modules
                        .iter()
                        .find(|module| module.source == namespace.target)
                }),
        };
        if let Some(overload) = target.and_then(|target| {
            target.items.iter().find_map(|item| match item {
                ScalarItem::Function(function)
                    if function.name == *name
                        && !function.overload_arms.is_empty()
                        && (receiver.is_none() || target.members.contains_key(name)) =>
                {
                    Some(function)
                }
                _ => None,
            })
        }) {
            arguments_in_module(
                expressions,
                &[],
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            let actual = overload_arguments(
                expressions,
                &module.resolved_expression_outputs,
                &module.source,
            );
            let callable = if type_arguments.is_empty() {
                resolve_overload_candidate_outputs(overload, &actual, expected, expressions.len())
            } else {
                Err("overload calls do not accept explicit type arguments")
            };
            match callable {
                Ok((
                    ScalarType::Callable {
                        outputs,
                        parameters,
                    },
                    _,
                )) => {
                    let inputs = arguments_in_module(
                        expressions,
                        &parameters,
                        scope,
                        visible_names,
                        folded_names,
                        module,
                        modules,
                        diagnostics,
                        unsafe_context,
                    );
                    for (input, parameter) in inputs.iter().zip(&parameters) {
                        expect_module_type(module, parameter, &input.ty, input.span, diagnostics);
                    }
                    record(
                        &module.resolved_expression_outputs,
                        &module.source,
                        expression_span(expression),
                        outputs.clone(),
                    );
                    return outputs;
                }
                Ok(_) => unreachable!("overload callable"),
                Err(message) => {
                    diagnostics.push(module_diagnostic(module, "B0004", message, *name_span));
                    let outputs = ScalarOutputSequence {
                        outputs: vec![ScalarOutput {
                            ty: ScalarType::Error,
                            span: expression_span(expression),
                        }],
                        span: expression_span(expression),
                    };
                    record(
                        &module.resolved_expression_outputs,
                        &module.source,
                        expression_span(expression),
                        outputs.clone(),
                    );
                    return outputs;
                }
            }
        }
    }
    let outputs = match expression {
        ScalarExpression::Block(block) => block_sequence_in_module(
            block,
            expected,
            scope,
            visible_names,
            folded_names,
            module,
            modules,
            diagnostics,
            unsafe_context,
        ),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let condition_type = expression_type_in_module_expected(
                condition,
                Some(&ScalarType::Bool),
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
            let left = block_sequence_in_module(
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
            let right = block_sequence_in_module(
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
            if !equal(&left, &right) {
                diagnostics.push(module_diagnostic(
                    module,
                    if left.outputs.len() == right.outputs.len() {
                        "B0006"
                    } else {
                        "B0004"
                    },
                    "conditional branches must have equal ordered output types and arity",
                    expression_span(expression),
                ));
            }
            left
        }
        _ => {
            expression_type_in_module_expected(
                expression,
                expected.first(),
                scope,
                visible_names,
                folded_names,
                module,
                modules,
                diagnostics,
                unsafe_context,
            );
            lookup(
                &module.resolved_expression_outputs,
                &module.source,
                expression_span(expression),
            )
            .expect("typed expression output sequence")
        }
    };
    record(
        &module.resolved_expression_outputs,
        &module.source,
        expression_span(expression),
        outputs.clone(),
    );
    outputs
}

fn equal(left: &ScalarOutputSequence, right: &ScalarOutputSequence) -> bool {
    left.outputs.len() == right.outputs.len()
        && left
            .outputs
            .iter()
            .zip(&right.outputs)
            .all(|(left, right)| scalar_type_equal(&left.ty, &right.ty))
}

pub(super) fn overload_arguments(
    expressions: &[ScalarExpression],
    facts: &RefCell<Vec<ResolvedExpressionOutputFact>>,
    source: &SourceIdentity,
) -> Vec<ScalarOverloadArgument> {
    expressions
        .iter()
        .flat_map(|expression| {
            lookup(facts, source, expression_span(expression))
                .expect("typed overload argument outputs")
                .outputs
                .iter()
                .map(|output| overload_argument_from_actual(expression, &output.ty))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn block_sequence(
    block: &ScalarBlock,
    expected: &[ScalarType],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarOutputSequence {
    let mut scope = scope.clone();
    let mut names = visible_names.clone();
    let mut folded = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        block_item_type(
            item,
            &mut scope,
            &mut names,
            &mut folded,
            program,
            diagnostics,
            unsafe_context,
        );
    }
    match group_conditional_final_outputs(&program.source, block) {
        Ok(Some(group)) => {
            return sequence(&group.expression, expected, &scope, &names, &folded, program, diagnostics, unsafe_context);
        }
        Ok(None) => {}
        Err(error) => {
            let diagnostic = error.diagnostic();
            let span = diagnostic.labels[0].span.range;
            diagnostics.push(diagnostic);
            return ScalarOutputSequence { outputs: vec![ScalarOutput { ty: ScalarType::Error, span }], span: block.span };
        }
    }
    let mut outputs = Vec::new();
    for value in &block.final_output_values {
        let start = outputs.len().min(expected.len());
        outputs.extend(
            sequence(
                &value.value,
                &expected[start..],
                &scope,
                &names,
                &folded,
                program,
                diagnostics,
                unsafe_context,
            )
            .outputs,
        );
    }
    if block.final_output_values.is_empty() {
        outputs.push(ScalarOutput {
            ty: ScalarType::Unit,
            span: block.span,
        });
    }
    ScalarOutputSequence {
        outputs,
        span: block.span,
    }
}

pub(super) fn block_sequence_in_module(
    block: &ScalarBlock,
    expected: &[ScalarType],
    scope: &BTreeMap<String, ScalarType>,
    visible_names: &BTreeSet<String>,
    folded_names: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::super::Diagnostic>,
    unsafe_context: bool,
) -> ScalarOutputSequence {
    let mut scope = scope.clone();
    let mut names = visible_names.clone();
    let mut folded = folded_names.clone();
    let unsafe_context = unsafe_context || block.unsafe_context;
    let final_start = block.items.len() - block.final_output_values.len();
    for item in &block.items[..final_start] {
        block_item_type_in_module(
            item,
            &mut scope,
            &mut names,
            &mut folded,
            module,
            modules,
            diagnostics,
            unsafe_context,
        );
    }
    match group_conditional_final_outputs(&module.source, block) {
        Ok(Some(group)) => {
            return sequence_in_module(&group.expression, expected, &scope, &names, &folded, module, modules, diagnostics, unsafe_context);
        }
        Ok(None) => {}
        Err(error) => {
            let diagnostic = error.diagnostic();
            let span = diagnostic.labels[0].span.range;
            diagnostics.push(diagnostic);
            return ScalarOutputSequence { outputs: vec![ScalarOutput { ty: ScalarType::Error, span }], span: block.span };
        }
    }
    let mut outputs = Vec::new();
    for value in &block.final_output_values {
        let start = outputs.len().min(expected.len());
        outputs.extend(
            sequence_in_module(
                &value.value,
                &expected[start..],
                &scope,
                &names,
                &folded,
                module,
                modules,
                diagnostics,
                unsafe_context,
            )
            .outputs,
        );
    }
    if block.final_output_values.is_empty() {
        outputs.push(ScalarOutput {
            ty: ScalarType::Unit,
            span: block.span,
        });
    }
    ScalarOutputSequence {
        outputs,
        span: block.span,
    }
}

pub(super) fn validate_function(
    body: &ScalarBlock,
    expected: &ScalarOutputSequence,
    scope: &BTreeMap<String, ScalarType>,
    names: &BTreeSet<String>,
    folded: &BTreeMap<String, (String, ByteSpan)>,
    program: &ScalarProgram,
    diagnostics: &mut Vec<super::super::Diagnostic>,
) {
    let types = expected
        .outputs
        .iter()
        .map(|output| output.ty.clone())
        .collect::<Vec<_>>();
    let actual = block_sequence(
        body,
        &types,
        scope,
        names,
        folded,
        program,
        diagnostics,
        false,
    );
    if expected.outputs.len() > 1 && actual.outputs.len() != expected.outputs.len() {
        diagnostics.push(diagnostic(
            program,
            "B0004",
            "function output arity does not match its final output list",
            body.span,
        ));
    }
    for (actual, expected) in actual.outputs.iter().zip(&expected.outputs) {
        expect_type(program, &expected.ty, &actual.ty, actual.span, diagnostics);
    }
    if expected.outputs.is_empty() {
        expect_type(
            program,
            &ScalarType::Unit,
            &scalar_call_result(&actual),
            body.span,
            diagnostics,
        );
    }
}

pub(in crate::scalar) fn validate_function_in_module(
    body: &ScalarBlock,
    expected: &ScalarOutputSequence,
    scope: &BTreeMap<String, ScalarType>,
    names: &BTreeSet<String>,
    folded: &BTreeMap<String, (String, ByteSpan)>,
    module: &ScalarModule,
    modules: &[ScalarModule],
    diagnostics: &mut Vec<super::super::Diagnostic>,
) {
    let types = expected
        .outputs
        .iter()
        .map(|output| output.ty.clone())
        .collect::<Vec<_>>();
    let actual = block_sequence_in_module(
        body,
        &types,
        scope,
        names,
        folded,
        module,
        modules,
        diagnostics,
        false,
    );
    if expected.outputs.len() > 1 && actual.outputs.len() != expected.outputs.len() {
        diagnostics.push(module_diagnostic(
            module,
            "B0004",
            "function output arity does not match its final output list",
            body.span,
        ));
    }
    for (actual, expected) in actual.outputs.iter().zip(&expected.outputs) {
        expect_module_type(module, &expected.ty, &actual.ty, actual.span, diagnostics);
    }
    if expected.outputs.is_empty() {
        expect_module_type(
            module,
            &ScalarType::Unit,
            &scalar_call_result(&actual),
            body.span,
            diagnostics,
        );
    }
}
