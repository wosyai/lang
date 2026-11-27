//! The opt-in WASI preview1 stdout entry for a compiler-lowered comptime result.
//! All typed memory access is inside the evaluator module.
use super::{builder_error, dynamic_return_outputs, memory, ScalarFunction, ScalarReferenceCfg,
    ScalarStruct, ScalarTargetLayout, ScalarType};
use crate::scalar::{layout::ScalarLayoutIndex, ScalarStructFieldId, ScalarStructId};
use wosy_syntax::{SourceIdentity, SourceSpan};
use std::collections::HashMap;
use inkwell::{
    builder::Builder, context::Context, module::{Linkage, Module},
    types::{BasicTypeEnum, FunctionType, IntType},
    values::{BasicValue, FunctionValue, IntValue, PhiValue, PointerValue},
    attributes::AttributeLoc, basic_block::BasicBlock, IntPredicate,
};

macro_rules! ir { ($instruction:expr) => { $instruction.map_err(builder_error)? }; }

#[derive(Clone, Copy)]
pub(crate) struct Wasip1ComptimeOutput<'a> {
    pub(crate) site: &'a SourceSpan,
    pub(crate) ty: &'a ScalarType,
    pub(crate) max_stdout: usize,
    pub(crate) text_fields: &'a HashMap<ScalarStructId, [&'a ScalarStructFieldId; 2]>,
    pub(crate) max_text_bytes: u32,
}

impl Wasip1ComptimeOutput<'_> {
    // The ordinary frame captures this exact selected value before its
    // local result storage is dropped. Pointers inside the admitted text
    // descriptors reference only compiler-emitted immutable literal data.
    // An ordinary legacy pointer to the frame itself never crosses the ABI.
    pub(super) fn captured_type<'ctx>(
        self, context: &'ctx Context, layouts: &ScalarLayoutIndex<'_>, target: ScalarTargetLayout,
        source: &SourceIdentity, function: &ScalarFunction, cfg: &ScalarReferenceCfg,
    ) -> Result<Option<FunctionType<'ctx>>, String> {
        if self.text_fields.is_empty() || self.site.source != *source || self.site.range != function.span {
            return Ok(None);
        }
        if !dynamic_return_outputs(function.span, cfg).is_empty() {
            return Err("WASI comptime text capture does not take ownership sidecars".into());
        }
        let BasicTypeEnum::ArrayType(storage) = memory::storage_type(context, self.ty, layouts, target)? else {
            return Err("WASI comptime text capture requires fixed result storage".into());
        };
        Ok(Some(storage.fn_type(&[], false)))
    }
}

enum Wire {
    Bool,
    Int { signed: bool },
    Utf8 { data: u32, length: u32 },
    Struct(Vec<(u32, usize)>),
    Fixed { count: u32, stride: u32, element: usize },
}

struct WirePlan {
    nodes: Vec<Wire>,
    root: usize,
    bound: u32,
}

// Typed offsets and array strides come from the ordinary compiler layout.
// Postorder rows own only earlier indices, including shared nominal shapes.
fn wire(
    ty: &ScalarType, layouts: &ScalarLayoutIndex<'_>, structs: &[&ScalarStruct],
    output: Wasip1ComptimeOutput<'_>,
) -> Result<WirePlan, String> {
    enum Task<'a> { Enter(&'a ScalarType), Struct(&'a ScalarStruct), Array(u64) }
    #[derive(Clone, Copy)]
    struct Row { id: usize, bound: u32, allocation: u64 }
    enum Visit { Active, Done(Row) }

    let overflow = || "comptime stdout shape exceeds its WASI representation".to_owned();
    let offset = |id: &ScalarStructFieldId| layouts.field(id).and_then(|layout| layout.offset)
        .and_then(|offset| u32::try_from(offset).ok()).ok_or_else(overflow);
    let structs: HashMap<_, _> = structs.iter().map(|structure| (&structure.id, *structure)).collect();
    let allocation = |ty| memory::allocation_layout(ty, layouts, ScalarTargetLayout::WASM32).map(|(size, _)| size);
    let mut memo = HashMap::new();
    let mut tasks = vec![Task::Enter(ty)];
    let mut rows: Vec<Row> = Vec::new();
    let mut nodes = Vec::new();
    while let Some(task) = tasks.pop() {
        let (node, bound, size, done) = match task {
            Task::Enter(kind) => match kind {
                ScalarType::Bool => (Wire::Bool, 5, allocation(kind)?, None),
                ScalarType::I32 => (Wire::Int { signed: true }, 11, allocation(kind)?, None),
                ScalarType::U32 => (Wire::Int { signed: false }, 10, allocation(kind)?, None),
                ScalarType::Struct(id) => {
                    match memo.get(id) {
                        Some(Visit::Active) => return Err("cyclic comptime result layout".into()),
                        Some(Visit::Done(row)) => { rows.push(*row); continue; }
                        None => {}
                    }
                    if let Some([data, length]) = output.text_fields.get(id) {
                        let maximum = output.max_text_bytes.checked_mul(6).and_then(|n| n.checked_add(2))
                            .ok_or_else(overflow)?;
                        (Wire::Utf8 { data: offset(data)?, length: offset(length)? }, maximum, allocation(kind)?, Some(id))
                    } else {
                        let structure = *structs.get(id).ok_or("missing comptime result type")?;
                        memo.insert(id, Visit::Active);
                        tasks.push(Task::Struct(structure));
                        tasks.extend(structure.fields.iter().rev().map(|field| Task::Enter(&field.ty)));
                        continue;
                    }
                }
                ScalarType::Array { element, length, .. } => {
                    tasks.push(Task::Array(*length));
                    tasks.push(Task::Enter(element));
                    continue;
                }
                _ => return Err("comptime WASI stdout bridge supports bool, i32, u32, std.utf8 and fixed aggregates of those values".into()),
            },
            Task::Struct(structure) => {
                let start = rows.len() - structure.fields.len();
                let mut fields = Vec::with_capacity(structure.fields.len());
                let mut bound = 2u32;
                for (field, child) in structure.fields.iter().zip(rows.drain(start..)) {
                    bound = bound.checked_add(child.bound)
                        .and_then(|n| n.checked_add(u32::from(!fields.is_empty()))).ok_or_else(overflow)?;
                    fields.push((offset(&field.id)?, child.id));
                }
                let size = layouts.aggregate(&structure.id).and_then(|structure| structure.layout.as_ref())
                    .map(|layout| layout.size).ok_or("missing comptime result allocation layout")?;
                (Wire::Struct(fields), bound, size, Some(&structure.id))
            }
            Task::Array(length) => {
                let child = rows.pop().expect("array element row is complete");
                let count = u32::try_from(length).map_err(|_| overflow())?;
                let stride = u32::try_from(child.allocation).map_err(|_| overflow())?;
                let size = u64::from(stride.checked_mul(count).ok_or_else(overflow)?);
                let bound = child.bound.checked_mul(count)
                    .and_then(|n| n.checked_add(count.saturating_sub(1)))
                    .and_then(|n| n.checked_add(2)).ok_or_else(overflow)?;
                (Wire::Fixed { count, stride, element: child.id }, bound, size, None)
            }
        };
        let row = Row { id: nodes.len(), bound, allocation: size };
        if let Some(id) = done { memo.insert(id, Visit::Done(row)); }
        nodes.push(node);
        rows.push(row);
    }
    let [row] = rows.as_slice() else { unreachable!("one completed wire root"); };
    Ok(WirePlan { nodes, root: row.id, bound: row.bound })
}

pub(super) fn emit<'ctx>(
    context: &'ctx Context, module: &Module<'ctx>, layouts: &ScalarLayoutIndex<'_>,
    structs: &[&ScalarStruct], layout: ScalarTargetLayout,
    main: FunctionValue<'ctx>, evaluate: FunctionValue<'ctx>, output: Wasip1ComptimeOutput<'_>,
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
    let storage = memory::storage_type(context, output.ty, layouts, layout)?;
    let abi = evaluate.get_type();
    // A returned value has already been captured by the ordinary emitter.
    // Do not reinterpret an indirect or owning return.
    if abi.count_param_types() != 0 || abi.get_return_type() != Some(storage) {
        return Err("comptime stdout bridge does not support this ordinary return ABI".into());
    }
    let wire = wire(output.ty, layouts, structs, output)?;
    let bound = wire.bound.checked_add(1).ok_or("comptime stdout line exceeds its WASI representation")?;
    if (bound as usize) > output.max_stdout { return Err("comptime stdout shape exceeds the selected byte budget".into()); }

    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let byte = context.i8_type();
    let builder = context.create_builder();
    let decimal = module.add_function("__wosy_ct_decimal", context.void_type().fn_type(
        &[word.into(), ptr.into(), ptr.into()], false,
    ), Some(Linkage::Internal));
    let [number, buffer, cursor] = decimal.get_params()[..] else { unreachable!("compiler-defined decimal signature"); };
    let (n, buffer, cursor) = (number.into_int_value(), buffer.into_pointer_value(), cursor.into_pointer_value());
    builder.position_at_end(context.append_basic_block(decimal, "entry"));
    let digits = Encoder { context, builder: &builder, function: decimal, decimal, quote: None, buffer, cursor };
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
    let quote = (!output.text_fields.is_empty()).then(||
        define_quoted_text(context, module, &builder, decimal)).transpose()?;

    builder.position_at_end(context.append_basic_block(main, "entry"));
    let result = ir!(builder.build_call(evaluate, &[], "comptime.value"))
        .try_as_basic_value().basic().ok_or("ordinary comptime evaluator returned no value")?;
    let result_slot = ir!(builder.build_alloca(storage, "comptime.result"));
    ir!(builder.build_store(result_slot, result));
    let wrapper = Encoder {
        context, builder: &builder, function: main, decimal, quote,
        buffer: ir!(builder.build_array_alloca(byte, word.const_int(bound as u64, false), "stdout.buffer")),
        cursor: ir!(builder.build_alloca(word, "stdout.length")),
    };
    ir!(builder.build_store(wrapper.cursor, word.const_zero()));
    wrapper.value(&wire, result_slot)?;
    wrapper.constant(b'\n')?;
    wrapper.finish(module)
}

fn define_quoted_text<'ctx>(
    context: &'ctx Context, module: &Module<'ctx>, b: &Builder<'ctx>, decimal: FunctionValue<'ctx>,
) -> Result<FunctionValue<'ctx>, String> {
    let word = context.i32_type();
    let byte = context.i8_type();
    let u8 = |value| byte.const_int(u64::from(value), false);
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let quote = module.add_function("__wosy_ct_quote_utf8", context.void_type()
        .fn_type(&[ptr.into(), word.into(), ptr.into(), ptr.into()], false), Some(Linkage::Internal));
    let [data, length, buffer, cursor] = quote.get_params()[..] else { unreachable!("compiler text signature"); };
    let (data, length, buffer, cursor) = (data.into_pointer_value(), length.into_int_value(),
        buffer.into_pointer_value(), cursor.into_pointer_value());
    let emit = Encoder { context, builder: b, function: quote, decimal, quote: None, buffer, cursor };
    let entry = context.append_basic_block(quote, "entry");
    let check = context.append_basic_block(quote, "utf8.check");
    let item = context.append_basic_block(quote, "utf8.byte");
    let short = context.append_basic_block(quote, "utf8.short");
    let other = context.append_basic_block(quote, "utf8.other");
    let control = context.append_basic_block(quote, "utf8.control");
    let raw = context.append_basic_block(quote, "utf8.raw");
    let advance = context.append_basic_block(quote, "utf8.advance");
    let end = context.append_basic_block(quote, "utf8.end");

    b.position_at_end(entry);
    emit.constant(b'"')?;
    ir!(b.build_unconditional_branch(check));
    b.position_at_end(check);
    let step = ir!(b.build_phi(word, "utf8.index"));
    step.add_incoming(&[(&word.const_zero(), entry)]);
    let index = step.as_basic_value().into_int_value();
    let more = ir!(b.build_int_compare(IntPredicate::ULT, index, length, "utf8.more"));
    ir!(b.build_conditional_branch(more, item, end));

    b.position_at_end(item);
    let value = emit.load(emit.address(data, index)?, byte)?;
    let suffix = [(b'\n', b'n'), (b'\r', b'r'), (b'\t', b't')].into_iter()
        .try_fold(value, |suffix, (from, to)| -> Result<_, String> {
            let selected = ir!(b.build_int_compare(IntPredicate::EQ, value, u8(from), "utf8.short.kind"));
            Ok(ir!(b.build_select(selected, u8(to), suffix, "utf8.short.byte")).into_int_value())
        })?;
    let special = [b'"', b'\\', b'\n', b'\r', b'\t'].map(|value| (u8(value), short));
    ir!(b.build_switch(value, other, &special));
    b.position_at_end(short);
    emit.constant(b'\\')?;
    emit.put(suffix)?;
    ir!(b.build_unconditional_branch(advance));

    b.position_at_end(other);
    let escaped = ir!(b.build_int_compare(IntPredicate::ULT, value, u8(0x20), "utf8.c0"));
    ir!(b.build_conditional_branch(escaped, control, raw));
    b.position_at_end(control);
    b"\\u00".iter().try_for_each(|&byte| emit.constant(byte))?;
    let high = ir!(b.build_right_shift(value, u8(4), false, "utf8.nibble.high"));
    let low = ir!(b.build_and(value, u8(15), "utf8.nibble.low"));
    for digit in [high, low] {
        let decimal = ir!(b.build_int_compare(IntPredicate::ULT, digit, u8(10), "utf8.hex.numeric"));
        let prefix = ir!(b.build_select(decimal, u8(b'0'), u8(b'a' - 10), "utf8.hex.prefix")).into_int_value();
        emit.put(ir!(b.build_int_add(digit, prefix, "utf8.hex")))?;
    }
    ir!(b.build_unconditional_branch(advance));

    b.position_at_end(raw);
    emit.put(value)?;
    ir!(b.build_unconditional_branch(advance));
    b.position_at_end(advance);
    let next = ir!(b.build_int_add(index, word.const_int(1, false), "utf8.next"));
    step.add_incoming(&[(&next, advance)]);
    ir!(b.build_unconditional_branch(check));
    b.position_at_end(end);
    emit.constant(b'"')?;
    ir!(b.build_return(None));
    Ok(quote)
}

struct Encoder<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    function: FunctionValue<'ctx>,
    decimal: FunctionValue<'ctx>,
    quote: Option<FunctionValue<'ctx>>,
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

    fn load(&self, from: PointerValue<'ctx>, ty: IntType<'ctx>) -> Result<IntValue<'ctx>, String> {
        let value = ir!(self.builder.build_load(ty, from, "wire.field"));
        value.as_instruction_value().ok_or("missing wire load")?.set_alignment(1).map_err(|error| error.to_string())?;
        Ok(value.into_int_value())
    }

    fn value(&self, plan: &WirePlan, from: PointerValue<'ctx>) -> Result<(), String> {
        enum Task<'a, 'ctx> {
            Value(usize, PointerValue<'ctx>),
            Fields(&'a [(u32, usize)], PointerValue<'ctx>, bool),
            ArrayEnd(PhiValue<'ctx>, IntValue<'ctx>, BasicBlock<'ctx>, BasicBlock<'ctx>),
        }
        let b = self.builder;
        let mut tasks = vec![Task::Value(plan.root, from)];
        while let Some(task) = tasks.pop() {
            let (value, from) = match task {
                Task::Value(value, from) => (&plan.nodes[value], from),
                Task::Fields(fields, from, comma) => {
                    if let Some(((offset, field), rest)) = fields.split_first() {
                        if comma { self.constant(b',')?; }
                        tasks.push(Task::Fields(rest, from, true));
                        tasks.push(Task::Value(*field, self.address(from, self.word(*offset))?));
                    } else {
                        self.constant(b']')?;
                    }
                    continue;
                }
                Task::ArrayEnd(step, index, check, end) => {
                    let next = ir!(b.build_int_add(index, self.word(1), "wire.array.next"));
                    let body = b.get_insert_block().ok_or("missing wire loop body")?;
                    ir!(b.build_unconditional_branch(check));
                    step.add_incoming(&[(&next, body)]);
                    b.position_at_end(end);
                    self.constant(b']')?;
                    continue;
                }
            };
            match value {
                Wire::Bool => {
                    let condition = self.load(from, self.context.bool_type())?;
                    let no = ir!(b.build_not(condition, "wire.false"));
                    self.then(condition, || b"true".iter().try_for_each(|&byte| self.constant(byte)))?;
                    self.then(no, || b"false".iter().try_for_each(|&byte| self.constant(byte)))?;
                }
                Wire::Int { signed } => {
                    let mut value = self.load(from, self.context.i32_type())?;
                    if *signed {
                        let negative = ir!(b.build_int_compare(IntPredicate::SLT, value, self.word(0), "wire.negative"));
                        self.then(negative, || self.constant(b'-'))?;
                        // Unsigned magnitude, including i32::MIN; no nsw flag.
                        let negated = ir!(b.build_int_sub(self.word(0), value, "wire.magnitude"));
                        value = ir!(b.build_select(negative, negated, value, "wire.absolute")).into_int_value();
                    }
                    ir!(b.build_call(self.decimal, &[value.into(), self.buffer.into(), self.cursor.into()], ""));
                }
                Wire::Utf8 { data, length } => {
                    let data = self.load(self.address(from, self.word(*data))?, self.context.i32_type())?;
                    let length = self.load(self.address(from, self.word(*length))?, self.context.i64_type())?;
                    let data = ir!(b.build_int_to_ptr(data, self.context.ptr_type(inkwell::AddressSpace::default()), "utf8.data"));
                    // Every admitted descriptor refers to an authored literal
                    // bounded by the selected closed source, far below u32::MAX.
                    let length = ir!(b.build_int_truncate(length, self.context.i32_type(), "utf8.length"));
                    let quote = self.quote.ok_or("missing WASI comptime text encoder")?;
                    ir!(b.build_call(quote, &[data.into(), length.into(), self.buffer.into(), self.cursor.into()], ""));
                }
                Wire::Struct(fields) => {
                    self.constant(b'[')?;
                    tasks.push(Task::Fields(fields, from, false));
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
                    tasks.push(Task::ArrayEnd(step, index, check, end));
                    tasks.push(Task::Value(*element, self.address(from, offset)?));
                }
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

#[cfg(test)]
mod tests {
    use inkwell::types::BasicType;
    use super::*;
    use crate::scalar::{derive_scalar_declarations_from_cst, prepare_scalar_project, ScalarPreparationInputs};
    use std::fmt::Write;
    use wosy_syntax::ByteSpan;

    #[test]
    fn deep_nominal_wasip1_wire_lowers_using_checked_offsets_and_stride() {
        let mut authored = "%%start\nstruct copy Level0 { u32 value; }\n".to_owned();
        for index in 1..72 {
            writeln!(authored, "struct copy Level{index} {{ Level{} inner; }}", index - 1).unwrap();
        }
        authored.push_str("struct copy Top { Level71[2] values; }\n%%end\n");
        let source = SourceIdentity::new("wire".into(), "app".into(), "main.w".into(), "fixture".into());
        let fresh = wosy_syntax::parse_with_snapshot(source.clone(), authored, &[]);
        assert!(fresh.result().is_valid());
        let layout = ScalarTargetLayout::WASM32;
        let prepared = prepare_scalar_project(
            vec![(derive_scalar_declarations_from_cst(fresh.result().canonical_cst()), Vec::new())],
            vec![source.clone()], ScalarPreparationInputs { target_layout: layout },
        ).unwrap_or_else(|error| panic!("{:?}", error.into_ordered_diagnostics()));
        let project = prepared.project();
        let layouts = ScalarLayoutIndex::project(&project.modules);
        let structs: Vec<_> = project.modules.iter().flat_map(|module| &module.structs).collect();
        let top = structs.iter().find(|structure| structure.name == "Top").unwrap();
        let ty = ScalarType::Struct(top.id.clone());
        let site = SourceSpan::new(source, ByteSpan { start: 0, end: 0 });
        let output = Wasip1ComptimeOutput { site: &site, ty: &ty, max_stdout: 314,
            text_fields: &HashMap::new(), max_text_bytes: 0 };
        let plan = wire(&ty, &layouts, &structs, output).unwrap();
        assert_eq!((plan.nodes.len(), plan.bound), (75, 313));
        let Wire::Struct(fields) = &plan.nodes[plan.root] else { panic!("nominal outer type"); };
        assert_eq!(fields[0].0, 0);
        assert!(matches!(plan.nodes[fields[0].1], Wire::Fixed { count: 2, stride: 4, .. }));

        let context = Context::create();
        let module = context.create_module("wire-depth-fixture");
        let main = module.add_function("main", context.i32_type().fn_type(&[], false), None);
        let storage = memory::storage_type(&context, &ty, &layouts, layout).unwrap();
        let evaluate = module.add_function("fixture_value", storage.fn_type(&[], false), None);
        emit(&context, &module, &layouts, &structs, layout, main, evaluate, output).unwrap();
        assert!(module.verify().is_ok());
        assert!(module.print_to_string().to_string().contains("wire.array.offset = mul i32 %wire.array.index, 4"));
    }
}
