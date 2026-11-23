use inkwell::basic_block::BasicBlock;

use super::*;

pub(super) fn eligible(
    context: &Context,
    state: &EmitState<'_, '_>,
    value: FunctionValue<'_>,
    function: &ScalarFunction,
    cfg: &ScalarReferenceCfg,
) -> bool {
    state.prepared_render
        && state.body_call_nodes.is_some_and(|nodes| {
            nodes.has_checked_self_tail()
                || (nodes.has_self_tail_candidates() && nodes.has_scalar_self_tail_types())
        })
        && state.return_owner_slots.is_empty()
        && state.fresh_return_outputs.is_empty()
        && state.branch_return_outputs.is_empty()
        && call_owners_allow_reuse(state.body_call_nodes, cfg, &state.checked_call_owners)
        && state.joined_return_bindings.is_empty()
        && state.joined_assignment_owners.is_empty()
        && state.automatic_return_result_owners.is_empty()
        && state.invalidations.is_empty()
        && (state
            .body_call_nodes
            .is_some_and(CallableBodyCallNodes::has_checked_self_tail)
            || (scalar_cfg(cfg) && scalar_block(&function.body, true)))
        && arguments_are_representable(context, state, value, function)
}

pub(super) fn call_owners_allow_reuse(
    nodes: Option<&CallableBodyCallNodes<'_>>,
    cfg: &ScalarReferenceCfg,
    owners: &BTreeMap<(u32, u32, usize), CheckedCallOwner>,
) -> bool {
    let Some(nodes) = nodes.filter(|nodes| nodes.has_checked_self_tail()) else {
        return owners.is_empty();
    };
    let Some(points) = nodes.checked_primitive_outputs(cfg) else {
        return false;
    };
    let owner_key = |id: crate::scalar::CfgPointId| match &cfg.points[id.0].kind {
        CfgPointKind::Call {
            expression: ScalarExpression::Call { span, .. },
            output: 0,
            ..
        } if cfg.points[id.0].kind == cfg.initial_points[id.0].kind => {
            Some((span.start, span.end, 0))
        }
        _ => None,
    };
    let [Some(first), Some(second)] = points.map(owner_key) else {
        return false;
    };
    owners.len() == 2
        && first != second
        && [first, second]
            .iter()
            .all(|key| matches!(owners.get(key), Some(CheckedCallOwner::Borrowed)))
}

fn arguments_are_representable(
    context: &Context,
    state: &EmitState<'_, '_>,
    value: FunctionValue<'_>,
    function: &ScalarFunction,
) -> bool {
    let ScalarType::Callable { parameters, .. } = &function.signature else {
        return false;
    };
    let nodes = state.body_call_nodes.expect("prepared call nodes");
    (!nodes.has_checked_self_tail()
        || value.get_type().get_return_type() == Some(context.i32_type().into()))
        && abi_parameters_match(context, state.target_layout, value, parameters)
        && nodes.self_tail_candidates().all(|call| {
            let ScalarExpression::Call { arguments, .. } = call else {
                return false;
            };
            arguments.len() == parameters.len()
                && arguments.iter().zip(parameters).all(|(argument, ty)| {
                    argument_is_representable(context, state, argument, ty, true)
                })
        })
}

fn abi_parameters_match<'ctx>(
    context: &'ctx Context,
    layout: ScalarTargetLayout,
    function: FunctionValue<'ctx>,
    parameters: &[ScalarType],
) -> bool {
    let abi = function.get_type().get_param_types();
    abi.len() == parameters.len()
        && abi.iter().zip(parameters).all(|(actual, ty)| {
            basic_type(context, ty, layout)
                .is_ok_and(|expected| BasicMetadataTypeEnum::from(expected) == *actual)
        })
}

fn argument_is_representable(
    context: &Context,
    state: &EmitState<'_, '_>,
    argument: &ScalarExpression,
    expected: &ScalarType,
    checked_arithmetic: bool,
) -> bool {
    let nodes = state.body_call_nodes.expect("prepared call nodes");
    if nodes.expression_type(argument) != Some(expected) {
        return false;
    }
    match argument {
        ScalarExpression::Name { name, .. } if name != "null" => {
            state
                .storage
                .get(name)
                .map(|(_, ty)| ty)
                .or_else(|| state.globals.get(name).map(|(_, ty)| ty))
                == Some(expected)
        }
        ScalarExpression::Integer { .. } => integer_width(expected).is_some(),
        ScalarExpression::Float { .. } => matches!(expected, ScalarType::F32 | ScalarType::F64),
        ScalarExpression::Boolean { .. } => *expected == ScalarType::Bool,
        ScalarExpression::Char { .. } => *expected == ScalarType::Char,
        ScalarExpression::Binary {
            operator: BinaryOperator::Add | BinaryOperator::Subtract,
            left,
            right,
            ..
        } => {
            checked_arithmetic
                && integer_width(expected).is_some()
                && argument_is_representable(context, state, left, expected, true)
                && argument_is_representable(context, state, right, expected, true)
        }
        ScalarExpression::Call { arguments, .. } => {
            let Ok(Some(PreparedCall::Symbol(symbol))) = state.prepared_call(argument) else {
                return false;
            };
            let Some(ScalarType::Callable {
                parameters,
                outputs,
            }) = state.signatures.get(&symbol)
            else {
                return false;
            };
            let [result] = outputs.outputs.as_slice() else {
                return false;
            };
            let Some(&callee) = state.functions.get(&symbol) else {
                return false;
            };
            &result.ty == expected
                && basic_type(context, expected, state.target_layout)
                    .is_ok_and(|ty| callee.get_type().get_return_type() == Some(ty))
                && abi_parameters_match(context, state.target_layout, callee, parameters)
                && arguments.len() == parameters.len()
                && arguments
                    .iter()
                    .zip(parameters)
                    .all(|(child, ty)| argument_is_representable(context, state, child, ty, false))
        }
        _ => false,
    }
}

fn scalar_cfg(cfg: &ScalarReferenceCfg) -> bool {
    cfg.allocations.is_empty()
        && cfg.facts.is_empty()
        && cfg.specialized_policies.is_empty()
        && cfg.points.iter().chain(&cfg.initial_points).all(|point| {
            point
                .possible_origins
                .as_ref()
                .is_none_or(BTreeSet::is_empty)
                && matches!(
                    point.kind,
                    CfgPointKind::Entry
                        | CfgPointKind::ScopeEnter
                        | CfgPointKind::ScopeExit
                        | CfgPointKind::Evaluate(_)
                        | CfgPointKind::Read { .. }
                        | CfgPointKind::Call { .. }
                        | CfgPointKind::Branch { .. }
                        | CfgPointKind::Join
                        | CfgPointKind::ReturnOutput { checked: false, .. }
                        | CfgPointKind::Return
                        | CfgPointKind::ConditionalOutput { .. }
                )
        })
}

fn scalar_block(block: &ScalarBlock, returned: bool) -> bool {
    let outputs = block.final_output_values.len();
    if outputs > 1 || (returned && outputs != 1) {
        return false;
    }
    let final_start = block.items.len() - outputs;
    block.items.iter().enumerate().all(|(index, item)| {
        matches!(item, ScalarBlockItem::Expression(expression)
            if scalar_expression(expression, returned && index >= final_start))
    })
}

fn scalar_expression(expression: &ScalarExpression, returned: bool) -> bool {
    match expression {
        ScalarExpression::Name { .. }
        | ScalarExpression::Integer { .. }
        | ScalarExpression::Float { .. }
        | ScalarExpression::Boolean { .. }
        | ScalarExpression::Char { .. } => true,
        ScalarExpression::Unary { operand, .. } => scalar_expression(operand, false),
        ScalarExpression::Binary { left, right, .. } => {
            scalar_expression(left, false) && scalar_expression(right, false)
        }
        ScalarExpression::Call { arguments, .. } => arguments
            .iter()
            .all(|argument| scalar_expression(argument, false)),
        ScalarExpression::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            scalar_expression(condition, false)
                && scalar_block(then_branch, returned)
                && scalar_block(else_branch, returned)
        }
        ScalarExpression::UnitIf {
            condition,
            then_branch,
            ..
        } => scalar_expression(condition, false) && scalar_block(then_branch, false),
        ScalarExpression::Block(block) => scalar_block(block, returned),
        _ => false,
    }
}

pub(super) fn emit<'ctx, 'module>(
    context: &'ctx Context,
    state: &mut EmitState<'ctx, 'module>,
    value: FunctionValue<'ctx>,
    function: &ScalarFunction,
    project: Option<ProjectCallScope<'_>>,
) -> Result<(), String> {
    let ScalarType::Callable {
        outputs,
        parameters,
    } = &function.signature
    else {
        unreachable!("prepared self-tail callable")
    };
    let header = context.append_basic_block(value, "self_tail.header");
    state
        .builder
        .build_unconditional_branch(header)
        .map_err(builder_error)?;
    state.builder.position_at_end(header);
    ReturnSpine {
        context,
        value,
        parameter_names: &function.parameters,
        parameters,
        outputs,
        header,
        project,
    }
    .block(state, &function.body)
}

struct ReturnSpine<'ctx, 'a> {
    context: &'ctx Context,
    value: FunctionValue<'ctx>,
    parameter_names: &'a [String],
    parameters: &'a [ScalarType],
    outputs: &'a crate::ScalarOutputSequence,
    header: BasicBlock<'ctx>,
    project: Option<ProjectCallScope<'a>>,
}

impl<'ctx> ReturnSpine<'ctx, '_> {
    fn block(&self, state: &mut EmitState<'ctx, '_>, block: &ScalarBlock) -> Result<(), String> {
        let (last, prefixes) = block
            .items
            .split_last()
            .expect("single returned expression");
        for item in prefixes {
            let ScalarBlockItem::Expression(expression) = item else {
                unreachable!("scalar expression prefix")
            };
            self.expression(state, expression)?;
        }
        let ScalarBlockItem::Expression(expression) = last else {
            unreachable!("scalar returned expression")
        };
        match expression {
            ScalarExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let condition = take_basic(self.expression(state, condition)?)?.into_int_value();
                let then_block = self
                    .context
                    .append_basic_block(self.value, "self_tail.then");
                let else_block = self
                    .context
                    .append_basic_block(self.value, "self_tail.else");
                state
                    .builder
                    .build_conditional_branch(condition, then_block, else_block)
                    .map_err(builder_error)?;
                state.builder.position_at_end(then_block);
                self.block(state, then_branch)?;
                state.builder.position_at_end(else_block);
                self.block(state, else_branch)
            }
            ScalarExpression::Block(block) => self.block(state, block),
            ScalarExpression::Call { arguments, .. }
                if state
                    .body_call_nodes
                    .expect("prepared call nodes")
                    .is_self_tail_candidate(expression) =>
            {
                assert_eq!(arguments.len(), self.parameters.len());
                let staged = arguments
                    .iter()
                    .map(|argument| take_basic(self.argument(state, argument)?))
                    .collect::<Result<Vec<_>, _>>()?;
                let abi = self.value.get_type().get_param_types();
                if staged.len() != abi.len()
                    || staged
                        .iter()
                        .zip(&abi)
                        .any(|(value, ty)| BasicMetadataTypeEnum::from(value.get_type()) != *ty)
                {
                    return Err(
                        "self-tail staged argument types differ from the function ABI".into(),
                    );
                }
                for ((name, ty), argument) in
                    self.parameter_names.iter().zip(self.parameters).zip(staged)
                {
                    let slot = state.storage.get(name).expect("self-tail parameter storage").0;
                    store_value(self.context, state, slot, ty, EmitValue::Basic(argument))?;
                }
                state
                    .builder
                    .build_unconditional_branch(self.header)
                    .map_err(builder_error)?;
                Ok(())
            }
            _ => {
                let ty = &self.outputs.outputs[0].ty;
                let value = match self.project {
                    Some(scope) => emit_project_typed_expression(
                        self.context,
                        state,
                        expression,
                        ty,
                        scope.module,
                        scope.modules,
                    )?,
                    None => emit_typed_expression(self.context, state, expression, ty)?,
                };
                emit_return(self.context, state, value, self.outputs)
            }
        }
    }

    fn argument(
        &self,
        state: &mut EmitState<'ctx, '_>,
        argument: &ScalarExpression,
    ) -> Result<EmitValue<'ctx>, String> {
        let ty = state
            .body_call_nodes
            .and_then(|nodes| nodes.expression_type(argument))
            .expect("preflighted self-tail argument type");
        if let ScalarExpression::Binary {
            operator,
            left,
            right,
            ..
        } = argument
        {
            let left = self.argument(state, left)?;
            let right = self.argument(state, right)?;
            return expression::emit_binary_values(state, operator, left, right, false);
        }
        match self.project {
            Some(scope) => emit_project_typed_expression(
                self.context,
                state,
                argument,
                ty,
                scope.module,
                scope.modules,
            ),
            None => emit_typed_expression(self.context, state, argument, ty),
        }
    }

    fn expression(
        &self,
        state: &mut EmitState<'ctx, '_>,
        expression: &ScalarExpression,
    ) -> Result<EmitValue<'ctx>, String> {
        match self.project {
            Some(scope) => emit_project_expression(
                self.context,
                state,
                expression,
                scope.module,
                scope.modules,
            ),
            None => emit_expression(self.context, state, expression),
        }
    }
}
