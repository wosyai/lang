//! The opt-in WASI preview1 stdout entry for a compiler-lowered comptime result.
//! All typed memory access is inside the evaluator module.
use super::{builder_error, memory, ScalarStruct, ScalarTargetLayout, ScalarType};
use crate::scalar::layout::ScalarLayoutIndex;
use crate::ScalarStructId;
use std::{collections::HashMap, sync::Arc};
use inkwell::{
    builder::Builder, context::Context, module::{Linkage, Module},
    values::{BasicValue, FunctionValue, IntValue, PointerValue},
    attributes::AttributeLoc, IntPredicate,
};

macro_rules! ir { ($instruction:expr) => { $instruction.map_err(builder_error)? }; }

enum Wire {
    Bool,
    Int { signed: bool },
    Struct(Vec<(u32, Arc<Wire>)>),
    Fixed { count: u32, stride: u32, element: Arc<Wire> },
}

// One typed plan associates actual compiler field offsets with semantic
// wire reads; padding itself is never part of the output.
fn wire(
    ty: &ScalarType, layouts: &ScalarLayoutIndex<'_>, structs: &[&ScalarStruct],
    memo: &mut HashMap<ScalarStructId, (Arc<Wire>, u32)>,
) -> Result<(Arc<Wire>, u32), String> {
    let overflow = || "comptime stdout shape exceeds its WASI representation".to_owned();
    match ty {
        ScalarType::Bool => Ok((Arc::new(Wire::Bool), 5)),
        ScalarType::I32 => Ok((Arc::new(Wire::Int { signed: true }), 11)),
        ScalarType::U32 => Ok((Arc::new(Wire::Int { signed: false }), 10)),
        ScalarType::Struct(id) => {
            if let Some(cached) = memo.get(id) { return Ok(cached.clone()); }
            let shape = structs.iter().find(|s| &s.id == id)
                .ok_or("missing comptime result type")?;
            let mut fields = Vec::with_capacity(shape.fields.len());
            let mut maximum = 2u32;
            for field in &shape.fields {
                let offset = layouts.field(&field.id).and_then(|layout| layout.offset)
                    .and_then(|offset| u32::try_from(offset).ok()).ok_or_else(overflow)?;
                let (field, size) = wire(&field.ty, layouts, structs, memo)?;
                maximum = maximum.checked_add(size).and_then(|n| n.checked_add(u32::from(!fields.is_empty())))
                    .ok_or_else(overflow)?;
                fields.push((offset, field));
            }
            let planned = (Arc::new(Wire::Struct(fields)), maximum);
            memo.insert(id.clone(), planned.clone());
            Ok(planned)
        }
        ScalarType::Array { element, length, .. } => {
            let (kind, maximum) = wire(element, layouts, structs, memo)?;
            let count = u32::try_from(*length).map_err(|_| overflow())?;
            let (size, _) = memory::allocation_layout(element, layouts, ScalarTargetLayout::WASM32)?;
            let stride = u32::try_from(size).map_err(|_| overflow())?;
            stride.checked_mul(count).ok_or_else(overflow)?;
            let maximum = maximum.checked_mul(count)
                .and_then(|n| n.checked_add(count.saturating_sub(1)))
                .and_then(|n| n.checked_add(2)).ok_or_else(overflow)?;
            Ok((Arc::new(Wire::Fixed { count, stride, element: kind }), maximum))
        }
        _ => Err("comptime WASI stdout bridge supports bool, i32, u32 and fixed aggregates of those values".into()),
    }
}

pub(super) fn emit<'ctx>(
    context: &'ctx Context, module: &Module<'ctx>, layouts: &ScalarLayoutIndex<'_>,
    structs: &[&ScalarStruct], layout: ScalarTargetLayout,
    main: FunctionValue<'ctx>, evaluate: FunctionValue<'ctx>, ty: &ScalarType, budget: usize,
) -> Result<(), String> {
    if layout != ScalarTargetLayout::WASM32 {
        return Err("comptime stdout bridge requires the explicit WASI preview1 evaluator layout".into());
    }
    let word = context.i32_type();
    if main.get_type() != word.fn_type(&[], false) || main.get_call_conventions() != 0
        || main.count_basic_blocks() != 0
    {
        return Err("comptime stdout bridge requires the ordinary unfilled command entry".into());
    }
    let storage = memory::storage_type(context, ty, layouts, layout)?;
    let abi = evaluate.get_type();
    // Only the direct, already emitted ordinary return ABI participates in
    // this first bridge. Do not reinterpret an indirect or owner result.
    if abi.count_param_types() != 0 || abi.get_return_type() != Some(storage) {
        return Err("comptime stdout bridge does not support this ordinary return ABI".into());
    }
    let (wire, bound) = wire(ty, layouts, structs, &mut HashMap::new())?;
    let bound = bound.checked_add(1).ok_or("comptime stdout line exceeds its WASI representation")?;
    if (bound as usize) > budget { return Err("comptime stdout shape exceeds the selected byte budget".into()); }

    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let byte = context.i8_type();
    let builder = context.create_builder();
    let decimal = module.add_function("__wosy_ct_decimal", context.void_type().fn_type(
        &[word.into(), ptr.into(), ptr.into()], false,
    ), Some(Linkage::Internal));
    let [number, buffer, cursor] = decimal.get_params()[..] else { unreachable!("compiler-defined decimal signature"); };
    let (n, buffer, cursor) = (number.into_int_value(), buffer.into_pointer_value(), cursor.into_pointer_value());
    builder.position_at_end(context.append_basic_block(decimal, "entry"));
    let digits = Encoder { context, builder: &builder, function: decimal, decimal, buffer, cursor };
    let ten = word.const_int(10, false);
    let quotient = ir!(builder.build_int_unsigned_div(n, ten, "quotient"));
    let more = ir!(builder.build_int_compare(IntPredicate::NE, quotient, word.const_zero(), "more"));
    digits.then(more, || {
        ir!(builder.build_call(decimal, &[quotient.into(), buffer.into(), cursor.into()], ""));
        Ok(())
    })?;
    let remainder = ir!(builder.build_int_unsigned_rem(n, ten, "remainder"));
    let ascii = ir!(builder.build_int_add(remainder, word.const_int(b'0' as u64, false), "ascii"));
    digits.put(ir!(builder.build_int_truncate(ascii, byte, "digit")))?;
    ir!(builder.build_return(None));

    builder.position_at_end(context.append_basic_block(main, "entry"));
    let result = ir!(builder.build_call(evaluate, &[], "comptime.value"))
        .try_as_basic_value().basic().ok_or("ordinary comptime evaluator returned no value")?;
    let result_slot = ir!(builder.build_alloca(storage, "comptime.result"));
    ir!(builder.build_store(result_slot, result));
    let wrapper = Encoder {
        context, builder: &builder, function: main, decimal,
        buffer: ir!(builder.build_array_alloca(byte, word.const_int(bound as u64, false), "stdout.buffer")),
        cursor: ir!(builder.build_alloca(word, "stdout.length")),
    };
    ir!(builder.build_store(wrapper.cursor, word.const_zero()));
    wrapper.value(&wire, result_slot)?;
    wrapper.constant(b'\n')?;
    wrapper.finish(module)
}

struct Encoder<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    function: FunctionValue<'ctx>,
    decimal: FunctionValue<'ctx>,
    buffer: PointerValue<'ctx>,
    cursor: PointerValue<'ctx>,
}

impl<'ctx> Encoder<'_, 'ctx> {
    fn word(&self, n: u32) -> IntValue<'ctx> { self.context.i32_type().const_int(n as u64, false) }

    fn address(&self, base: PointerValue<'ctx>, offset: IntValue<'ctx>) -> Result<PointerValue<'ctx>, String> {
        // Each address is from checked ordinary result storage or the
        // output buffer's precomputed typed maximum.
        unsafe { self.builder.build_gep(self.context.i8_type(), base, &[offset], "byte.address") }
            .map_err(builder_error)
    }

    fn put(&self, byte: IntValue<'ctx>) -> Result<(), String> {
        let b = self.builder;
        let offset = ir!(b.build_load(self.context.i32_type(), self.cursor, "stdout.offset")).into_int_value();
        ir!(b.build_store(self.address(self.buffer, offset)?, byte));
        let next = ir!(b.build_int_add(offset, self.word(1), "stdout.next"));
        ir!(b.build_store(self.cursor, next));
        Ok(())
    }

    fn constant(&self, byte: u8) -> Result<(), String> {
        self.put(self.context.i8_type().const_int(byte as u64, false))
    }

    fn then(&self, condition: IntValue<'ctx>, body: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let yes = self.context.append_basic_block(self.function, "wire.then");
        let after = self.context.append_basic_block(self.function, "wire.after");
        ir!(self.builder.build_conditional_branch(condition, yes, after));
        self.builder.position_at_end(yes);
        body()?;
        ir!(self.builder.build_unconditional_branch(after));
        self.builder.position_at_end(after);
        Ok(())
    }

    fn load(&self, from: PointerValue<'ctx>, bits: u32) -> Result<IntValue<'ctx>, String> {
        let ty = if bits == 1 { self.context.bool_type() } else { self.context.i32_type() };
        let value = ir!(self.builder.build_load(ty, from, "wire.field"));
        value.as_instruction_value().ok_or("missing wire load")?.set_alignment(1).map_err(|error| error.to_string())?;
        Ok(value.into_int_value())
    }

    fn value(&self, value: &Wire, from: PointerValue<'ctx>) -> Result<(), String> {
        let b = self.builder;
        match value {
            Wire::Bool => {
                let condition = self.load(from, 1)?;
                let no = ir!(b.build_not(condition, "wire.false"));
                self.then(condition, || b"true".iter().try_for_each(|&byte| self.constant(byte)))?;
                self.then(no, || b"false".iter().try_for_each(|&byte| self.constant(byte)))?;
            }
            Wire::Int { signed } => {
                let mut value = self.load(from, 32)?;
                if *signed {
                    let negative = ir!(b.build_int_compare(IntPredicate::SLT, value, self.word(0), "wire.negative"));
                    self.then(negative, || self.constant(b'-'))?;
                    // Unsigned magnitude, including i32::MIN; no nsw flag.
                    let negated = ir!(b.build_int_sub(self.word(0), value, "wire.magnitude"));
                    value = ir!(b.build_select(negative, negated, value, "wire.absolute")).into_int_value();
                }
                ir!(b.build_call(self.decimal, &[value.into(), self.buffer.into(), self.cursor.into()], ""));
            }
            Wire::Struct(fields) => {
                self.constant(b'[')?;
                for (index, (offset, field)) in fields.iter().enumerate() {
                    if index != 0 { self.constant(b',')?; }
                    self.value(field, self.address(from, self.word(*offset))?)?;
                }
                self.constant(b']')?;
            }
            Wire::Fixed { count, stride, element } => {
                self.constant(b'[')?;
                let before = b.get_insert_block().ok_or("missing wire loop predecessor")?;
                let check = self.context.append_basic_block(self.function, "wire.array.check");
                let item = self.context.append_basic_block(self.function, "wire.array.element");
                let end = self.context.append_basic_block(self.function, "wire.array.end");
                ir!(b.build_unconditional_branch(check));
                b.position_at_end(check);
                let step = ir!(b.build_phi(self.context.i32_type(), "wire.array.index"));
                step.add_incoming(&[(&self.word(0), before)]);
                let index = step.as_basic_value().into_int_value();
                let more = ir!(b.build_int_compare(IntPredicate::ULT, index, self.word(*count), "wire.array.more"));
                ir!(b.build_conditional_branch(more, item, end));
                b.position_at_end(item);
                let comma = ir!(b.build_int_compare(IntPredicate::NE, index, self.word(0), "wire.array.comma"));
                self.then(comma, || self.constant(b','))?;
                let offset = ir!(b.build_int_mul(index, self.word(*stride), "wire.array.offset"));
                self.value(element, self.address(from, offset)?)?;
                let next = ir!(b.build_int_add(index, self.word(1), "wire.array.next"));
                let body = b.get_insert_block().ok_or("missing wire loop body")?;
                ir!(b.build_unconditional_branch(check));
                step.add_incoming(&[(&next, body)]);
                b.position_at_end(end);
                self.constant(b']')?;
            }
        }
        Ok(())
    }

    fn finish(&self, module: &Module<'ctx>) -> Result<(), String> {
        let b = self.builder;
        let word = self.context.i32_type();
        let ptr = self.context.ptr_type(inkwell::AddressSpace::default());
        // Existing preview1 fd_write ABI: fd, one {pointer,length} iovec,
        // count, and byte-count out. This is not a p2/component import.
        let sink = module.add_function("fd_write", word.fn_type(&[word.into(), ptr.into(), word.into(), ptr.into()], false), None);
        for (key, value) in [("wasm-import-module", "wasi_snapshot_preview1"), ("wasm-import-name", "fd_write")] {
            sink.add_attribute(AttributeLoc::Function, self.context.create_string_attribute(key, value));
        }
        let iovec = ir!(b.build_alloca(word.array_type(2), "stdout.iovec"));
        let length = self.address(iovec, self.word(4))?; // WASI32 iovec's second i32
        let written = ir!(b.build_alloca(word, "stdout.written"));
        let total = ir!(b.build_load(word, self.cursor, "stdout.total")).into_int_value();
        let before = b.get_insert_block().ok_or("missing stdout predecessor")?;
        let send = self.context.append_basic_block(self.function, "stdout.send");
        let advance = self.context.append_basic_block(self.function, "stdout.advance");
        let failure = self.context.append_basic_block(self.function, "stdout.failure");
        let done = self.context.append_basic_block(self.function, "stdout.done");
        ir!(b.build_unconditional_branch(send));
        b.position_at_end(send);
        let offset = ir!(b.build_phi(word, "stdout.written.total"));
        offset.add_incoming(&[(&self.word(0), before)]);
        let sent = offset.as_basic_value().into_int_value();
        let remaining = ir!(b.build_int_sub(total, sent, "stdout.remaining"));
        let address = ir!(b.build_ptr_to_int(self.address(self.buffer, sent)?, word, "stdout.data"));
        ir!(b.build_store(iovec, address));
        ir!(b.build_store(length, remaining));
        ir!(b.build_store(written, self.word(0)));
        let status = ir!(b.build_call(sink, &[self.word(1).into(), iovec.into(), self.word(1).into(), written.into()], "stdout.status"))
            .try_as_basic_value().basic().ok_or("WASI preview1 write returned no status")?.into_int_value();
        let count = ir!(b.build_load(word, written, "stdout.count")).into_int_value();
        let clear = ir!(b.build_int_compare(IntPredicate::EQ, status, self.word(0), "stdout.ok"));
        let nonzero = ir!(b.build_int_compare(IntPredicate::NE, count, self.word(0), "stdout.progress"));
        let within = ir!(b.build_int_compare(IntPredicate::ULE, count, remaining, "stdout.within"));
        let valid = ir!(b.build_and(nonzero, within, "stdout.valid.count"));
        let ok = ir!(b.build_and(clear, valid, "stdout.valid.write"));
        ir!(b.build_conditional_branch(ok, advance, failure));
        b.position_at_end(advance);
        let next = ir!(b.build_int_add(sent, count, "stdout.next.write"));
        offset.add_incoming(&[(&next, advance)]);
        let complete = ir!(b.build_int_compare(IntPredicate::EQ, next, total, "stdout.complete"));
        ir!(b.build_conditional_branch(complete, done, send));
        b.position_at_end(failure);
        ir!(b.build_return(Some(&self.word(1))));
        b.position_at_end(done);
        ir!(b.build_return(Some(&self.word(0))));
        Ok(())
    }
}
