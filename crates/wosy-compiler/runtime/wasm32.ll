target triple = "wasm32-wasi"

; Heap starts above the default 64 KiB stack-last region so the bump region never grows through live frames.
@__wosy_core_heap_cursor = internal global i64 131072
@__wosy_core_free_head = internal global i32 0

declare i32 @llvm.wasm.memory.size.i32(i32)
declare i32 @llvm.wasm.memory.grow.i32(i32, i32)
declare void @proc_exit(i32) #0

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define ptr @__wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %size_fits = icmp ule i64 %size, 4294967295
  %alignment_fits = icmp ule i64 %alignment, 2147483648
  %alignment_is_zero = icmp eq i64 %alignment, 0
  %alignment_minus_one = sub i64 %alignment, 1
  %alignment_power_mask = and i64 %alignment, %alignment_minus_one
  %alignment_is_power_of_two = icmp eq i64 %alignment_power_mask, 0
  %alignment_is_not_power_of_two = xor i1 %alignment_is_power_of_two, true
  %alignment_is_invalid = or i1 %alignment_is_zero, %alignment_is_not_power_of_two
  %fits = and i1 %size_fits, %alignment_fits
  %is_valid = xor i1 %alignment_is_invalid, true
  %can_allocate = and i1 %fits, %is_valid
  br i1 %can_allocate, label %reuse_check, label %failure

reuse_check:
  %free_head = load i32, ptr @__wosy_core_free_head
  %has_free_block = icmp ne i32 %free_head, 0
  br i1 %has_free_block, label %reuse_search, label %extend_header

reuse_search:
  %current_address = phi i32 [ %free_head, %reuse_check ], [ %next_free, %reuse_continue ]
  %previous_address = phi i32 [ 0, %reuse_check ], [ %current_address, %reuse_continue ]
  %free_header = inttoptr i32 %current_address to ptr
  %free_size_address = getelementptr i8, ptr %free_header, i32 4
  %free_size = load i32, ptr %free_size_address, align 4
  %free_size64 = zext i32 %free_size to i64
  %free_alignment_address = getelementptr i8, ptr %free_header, i32 8
  %free_alignment = load i32, ptr %free_alignment_address, align 4
  %free_alignment64 = zext i32 %free_alignment to i64
  %size_reusable = icmp uge i64 %free_size64, %size
  %alignment_reusable = icmp uge i64 %free_alignment64, %alignment
  %reusable = and i1 %size_reusable, %alignment_reusable
  br i1 %reusable, label %reuse, label %reuse_continue

reuse_continue:
  %next_free = load i32, ptr %free_header, align 4
  %has_next_free = icmp ne i32 %next_free, 0
  br i1 %has_next_free, label %reuse_search, label %extend_header

reuse:
  %free_alignment_fits = icmp ule i64 %free_alignment64, 2147483648
  %free_alignment_is_zero = icmp eq i64 %free_alignment64, 0
  %free_alignment_minus_one = sub i64 %free_alignment64, 1
  %free_alignment_power_mask = and i64 %free_alignment64, %free_alignment_minus_one
  %free_alignment_is_power_of_two = icmp eq i64 %free_alignment_power_mask, 0
  %free_alignment_is_not_power_of_two = xor i1 %free_alignment_is_power_of_two, true
  %free_alignment_is_invalid = or i1 %free_alignment_is_zero, %free_alignment_is_not_power_of_two
  %free_alignment_is_valid = xor i1 %free_alignment_is_invalid, true
  %free_alignment_can_derive = and i1 %free_alignment_fits, %free_alignment_is_valid
  br i1 %free_alignment_can_derive, label %reuse_header_end, label %reuse_continue

reuse_header_end:
  %free_header64 = zext i32 %current_address to i64
  %free_header_end = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %free_header64, i64 16)
  %free_header_end_value = extractvalue { i64, i1 } %free_header_end, 0
  %free_header_end_overflow = extractvalue { i64, i1 } %free_header_end, 1
  br i1 %free_header_end_overflow, label %reuse_continue, label %reuse_alignment

reuse_alignment:
  %free_aligned_sum = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %free_header_end_value, i64 %free_alignment_minus_one)
  %free_aligned_sum_value = extractvalue { i64, i1 } %free_aligned_sum, 0
  %free_aligned_sum_overflow = extractvalue { i64, i1 } %free_aligned_sum, 1
  br i1 %free_aligned_sum_overflow, label %reuse_continue, label %reuse_size

reuse_size:
  %free_alignment_inverse = xor i64 %free_alignment_minus_one, -1
  %reused_pointer_address = and i64 %free_aligned_sum_value, %free_alignment_inverse
  %reused_allocation_end = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %reused_pointer_address, i64 %free_size64)
  %reused_allocation_end_value = extractvalue { i64, i1 } %reused_allocation_end, 0
  %reused_allocation_end_overflow = extractvalue { i64, i1 } %reused_allocation_end, 1
  %reused_address_space_end = icmp ule i64 %reused_allocation_end_value, 4294967296
  %reused_exceeds_address_space = xor i1 %reused_address_space_end, true
  %reused_extends_address_space = or i1 %reused_allocation_end_overflow, %reused_exceeds_address_space
  br i1 %reused_extends_address_space, label %reuse_continue, label %reuse_unlink

reuse_unlink:
  %selected_next_free = load i32, ptr %free_header, align 4
  %selects_head = icmp eq i32 %previous_address, 0
  br i1 %selects_head, label %reuse_head, label %reuse_after

reuse_head:
  store i32 %selected_next_free, ptr @__wosy_core_free_head
  br label %reuse_return

reuse_after:
  %previous_header = inttoptr i32 %previous_address to ptr
  store i32 %selected_next_free, ptr %previous_header, align 4
  br label %reuse_return

reuse_return:
  %reused_payload_address = trunc i64 %reused_pointer_address to i32
  %reused_pointer = inttoptr i32 %reused_payload_address to ptr
  ret ptr %reused_pointer

extend_header:
  %cursor = load i64, ptr @__wosy_core_heap_cursor
  %header_end = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %cursor, i64 16)
  %header_end_value = extractvalue { i64, i1 } %header_end, 0
  %header_end_overflow = extractvalue { i64, i1 } %header_end, 1
  br i1 %header_end_overflow, label %failure, label %extend_alignment

extend_alignment:
  %aligned_sum = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %header_end_value, i64 %alignment_minus_one)
  %aligned_sum_value = extractvalue { i64, i1 } %aligned_sum, 0
  %aligned_sum_overflow = extractvalue { i64, i1 } %aligned_sum, 1
  br i1 %aligned_sum_overflow, label %failure, label %extend_size

extend_size:
  %alignment_inverse = xor i64 %alignment_minus_one, -1
  %pointer_address = and i64 %aligned_sum_value, %alignment_inverse
  %allocation_end = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %pointer_address, i64 %size)
  %allocation_end_value = extractvalue { i64, i1 } %allocation_end, 0
  %allocation_end_overflow = extractvalue { i64, i1 } %allocation_end, 1
  %address_space_end = icmp ule i64 %allocation_end_value, 4294967296
  %exceeds_address_space = xor i1 %address_space_end, true
  %extends_address_space = or i1 %allocation_end_overflow, %exceeds_address_space
  br i1 %extends_address_space, label %failure, label %capacity_check

capacity_check:
  %pages = call i32 @llvm.wasm.memory.size.i32(i32 0)
  %pages64 = zext i32 %pages to i64
  %current_bytes = shl i64 %pages64, 16
  %has_capacity = icmp ule i64 %allocation_end_value, %current_bytes
  br i1 %has_capacity, label %commit, label %grow_difference

grow_difference:
  %required_bytes = sub i64 %allocation_end_value, %current_bytes
  %pages_before_rounding = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %required_bytes, i64 65535)
  %pages_before_rounding_value = extractvalue { i64, i1 } %pages_before_rounding, 0
  %pages_before_rounding_overflow = extractvalue { i64, i1 } %pages_before_rounding, 1
  br i1 %pages_before_rounding_overflow, label %failure, label %grow_limit

grow_limit:
  %pages_to_grow64 = lshr i64 %pages_before_rounding_value, 16
  %growth_fits = icmp ule i64 %pages_to_grow64, 65536
  br i1 %growth_fits, label %grow, label %failure

grow:
  %pages_to_grow = trunc i64 %pages_to_grow64 to i32
  %grown_pages = call i32 @llvm.wasm.memory.grow.i32(i32 0, i32 %pages_to_grow)
  %growth_failed = icmp eq i32 %grown_pages, -1
  br i1 %growth_failed, label %failure, label %commit

commit:
  %header_address = trunc i64 %cursor to i32
  %payload_address = trunc i64 %pointer_address to i32
  %header = inttoptr i32 %header_address to ptr
  %payload = inttoptr i32 %payload_address to ptr
  %back_pointer = getelementptr i8, ptr %payload, i32 -4
  store i32 %header_address, ptr %back_pointer, align 4
  store i32 0, ptr %header, align 4
  %size_address = getelementptr i8, ptr %header, i32 4
  %size32 = trunc i64 %size to i32
  store i32 %size32, ptr %size_address, align 4
  %alignment_address = getelementptr i8, ptr %header, i32 8
  %alignment32 = trunc i64 %alignment to i32
  store i32 %alignment32, ptr %alignment_address, align 4
  store i64 %allocation_end_value, ptr @__wosy_core_heap_cursor
  ret ptr %payload

failure:
  ret ptr null
}

define void @__wosy_core_free(ptr %pointer) {
entry:
  %is_null = icmp eq ptr %pointer, null
  br i1 %is_null, label %complete, label %release

release:
  %back_pointer = getelementptr i8, ptr %pointer, i32 -4
  %header_address = load i32, ptr %back_pointer, align 4
  %header = inttoptr i32 %header_address to ptr
  %free_head = load i32, ptr @__wosy_core_free_head
  store i32 %free_head, ptr %header, align 4
  store i32 %header_address, ptr @__wosy_core_free_head
  br label %complete

complete:
  ret void
}

define void @__wosy_core_system_panic() {
entry:
  call void @proc_exit(i32 1)
  unreachable
}

define i128 @__multi3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %a0.lo = and i64 %a0, 4294967295
  %a0.hi = lshr i64 %a0, 32
  %b0.lo = and i64 %b0, 4294967295
  %b0.hi = lshr i64 %b0, 32
  %p.ll = mul i64 %a0.lo, %b0.lo
  %p.lh = mul i64 %a0.lo, %b0.hi
  %p.hl = mul i64 %a0.hi, %b0.lo
  %p.hh = mul i64 %a0.hi, %b0.hi
  %p.lh.lo = and i64 %p.lh, 4294967295
  %p.hl.lo = and i64 %p.hl, 4294967295
  %p.ll.hi = lshr i64 %p.ll, 32
  %t0 = add i64 %p.ll.hi, %p.lh.lo
  %c0 = icmp ult i64 %t0, %p.ll.hi
  %c0.ext = zext i1 %c0 to i64
  %t1 = add i64 %t0, %p.hl.lo
  %c1 = icmp ult i64 %t1, %t0
  %c1.ext = zext i1 %c1 to i64
  %p.lh.hi = lshr i64 %p.lh, 32
  %p.hl.hi = lshr i64 %p.hl, 32
  %t1.hi = lshr i64 %t1, 32
  %hi0.a = add i64 %p.hh, %p.lh.hi
  %hi0.b = add i64 %hi0.a, %p.hl.hi
  %hi0.c = add i64 %hi0.b, %t1.hi
  %hi0.d = add i64 %hi0.c, %c0.ext
  %hi0 = add i64 %hi0.d, %c1.ext
  %lo0 = mul i64 %a0, %b0
  %t.a0b1 = mul i64 %a0, %b1
  %t.a1b0 = mul i64 %a1, %b0
  %hi.a = add i64 %hi0, %t.a0b1
  %hi = add i64 %hi.a, %t.a1b0
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %lo0, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__udivti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %b.lo.zero = icmp eq i64 %b0, 0
  %b.hi.zero = icmp eq i64 %b1, 0
  %b.is.zero = and i1 %b.lo.zero, %b.hi.zero
  br i1 %b.is.zero, label %div.zero, label %loop

div.zero:
  ret i128 -1

loop:
  %i = phi i64 [ 127, %entry ], [ %i.next, %loop ]
  %q1 = phi i64 [ 0, %entry ], [ %q1.next, %loop ]
  %q0 = phi i64 [ 0, %entry ], [ %q0.next, %loop ]
  %r1 = phi i64 [ 0, %entry ], [ %r1.next, %loop ]
  %r0 = phi i64 [ 0, %entry ], [ %r0.next, %loop ]
  %i.ge.64 = icmp uge i64 %i, 64
  %i.sub.64 = sub i64 %i, 64
  %a1.bit.raw = lshr i64 %a1, %i.sub.64
  %a0.bit.raw = lshr i64 %a0, %i
  %bit.raw = select i1 %i.ge.64, i64 %a1.bit.raw, i64 %a0.bit.raw
  %bit = and i64 %bit.raw, 1
  %r0.carry = lshr i64 %r0, 63
  %r0.sh = shl i64 %r0, 1
  %r0.acc = or i64 %r0.sh, %bit
  %r1.sh = shl i64 %r1, 1
  %r1.acc = or i64 %r1.sh, %r0.carry
  %r.hi.gt = icmp ugt i64 %r1.acc, %b1
  %r.hi.eq = icmp eq i64 %r1.acc, %b1
  %r.lo.ge = icmp uge i64 %r0.acc, %b0
  %r.lo.take = and i1 %r.hi.eq, %r.lo.ge
  %r.ge = or i1 %r.hi.gt, %r.lo.take
  %sub0 = sub i64 %r0.acc, %b0
  %borrow = icmp ult i64 %r0.acc, %b0
  %borrow.ext = zext i1 %borrow to i64
  %sub1.tmp = sub i64 %r1.acc, %b1
  %sub1 = sub i64 %sub1.tmp, %borrow.ext
  %r0.next = select i1 %r.ge, i64 %sub0, i64 %r0.acc
  %r1.next = select i1 %r.ge, i64 %sub1, i64 %r1.acc
  %qbit = select i1 %r.ge, i64 1, i64 0
  %q.sh.hi = shl i64 %qbit, %i.sub.64
  %q.sh.lo = shl i64 %qbit, %i
  %q1.or = or i64 %q1, %q.sh.hi
  %q0.or = or i64 %q0, %q.sh.lo
  %q1.next = select i1 %i.ge.64, i64 %q1.or, i64 %q1
  %q0.next = select i1 %i.ge.64, i64 %q0, i64 %q0.or
  %last = icmp eq i64 %i, 0
  %i.next = sub i64 %i, 1
  br i1 %last, label %exit, label %loop

exit:
  %q1.fin = phi i64 [ %q1.next, %loop ]
  %q0.fin = phi i64 [ %q0.next, %loop ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %q0.fin, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %q1.fin, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__divti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %a.neg = icmp slt i64 %a1, 0
  %b.neg = icmp slt i64 %b1, 0
  %a0.nz = icmp ne i64 %a0, 0
  %a0.nz.ext = zext i1 %a0.nz to i64
  %na0 = sub i64 0, %a0
  %na1.tmp = sub i64 0, %a1
  %na1 = sub i64 %na1.tmp, %a0.nz.ext
  %ma0 = select i1 %a.neg, i64 %na0, i64 %a0
  %ma1 = select i1 %a.neg, i64 %na1, i64 %a1
  %b0.nz = icmp ne i64 %b0, 0
  %b0.nz.ext = zext i1 %b0.nz to i64
  %nb0 = sub i64 0, %b0
  %nb1.tmp = sub i64 0, %b1
  %nb1 = sub i64 %nb1.tmp, %b0.nz.ext
  %mb0 = select i1 %b.neg, i64 %nb0, i64 %b0
  %mb1 = select i1 %b.neg, i64 %nb1, i64 %b1
  %ma.slot = alloca i128, align 16
  %ma.lo.ptr = getelementptr i64, ptr %ma.slot, i32 0
  store i64 %ma0, ptr %ma.lo.ptr, align 8
  %ma.hi.ptr = getelementptr i64, ptr %ma.slot, i32 1
  store i64 %ma1, ptr %ma.hi.ptr, align 8
  %ma = load i128, ptr %ma.slot, align 16
  %mb.slot = alloca i128, align 16
  %mb.lo.ptr = getelementptr i64, ptr %mb.slot, i32 0
  store i64 %mb0, ptr %mb.lo.ptr, align 8
  %mb.hi.ptr = getelementptr i64, ptr %mb.slot, i32 1
  store i64 %mb1, ptr %mb.hi.ptr, align 8
  %mb = load i128, ptr %mb.slot, align 16
  %q.mag = call i128 @__udivti3(i128 %ma, i128 %mb)
  %q.slot = alloca i128, align 16
  store i128 %q.mag, ptr %q.slot, align 16
  %q.lo.ptr = getelementptr i64, ptr %q.slot, i32 0
  %q0 = load i64, ptr %q.lo.ptr, align 8
  %q.hi.ptr = getelementptr i64, ptr %q.slot, i32 1
  %q1 = load i64, ptr %q.hi.ptr, align 8
  %q.neg = xor i1 %a.neg, %b.neg
  %q0.nz = icmp ne i64 %q0, 0
  %q0.nz.ext = zext i1 %q0.nz to i64
  %nq0 = sub i64 0, %q0
  %nq1.tmp = sub i64 0, %q1
  %nq1 = sub i64 %nq1.tmp, %q0.nz.ext
  %r0 = select i1 %q.neg, i64 %nq0, i64 %q0
  %r1 = select i1 %q.neg, i64 %nq1, i64 %q1
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r0, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r1, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__umodti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %b.lo.zero = icmp eq i64 %b0, 0
  %b.hi.zero = icmp eq i64 %b1, 0
  %b.is.zero = and i1 %b.lo.zero, %b.hi.zero
  br i1 %b.is.zero, label %mod.zero, label %loop

mod.zero:
  ret i128 %a

loop:
  %i = phi i64 [ 127, %entry ], [ %i.next, %loop ]
  %q1 = phi i64 [ 0, %entry ], [ %q1.next, %loop ]
  %q0 = phi i64 [ 0, %entry ], [ %q0.next, %loop ]
  %r1 = phi i64 [ 0, %entry ], [ %r1.next, %loop ]
  %r0 = phi i64 [ 0, %entry ], [ %r0.next, %loop ]
  %i.ge.64 = icmp uge i64 %i, 64
  %i.sub.64 = sub i64 %i, 64
  %a1.bit.raw = lshr i64 %a1, %i.sub.64
  %a0.bit.raw = lshr i64 %a0, %i
  %bit.raw = select i1 %i.ge.64, i64 %a1.bit.raw, i64 %a0.bit.raw
  %bit = and i64 %bit.raw, 1
  %r0.carry = lshr i64 %r0, 63
  %r0.sh = shl i64 %r0, 1
  %r0.acc = or i64 %r0.sh, %bit
  %r1.sh = shl i64 %r1, 1
  %r1.acc = or i64 %r1.sh, %r0.carry
  %r.hi.gt = icmp ugt i64 %r1.acc, %b1
  %r.hi.eq = icmp eq i64 %r1.acc, %b1
  %r.lo.ge = icmp uge i64 %r0.acc, %b0
  %r.lo.take = and i1 %r.hi.eq, %r.lo.ge
  %r.ge = or i1 %r.hi.gt, %r.lo.take
  %sub0 = sub i64 %r0.acc, %b0
  %borrow = icmp ult i64 %r0.acc, %b0
  %borrow.ext = zext i1 %borrow to i64
  %sub1.tmp = sub i64 %r1.acc, %b1
  %sub1 = sub i64 %sub1.tmp, %borrow.ext
  %r0.next = select i1 %r.ge, i64 %sub0, i64 %r0.acc
  %r1.next = select i1 %r.ge, i64 %sub1, i64 %r1.acc
  %qbit = select i1 %r.ge, i64 1, i64 0
  %q.sh.hi = shl i64 %qbit, %i.sub.64
  %q.sh.lo = shl i64 %qbit, %i
  %q1.or = or i64 %q1, %q.sh.hi
  %q0.or = or i64 %q0, %q.sh.lo
  %q1.next = select i1 %i.ge.64, i64 %q1.or, i64 %q1
  %q0.next = select i1 %i.ge.64, i64 %q0, i64 %q0.or
  %last = icmp eq i64 %i, 0
  %i.next = sub i64 %i, 1
  br i1 %last, label %exit, label %loop

exit:
  %r0.fin = phi i64 [ %r0.next, %loop ]
  %r1.fin = phi i64 [ %r1.next, %loop ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r0.fin, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r1.fin, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__modti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %b.hi.ptr = getelementptr i64, ptr %b.slot, i32 1
  %b1 = load i64, ptr %b.hi.ptr, align 8
  %a.neg = icmp slt i64 %a1, 0
  %b.neg = icmp slt i64 %b1, 0
  %a0.nz = icmp ne i64 %a0, 0
  %a0.nz.ext = zext i1 %a0.nz to i64
  %na0 = sub i64 0, %a0
  %na1.tmp = sub i64 0, %a1
  %na1 = sub i64 %na1.tmp, %a0.nz.ext
  %ma0 = select i1 %a.neg, i64 %na0, i64 %a0
  %ma1 = select i1 %a.neg, i64 %na1, i64 %a1
  %b0.nz = icmp ne i64 %b0, 0
  %b0.nz.ext = zext i1 %b0.nz to i64
  %nb0 = sub i64 0, %b0
  %nb1.tmp = sub i64 0, %b1
  %nb1 = sub i64 %nb1.tmp, %b0.nz.ext
  %mb0 = select i1 %b.neg, i64 %nb0, i64 %b0
  %mb1 = select i1 %b.neg, i64 %nb1, i64 %b1
  %ma.slot = alloca i128, align 16
  %ma.lo.ptr = getelementptr i64, ptr %ma.slot, i32 0
  store i64 %ma0, ptr %ma.lo.ptr, align 8
  %ma.hi.ptr = getelementptr i64, ptr %ma.slot, i32 1
  store i64 %ma1, ptr %ma.hi.ptr, align 8
  %ma = load i128, ptr %ma.slot, align 16
  %mb.slot = alloca i128, align 16
  %mb.lo.ptr = getelementptr i64, ptr %mb.slot, i32 0
  store i64 %mb0, ptr %mb.lo.ptr, align 8
  %mb.hi.ptr = getelementptr i64, ptr %mb.slot, i32 1
  store i64 %mb1, ptr %mb.hi.ptr, align 8
  %mb = load i128, ptr %mb.slot, align 16
  %r.mag = call i128 @__umodti3(i128 %ma, i128 %mb)
  %m.slot = alloca i128, align 16
  store i128 %r.mag, ptr %m.slot, align 16
  %m.lo.ptr = getelementptr i64, ptr %m.slot, i32 0
  %m0 = load i64, ptr %m.lo.ptr, align 8
  %m.hi.ptr = getelementptr i64, ptr %m.slot, i32 1
  %m1 = load i64, ptr %m.hi.ptr, align 8
  %m0.nz = icmp ne i64 %m0, 0
  %m0.nz.ext = zext i1 %m0.nz to i64
  %nm0 = sub i64 0, %m0
  %nm1.tmp = sub i64 0, %m1
  %nm1 = sub i64 %nm1.tmp, %m0.nz.ext
  %r0 = select i1 %a.neg, i64 %nm0, i64 %m0
  %r1 = select i1 %a.neg, i64 %nm1, i64 %m1
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r0, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r1, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__ashlti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %n = and i64 %b0, 127
  %n.is.zero = icmp eq i64 %n, 0
  br i1 %n.is.zero, label %sh.identity, label %sh.split

sh.identity:
  ret i128 %a

sh.split:
  %n.ge.64 = icmp uge i64 %n, 64
  br i1 %n.ge.64, label %sh.big, label %sh.small

sh.small:
  %s.lo = shl i64 %a0, %n
  %s.comp = sub i64 64, %n
  %s.carry = lshr i64 %a0, %s.comp
  %s.hi.sh = shl i64 %a1, %n
  %s.hi = or i64 %s.hi.sh, %s.carry
  br label %sh.exit

sh.big:
  %b.sh = sub i64 %n, 64
  %b.hi = shl i64 %a0, %b.sh
  br label %sh.exit

sh.exit:
  %r.lo = phi i64 [ %s.lo, %sh.small ], [ 0, %sh.big ]
  %r.hi = phi i64 [ %s.hi, %sh.small ], [ %b.hi, %sh.big ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r.lo, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r.hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__lshrti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %n = and i64 %b0, 127
  %n.is.zero = icmp eq i64 %n, 0
  br i1 %n.is.zero, label %sh.identity, label %sh.split

sh.identity:
  ret i128 %a

sh.split:
  %n.ge.64 = icmp uge i64 %n, 64
  br i1 %n.ge.64, label %sh.big, label %sh.small

sh.small:
  %s.hi = lshr i64 %a1, %n
  %s.comp = sub i64 64, %n
  %s.carry = shl i64 %a1, %s.comp
  %s.lo.sh = lshr i64 %a0, %n
  %s.lo = or i64 %s.lo.sh, %s.carry
  br label %sh.exit

sh.big:
  %b.sh = sub i64 %n, 64
  %b.lo = lshr i64 %a1, %b.sh
  br label %sh.exit

sh.exit:
  %r.lo = phi i64 [ %s.lo, %sh.small ], [ %b.lo, %sh.big ]
  %r.hi = phi i64 [ %s.hi, %sh.small ], [ 0, %sh.big ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r.lo, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r.hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}

define i128 @__ashrti3(i128 %a, i128 %b) {
entry:
  %a.slot = alloca i128, align 16
  store i128 %a, ptr %a.slot, align 16
  %a.lo.ptr = getelementptr i64, ptr %a.slot, i32 0
  %a0 = load i64, ptr %a.lo.ptr, align 8
  %a.hi.ptr = getelementptr i64, ptr %a.slot, i32 1
  %a1 = load i64, ptr %a.hi.ptr, align 8
  %b.slot = alloca i128, align 16
  store i128 %b, ptr %b.slot, align 16
  %b.lo.ptr = getelementptr i64, ptr %b.slot, i32 0
  %b0 = load i64, ptr %b.lo.ptr, align 8
  %n = and i64 %b0, 127
  %n.is.zero = icmp eq i64 %n, 0
  br i1 %n.is.zero, label %sh.identity, label %sh.split

sh.identity:
  ret i128 %a

sh.split:
  %n.ge.64 = icmp uge i64 %n, 64
  br i1 %n.ge.64, label %sh.big, label %sh.small

sh.small:
  %s.hi = ashr i64 %a1, %n
  %s.comp = sub i64 64, %n
  %s.carry = shl i64 %a1, %s.comp
  %s.lo.sh = lshr i64 %a0, %n
  %s.lo = or i64 %s.lo.sh, %s.carry
  br label %sh.exit

sh.big:
  %b.sh = sub i64 %n, 64
  %b.lo = ashr i64 %a1, %b.sh
  %b.hi = ashr i64 %a1, 63
  br label %sh.exit

sh.exit:
  %r.lo = phi i64 [ %s.lo, %sh.small ], [ %b.lo, %sh.big ]
  %r.hi = phi i64 [ %s.hi, %sh.small ], [ %b.hi, %sh.big ]
  %r.slot = alloca i128, align 16
  %r.lo.ptr = getelementptr i64, ptr %r.slot, i32 0
  store i64 %r.lo, ptr %r.lo.ptr, align 8
  %r.hi.ptr = getelementptr i64, ptr %r.slot, i32 1
  store i64 %r.hi, ptr %r.hi.ptr, align 8
  %r = load i128, ptr %r.slot, align 16
  ret i128 %r
}
