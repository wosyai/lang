target triple = "wasm32-wasi"

@__wosy_core_heap_cursor = internal global i64 65536
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
