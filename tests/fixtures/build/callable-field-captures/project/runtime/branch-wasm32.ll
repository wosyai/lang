target triple = "wasm32-wasi"

@__branch_arrays = global [4 x ptr] zeroinitializer
@__branch_owners = global [12 x ptr] zeroinitializer
@__branch_array_frees = global [4 x i32] zeroinitializer
@__branch_owner_frees = global [12 x i32] zeroinitializer
@__branch_array_count = global i32 0
@__branch_owner_count = global i32 0
@__branch_free_count = global i32 0
@__branch_unmatched = global i32 0

declare ptr @__real___wosy_core_alloc(i64, i64)
declare void @__real___wosy_core_free(ptr)

define ptr @__wrap___wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %array_size = icmp eq i64 %size, 2
  %byte_aligned = icmp eq i64 %alignment, 1
  %array = and i1 %array_size, %byte_aligned
  %cell_size = icmp eq i64 %size, 72
  %environment_size = icmp eq i64 %size, 64
  %owner_size = or i1 %cell_size, %environment_size
  %owner_alignment = icmp eq i64 %alignment, 8
  %owner = and i1 %owner_size, %owner_alignment
  %present = icmp ne ptr %pointer, null
  %record_array = and i1 %array, %present
  br i1 %record_array, label %array_record, label %owner_check

array_record:
  %arrays = load i32, ptr @__branch_array_count
  %array_in_bounds = icmp ult i32 %arrays, 4
  br i1 %array_in_bounds, label %array_slot, label %array_increment

array_slot:
  %array_address = getelementptr [4 x ptr], ptr @__branch_arrays, i32 0, i32 %arrays
  store ptr %pointer, ptr %array_address
  br label %array_increment

array_increment:
  %next_array = add i32 %arrays, 1
  store i32 %next_array, ptr @__branch_array_count
  br label %done

owner_check:
  %record_owner = and i1 %owner, %present
  br i1 %record_owner, label %owner_record, label %done

owner_record:
  %owners = load i32, ptr @__branch_owner_count
  %owner_in_bounds = icmp ult i32 %owners, 12
  br i1 %owner_in_bounds, label %owner_slot, label %owner_increment

owner_slot:
  %owner_address = getelementptr [12 x ptr], ptr @__branch_owners, i32 0, i32 %owners
  store ptr %pointer, ptr %owner_address
  br label %owner_increment

owner_increment:
  %next_owner = add i32 %owners, 1
  store i32 %next_owner, ptr @__branch_owner_count
  br label %done

done:
  ret ptr %pointer
}

define void @__wrap___wosy_core_free(ptr %pointer) {
entry:
  %count = load i32, ptr @__branch_free_count
  %next = add i32 %count, 1
  store i32 %next, ptr @__branch_free_count
  br label %array_loop

array_loop:
  %array_index = phi i32 [ 0, %entry ], [ %array_next, %array_continue ]
  %array_done = icmp eq i32 %array_index, 4
  br i1 %array_done, label %owner_loop, label %array_check

array_check:
  %array_slot = getelementptr [4 x ptr], ptr @__branch_arrays, i32 0, i32 %array_index
  %array_value = load ptr, ptr %array_slot
  %array_match = icmp eq ptr %array_value, %pointer
  %array_present = icmp ne ptr %array_value, null
  %array_equal = and i1 %array_match, %array_present
  br i1 %array_equal, label %array_found, label %array_continue

array_found:
  %array_counter = getelementptr [4 x i32], ptr @__branch_array_frees, i32 0, i32 %array_index
  %old_array = load i32, ptr %array_counter
  %new_array = add i32 %old_array, 1
  store i32 %new_array, ptr %array_counter
  store ptr null, ptr %array_slot
  br label %done

array_continue:
  %array_next = add i32 %array_index, 1
  br label %array_loop

owner_loop:
  %owner_index = phi i32 [ 0, %array_loop ], [ %owner_next, %owner_continue ]
  %owner_done = icmp eq i32 %owner_index, 12
  br i1 %owner_done, label %unmatched, label %owner_check

owner_check:
  %owner_slot = getelementptr [12 x ptr], ptr @__branch_owners, i32 0, i32 %owner_index
  %owner_value = load ptr, ptr %owner_slot
  %owner_match = icmp eq ptr %owner_value, %pointer
  %owner_present = icmp ne ptr %owner_value, null
  %owner_equal = and i1 %owner_match, %owner_present
  br i1 %owner_equal, label %owner_found, label %owner_continue

owner_found:
  %owner_counter = getelementptr [12 x i32], ptr @__branch_owner_frees, i32 0, i32 %owner_index
  %old_owner = load i32, ptr %owner_counter
  %new_owner = add i32 %old_owner, 1
  store i32 %new_owner, ptr %owner_counter
  store ptr null, ptr %owner_slot
  br label %done

owner_continue:
  %owner_next = add i32 %owner_index, 1
  br label %owner_loop

unmatched:
  %old_unmatched = load i32, ptr @__branch_unmatched
  %new_unmatched = add i32 %old_unmatched, 1
  store i32 %new_unmatched, ptr @__branch_unmatched
  br label %done

done:
  call void @__real___wosy_core_free(ptr %pointer)
  ret void
}
