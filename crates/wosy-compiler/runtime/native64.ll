declare ptr @malloc(i64)
declare void @free(ptr)
declare void @exit(i32)
declare { i64, i1 } @llvm.uadd.with.overflow.i64(i64, i64)

define ptr @__wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %alignment_is_zero = icmp eq i64 %alignment, 0
  %alignment_minus_one = sub i64 %alignment, 1
  %alignment_power_mask = and i64 %alignment, %alignment_minus_one
  %alignment_is_power_of_two = icmp eq i64 %alignment_power_mask, 0
  %alignment_is_not_power_of_two = xor i1 %alignment_is_power_of_two, true
  %alignment_is_invalid = or i1 %alignment_is_zero, %alignment_is_not_power_of_two
  br i1 %alignment_is_invalid, label %failure, label %size_padding

size_padding:
  %size_with_alignment = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %size, i64 %alignment_minus_one)
  %size_padding_value = extractvalue { i64, i1 } %size_with_alignment, 0
  %size_padding_overflow = extractvalue { i64, i1 } %size_with_alignment, 1
  br i1 %size_padding_overflow, label %failure, label %header_padding

header_padding:
  %allocation_size = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %size_padding_value, i64 8)
  %allocation_size_value = extractvalue { i64, i1 } %allocation_size, 0
  %allocation_size_overflow = extractvalue { i64, i1 } %allocation_size, 1
  br i1 %allocation_size_overflow, label %failure, label %allocate

allocate:
  %allocation = call ptr @malloc(i64 %allocation_size_value)
  %allocation_failed = icmp eq ptr %allocation, null
  br i1 %allocation_failed, label %failure, label %align

align:
  %allocation_address = ptrtoint ptr %allocation to i64
  %header_address = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %allocation_address, i64 8)
  %header_address_value = extractvalue { i64, i1 } %header_address, 0
  %header_address_overflow = extractvalue { i64, i1 } %header_address, 1
  br i1 %header_address_overflow, label %release_failed_alignment, label %align_sum

align_sum:
  %aligned_sum = call { i64, i1 } @llvm.uadd.with.overflow.i64(i64 %header_address_value, i64 %alignment_minus_one)
  %aligned_sum_value = extractvalue { i64, i1 } %aligned_sum, 0
  %aligned_sum_overflow = extractvalue { i64, i1 } %aligned_sum, 1
  br i1 %aligned_sum_overflow, label %release_failed_alignment, label %commit

release_failed_alignment:
  call void @free(ptr %allocation)
  br label %failure

commit:
  %alignment_inverse = xor i64 %alignment_minus_one, -1
  %aligned_address = and i64 %aligned_sum_value, %alignment_inverse
  %pointer = inttoptr i64 %aligned_address to ptr
  %header = getelementptr i8, ptr %pointer, i64 -8
  store ptr %allocation, ptr %header, align 8
  ret ptr %pointer

failure:
  ret ptr null
}

define void @__wosy_core_free(ptr %pointer) {
entry:
  %is_null = icmp eq ptr %pointer, null
  br i1 %is_null, label %complete, label %release

release:
  %header = getelementptr i8, ptr %pointer, i64 -8
  %allocation = load ptr, ptr %header, align 8
  call void @free(ptr %allocation)
  br label %complete

complete:
  ret void
}

define void @__wosy_core_system_panic() {
entry:
  call void @exit(i32 1)
  unreachable
}
