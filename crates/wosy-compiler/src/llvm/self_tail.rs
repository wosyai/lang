use inkwell::basic_block::BasicBlock;

use super::*;

pub(super) fn eligible(
    state: &EmitState<'_, '_>,
    function: &ScalarFunction,
    cfg: &ScalarReferenceCfg,
) -> bool {
    state
        .body_call_nodes
        .is_some_and(|nodes| nodes.has_self_tail_candidates() && nodes.has_scalar_self_tail_types())
        && state.return_owner_slots.is_empty()
        && state.fresh_return_outputs.is_empty()
        && state.branch_return_outputs.is_empty()
        && state.checked_call_owners.is_empty()
        && state.joined_return_bindings.is_empty()
        && state.joined_assignment_owners.is_empty()
        && state.automatic_return_result_owners.is_empty()
        && state.invalidations.is_empty()
        && scalar_cfg(cfg)
        && scalar_block(&function.body, true)
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
                let staged = expression::emit_contextual_arguments(
                    self.context,
                    state,
                    arguments,
                    self.parameters,
                    self.project,
                )?;
                for ((name, ty), argument) in
                    self.parameter_names.iter().zip(self.parameters).zip(staged)
                {
                    let slot = state.storage[name].0;
                    store_value(self.context, state, slot, ty, argument)?;
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
