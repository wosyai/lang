target triple = "wasm32-wasi"

@addresses = global [12 x ptr] zeroinitializer
@freed = global [12 x i1] zeroinitializer
@release_order = private constant [12 x i32] [i32 3, i32 0, i32 5, i32 9, i32 10, i32 4, i32 11, i32 8, i32 6, i32 7, i32 2, i32 1]
@allocation_count = global i32 0
@free_count = global i32 0
@array_count = global i32 0
@environment_count = global i32 0
@cell_count = global i32 0

declare ptr @__real___wosy_core_alloc(i64, i64)
declare void @__real___wosy_core_free(ptr)
declare i32 @main()
declare void @proc_exit(i32) #0

define ptr @memset(ptr %destination, i32 %value, i32 %size) {
entry:
  br label %test
test:
  %index = phi i32 [ 0, %entry ], [ %next, %body ]
  %within = icmp ult i32 %index, %size
  br i1 %within, label %body, label %done
body:
  %address = getelementptr i8, ptr %destination, i32 %index
  %byte = trunc i32 %value to i8
  store i8 %byte, ptr %address
  %next = add i32 %index, 1
  br label %test
done:
  ret ptr %destination
}

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define ptr @__wrap___wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %count = load i32, ptr @allocation_count
  %within = icmp ult i32 %count, 12
  br i1 %within, label %classify, label %failure
classify:
  %first = icmp eq i32 %count, 0
  %old_array_size = icmp eq i64 %size, 28
  %array = and i1 %first, %old_array_size
  %new_array_generation = icmp eq i32 %count, 8
  %new_array_size = icmp eq i64 %size, 36
  %new_array_match = and i1 %new_array_generation, %new_array_size
  %is_array = or i1 %array, %new_array_match
  %env1 = icmp eq i32 %count, 1
  %env3 = icmp eq i32 %count, 3
  %env9 = icmp eq i32 %count, 9
  %env10 = icmp eq i32 %count, 10
  %early_nested = or i1 %env1, %env3
  %later_nested = or i1 %env9, %env10
  %nested_generation = or i1 %early_nested, %later_nested
  %nested_size = icmp eq i64 %size, 52
  %nested_env = and i1 %nested_size, %nested_generation
  %env5 = icmp eq i32 %count, 5
  %env7 = icmp eq i32 %count, 7
  %outer_generation = or i1 %env5, %env7
  %outer_size = icmp eq i64 %size, 64
  %outer_env = and i1 %outer_generation, %outer_size
  %env11 = icmp eq i32 %count, 11
  %last_env_size = icmp eq i64 %size, 56
  %last_env = and i1 %env11, %last_env_size
  %environment_size = or i1 %nested_env, %outer_env
  %environment = or i1 %environment_size, %last_env
  %cell2 = icmp eq i32 %count, 2
  %cell4 = icmp eq i32 %count, 4
  %scalar_cell_generation = or i1 %cell2, %cell4
  %scalar_cell_size = icmp eq i64 %size, 57
  %scalar_cell = and i1 %scalar_cell_generation, %scalar_cell_size
  %cell6 = icmp eq i32 %count, 6
  %array_cell_size = icmp eq i64 %size, 72
  %array_cell = and i1 %cell6, %array_cell_size
  %cell = or i1 %scalar_cell, %array_cell
  %node = or i1 %environment, %cell
  %known = or i1 %is_array, %node
  br i1 %known, label %allocate, label %failure
allocate:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %present = icmp ne ptr %pointer, null
  br i1 %present, label %record, label %failure
record:
  %slot = getelementptr [12 x ptr], ptr @addresses, i32 0, i32 %count
  store ptr %pointer, ptr %slot
  %next = add i32 %count, 1
  store i32 %next, ptr @allocation_count
  %old_array = load i32, ptr @array_count
  %one_array = zext i1 %is_array to i32
  %new_array = add i32 %old_array, %one_array
  store i32 %new_array, ptr @array_count
  %old_env = load i32, ptr @environment_count
  %one_env = zext i1 %environment to i32
  %new_env = add i32 %old_env, %one_env
  store i32 %new_env, ptr @environment_count
  %old_cell = load i32, ptr @cell_count
  %one_cell = zext i1 %cell to i32
  %new_cell = add i32 %old_cell, %one_cell
  store i32 %new_cell, ptr @cell_count
  ret ptr %pointer
failure:
  call void @proc_exit(i32 78)
  unreachable
}

define void @__wrap___wosy_core_free(ptr %pointer) {
entry:
  br label %test
test:
  %index = phi i32 [ 0, %entry ], [ %next, %advance ]
  %within = icmp ult i32 %index, 12
  br i1 %within, label %compare, label %failure
compare:
  %slot = getelementptr [12 x ptr], ptr @addresses, i32 0, i32 %index
  %expected = load ptr, ptr %slot
  %matched = icmp eq ptr %expected, %pointer
  br i1 %matched, label %found, label %advance
advance:
  %next = add i32 %index, 1
  br label %test
found:
  %flag = getelementptr [12 x i1], ptr @freed, i32 0, i32 %index
  %was_freed = load i1, ptr %flag
  %present = icmp ne ptr %expected, null
  %unused = xor i1 %was_freed, true
  %valid = and i1 %present, %unused
  br i1 %valid, label %generation_guard, label %failure
generation_guard:
  %release_count = load i32, ptr @free_count
  %release_slot = getelementptr [12 x i32], ptr @release_order, i32 0, i32 %release_count
  %expected_index = load i32, ptr %release_slot
  %ordered = icmp eq i32 %index, %expected_index
  br i1 %ordered, label %order_verified, label %failure
order_verified:
  %old_environment = icmp eq i32 %index, 1
  br i1 %old_environment, label %old_environment_guard, label %live_generation_guard
live_generation_guard:
  %array_generation = icmp eq i32 %index, 0
  %cell_generation = icmp eq i32 %index, 6
  %new_array_generation_at_free = icmp eq i32 %index, 8
  %outer_generation = icmp eq i32 %index, 5
  %keep_after_array = or i1 %array_generation, %cell_generation
  %keep_after_new_array = or i1 %new_array_generation_at_free, %outer_generation
  %must_keep_copy = or i1 %keep_after_array, %keep_after_new_array
  br i1 %must_keep_copy, label %live_copy, label %release
live_copy:
  %old_flag = getelementptr [12 x i1], ptr @freed, i32 0, i32 1
  %old_released = load i1, ptr %old_flag
  %old_pointer_slot = getelementptr [12 x ptr], ptr @addresses, i32 0, i32 1
  %old_pointer = load ptr, ptr %old_pointer_slot
  %old_count_slot = getelementptr i8, ptr %old_pointer, i32 24
  %live_owners = load i64, ptr %old_count_slot
  %initial_copy_count = icmp eq i64 %live_owners, 2
  %final_copy_count = icmp eq i64 %live_owners, 1
  %live = select i1 %array_generation, i1 %initial_copy_count, i1 %final_copy_count
  %not_released = xor i1 %old_released, true
  %copy_available = and i1 %live, %not_released
  br i1 %copy_available, label %release, label %failure
old_environment_guard:
  %array_flag = getelementptr [12 x i1], ptr @freed, i32 0, i32 0
  %array_released = load i1, ptr %array_flag
  %cell_flag = getelementptr [12 x i1], ptr @freed, i32 0, i32 6
  %cell_released = load i1, ptr %cell_flag
  %new_array_flag = getelementptr [12 x i1], ptr @freed, i32 0, i32 8
  %new_array_released = load i1, ptr %new_array_flag
  %old_owner_released = and i1 %array_released, %cell_released
  %owners_released = and i1 %old_owner_released, %new_array_released
  %owner_count_slot = getelementptr i8, ptr %pointer, i32 24
  %owner_count = load i64, ptr %owner_count_slot
  %last_owner = icmp eq i64 %owner_count, 0
  %ready = and i1 %owners_released, %last_owner
  br i1 %ready, label %release, label %failure
release:
  store i1 true, ptr %flag
  %old_count = load i32, ptr @free_count
  %new_count = add i32 %old_count, 1
  store i32 %new_count, ptr @free_count
  call void @__real___wosy_core_free(ptr %pointer)
  ret void
failure:
  call void @proc_exit(i32 79)
  unreachable
}

define void @_start() {
entry:
  %result = call i32 @main()
  %allocations = load i32, ptr @allocation_count
  %frees = load i32, ptr @free_count
  %arrays = load i32, ptr @array_count
  %environments = load i32, ptr @environment_count
  %cells = load i32, ptr @cell_count
  %exit_ok = icmp eq i32 %result, 0
  %alloc_ok = icmp eq i32 %allocations, 12
  %free_ok = icmp eq i32 %frees, 12
  %array_ok = icmp eq i32 %arrays, 2
  %env_ok = icmp eq i32 %environments, 7
  %cell_ok = icmp eq i32 %cells, 3
  %a = and i1 %exit_ok, %alloc_ok
  %b = and i1 %free_ok, %array_ok
  %c = and i1 %env_ok, %cell_ok
  %d = and i1 %a, %b
  %ok = and i1 %d, %c
  br i1 %ok, label %success, label %failure
success:
  ret void
failure:
  call void @proc_exit(i32 80)
  unreachable
}
