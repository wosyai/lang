use inkwell::values::BasicValue;
use super::{expression::emit_integer_binary, memory::{builder_error, integer_constant, integer_type}, *};
use crate::scalar::runtime_enum_match::{ArmResult, IntegerExpression, IntegerKind, Operand, Plan, Predicate, TypedSelector};
use crate::{ScalarEnumPayloadLayout, ScalarEnumPayloadSlotLayout};
use wosy_syntax::ByteSpan;

/// Emit one exhaustive value region and leave the builder at its continuation.
pub(super) fn emit_values<'ctx>(
    context: &'ctx Context, builder: &Builder<'ctx>, function: FunctionValue<'ctx>,
    source: &ScalarModule, callable: &ScalarFunction, plan: &Plan, target: ScalarTargetLayout,
) -> Result<Vec<BasicValueEnum<'ctx>>, String> {
    if !plan.authenticates(&source.source, callable) || source.target_layout != target {
        return Err("invalid prepared runtime enum match owner".to_owned());
    }
    let layout = source.layouts.enum_payload(&plan.enumeration)
        .filter(|layout| layout.target == target).ok_or("missing freshly prepared runtime enum layout")?;
    emit_region(context, builder, function, layout, plan)
}

pub(super) fn emit_initializer<'ctx>(
    context: &'ctx Context, state: &mut EmitState<'ctx, '_>,
    site: &crate::scalar::runtime_enum_match::Occurrence,
) -> Result<EmitValue<'ctx>, String> {
    let (authored, layout) = state.runtime_enum_initializer
        .filter(|(authored, layout)| state.prepared_render && std::sync::Arc::ptr_eq(&authored.0, &site.0)
            && layout.target == state.target_layout)
        .ok_or("runtime match is not the exact prepared scalar initializer")?;
    let function = state.builder.get_insert_block().and_then(|block| block.get_parent())
        .ok_or("missing ordinary runtime match function")?;
    let outputs = emit_region(context, state.builder, function, layout, &authored.0)?;
    let [value] = outputs.as_slice() else { return Err("runtime match initializer is not scalar".into()); };
    Ok(EmitValue::Basic(*value))
}

fn emit_region<'ctx>(
    context: &'ctx Context, builder: &Builder<'ctx>, function: FunctionValue<'ctx>,
    layout: &ScalarEnumPayloadLayout, plan: &Plan,
) -> Result<Vec<BasicValueEnum<'ctx>>, String> {
    let size = u32::try_from(layout.layout.size).map_err(|_| "runtime enum storage exceeds LLVM array limit")?;
    let alignment = u32::try_from(layout.layout.alignment).ok().filter(|value| value.is_power_of_two())
        .ok_or("invalid runtime enum storage alignment")?;
    if size < 4 || alignment < 4 || plan.arms.len() != layout.variants.len() {
        return Err("invalid runtime enum storage or exhaustive arm count".into());
    }
    // A lexical constructor initializes this exact slot. The match only
    // reads it: no reconstruction or second constructor path.
    let storage_name = if plan.local.is_some() { "enum.local.storage" } else { "enum.match.storage" };
    let storage = builder.build_alloca(context.i8_type().array_type(size), storage_name).map_err(builder_error)?;
    storage.as_instruction_value().ok_or("missing runtime enum storage instruction")?
        .set_alignment(alignment).map_err(|error| format!("{error:?}"))?;
    let arguments = plan.input_spans().map(|span| (span.start, span.end)).zip(function.get_param_iter())
        .collect::<BTreeMap<_, _>>();
    let join = context.append_basic_block(function, "enum.match.dispatch");
    let lowering = Lowering { context, builder, function, storage, layout, arguments };
    lowering.selector(&plan.selector, join)?;
    builder.position_at_end(join);
    let tag = builder.build_load(context.i32_type(), storage, "enum.match.tag").map_err(builder_error)?.into_int_value();
    let invalid = context.append_basic_block(function, "enum.match.invalid");
    let mut cases = Vec::with_capacity(plan.arms.len());
    let mut seen = BTreeSet::new();
    for arm in &plan.arms {
        if !seen.insert(arm.tag) { return Err("duplicate prepared runtime enum arm".into()); }
        let block = context.append_basic_block(function, "enum.match.arm");
        cases.push((context.i32_type().const_int(u64::from(arm.tag), false), block));
    }
    builder.build_switch(tag, invalid, &cases).map_err(builder_error)?;
    builder.position_at_end(invalid);
    builder.build_unreachable().map_err(builder_error)?;
    let continuation = context.append_basic_block(function, "enum.match.continue");
    builder.position_at_end(continuation);
    let results = plan.outputs.iter().map(|ty| {
        builder.build_phi(integer_type(context, ty)?, "enum.match.result").map_err(builder_error)
    }).collect::<Result<Vec<_>, _>>()?;
    for (arm, (_, block)) in plan.arms.iter().zip(cases) {
        builder.position_at_end(block);
        let variant = layout.variants.get(arm.tag as usize).filter(|variant| variant.tag == arm.tag)
            .ok_or("invalid prepared runtime enum arm tag")?;
        if variant.slots.len() != arm.bindings.len() || !arm.bindings.iter().zip(&variant.slots)
            .enumerate().all(|(index, (binding, slot))| binding.slot == index && binding.ty == slot.source_type.ty)
        {
            return Err("prepared runtime enum arm slot mismatch".into());
        }
        if arm.results.len() != plan.outputs.len() {
            return Err("prepared runtime enum output arity mismatch".into());
        }
        let values = arm.results.iter().zip(&plan.outputs).map(|(result, ty)| {
            match result {
                ArmResult::Literal { value, .. } => literal(context, ty, value),
                ArmResult::Expression(expression) if &expression.ty == ty => lowering.integer(expression, Some(&variant.slots)),
                ArmResult::Expression(_) => Err("invalid prepared runtime arm result type".into()),
                ArmResult::Payload { slot, .. } => {
                    let selected = variant.slots.get(*slot).filter(|slot| &slot.source_type.ty == ty)
                        .ok_or("missing prepared runtime enum projection")?;
                    builder.build_load(integer_type(context, ty)?, lowering.address(selected)?, "enum.match.payload")
                        .map_err(builder_error).map(|value| value.into_int_value())
                }
            }
        }).collect::<Result<Vec<_>, String>>()?;
        let from = builder.get_insert_block().ok_or("missing runtime enum arm result block")?;
        builder.build_unconditional_branch(continuation).map_err(builder_error)?;
        for (joined, value) in results.iter().zip(&values) {
            joined.add_incoming(&[(value, from)]);
        }
    }
    builder.position_at_end(continuation);
    Ok(results.into_iter().map(|result| result.as_basic_value()).collect())
}

fn literal<'ctx>(context: &'ctx Context, ty: &ScalarType, value: &num_bigint::BigInt)
    -> Result<inkwell::values::IntValue<'ctx>, String>
{
    if value.sign() == num_bigint::Sign::Minus {
        Ok(integer_constant(context, ty, &-value)?.const_neg())
    } else {
        integer_constant(context, ty, value)
    }
}

struct Lowering<'owner, 'ctx> {
    context: &'ctx Context,
    builder: &'owner Builder<'ctx>,
    function: FunctionValue<'ctx>,
    storage: PointerValue<'ctx>,
    layout: &'owner ScalarEnumPayloadLayout,
    arguments: BTreeMap<(u32, u32), BasicValueEnum<'ctx>>,
}

impl<'ctx> Lowering<'_, 'ctx> {
    fn actual(&self, span: ByteSpan, ty: inkwell::types::IntType<'ctx>)
        -> Result<inkwell::values::IntValue<'ctx>, String>
    {
        match self.arguments.get(&(span.start, span.end)) {
            Some(BasicValueEnum::IntValue(value)) if value.get_type() == ty => Ok(*value),
            _ => Err("missing prepared runtime enum formal".into()),
        }
    }

    fn address(&self, slot: &ScalarEnumPayloadSlotLayout) -> Result<PointerValue<'ctx>, String> {
        let offset = self.layout.payload_offset.checked_add(slot.offset_in_payload)
            .filter(|offset| *offset >= 4 && offset.checked_add(slot.layout.size)
                .is_some_and(|end| end <= self.layout.layout.size))
            .ok_or("invalid prepared runtime enum slot extent")?;
        let width = integer_type(self.context, &slot.source_type.ty)?.get_bit_width();
        if slot.layout.size != u64::from(width / 8) || !slot.layout.alignment.is_power_of_two()
            || slot.layout.alignment > self.layout.layout.alignment || offset % slot.layout.alignment != 0
        {
            return Err("invalid prepared runtime enum slot alignment".into());
        }
        // Exact offset and full integer extent checked against the fresh allocation.
        unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), self.storage,
            &[self.context.i64_type().const_int(offset, false)], "enum.match.slot") }.map_err(builder_error)
    }

    fn integer(&self, expression: &IntegerExpression, slots: Option<&[ScalarEnumPayloadSlotLayout]>)
        -> Result<inkwell::values::IntValue<'ctx>, String>
    {
        match &expression.kind {
            IntegerKind::Literal(value) => literal(self.context, &expression.ty, value),
            IntegerKind::Formal(declaration) => self.actual(*declaration, integer_type(self.context, &expression.ty)?),
            IntegerKind::Payload(index) => {
                let slot = slots.and_then(|slots| slots.get(*index))
                    .filter(|slot| slot.source_type.ty == expression.ty)
                    .ok_or("missing prepared enum arm payload")?;
                self.builder.build_load(integer_type(self.context, &expression.ty)?, self.address(slot)?, "enum.match.payload")
                    .map_err(builder_error).map(|value| value.into_int_value())
            }
            IntegerKind::Binary { operator, operands } => self.primitive_binary(operator, operands, slots),
        }
    }

    fn primitive_binary(&self, operator: &BinaryOperator, operands: &[IntegerExpression; 2],
        slots: Option<&[ScalarEnumPayloadSlotLayout]>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let [left, right] = operands;
        if left.ty != right.ty { return Err("mismatched prepared runtime enum operator types".into()); }
        let unsigned = matches!(left.ty, ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 | ScalarType::U128);
        let left = self.integer(left, slots)?;
        let right = self.integer(right, slots)?;
        emit_integer_binary(self.builder, operator, left, right, unsigned)
    }

    fn selector(&self, selector: &TypedSelector, join: inkwell::basic_block::BasicBlock<'ctx>) -> Result<(), String> {
        match selector {
            TypedSelector::Constructor { tag, payload, .. } => {
                let variant = self.layout.variants.get(*tag as usize).filter(|variant| variant.tag == *tag)
                    .ok_or("missing prepared runtime enum constructor variant")?;
                if variant.slots.len() != payload.len() { return Err("invalid runtime enum constructor arity".into()); }
                for (slot, (_, operand)) in variant.slots.iter().zip(payload) {
                    let ty = &slot.source_type.ty;
                    let value = match operand {
                        Operand::Literal(value) => literal(self.context, ty, value)?,
                        Operand::Formal(span) => self.actual(*span, integer_type(self.context, ty)?)?,
                        Operand::Expression(expression) if &expression.ty == ty => self.integer(expression, None)?,
                        Operand::Expression(_) => return Err("mismatched prepared runtime enum constructor operand".into()),
                    };
                    self.builder.build_store(self.address(slot)?, value).map_err(builder_error)?;
                }
                self.builder.build_store(self.storage, self.context.i32_type().const_int(u64::from(*tag), false))
                    .map_err(builder_error)?;
                self.builder.build_unconditional_branch(join).map_err(builder_error)?;
            }
            TypedSelector::If { predicate, yes, no } => {
                let condition = match predicate {
                    Predicate::Constant(value) => self.context.bool_type().const_int(u64::from(*value), false),
                    Predicate::Formal { declaration, .. } => self.actual(*declaration, self.context.bool_type())?,
                    Predicate::Compare { operator, operands } => self.primitive_binary(operator, operands, None)?,

                };
                let then_block = self.context.append_basic_block(self.function, "enum.match.select.yes");
                let else_block = self.context.append_basic_block(self.function, "enum.match.select.no");
                self.builder.build_conditional_branch(condition, then_block, else_block).map_err(builder_error)?;
                self.builder.position_at_end(then_block);
                self.selector(yes, join)?;
                self.builder.position_at_end(else_block);
                self.selector(no, join)?;
            }
        }
        Ok(())
    }
}
