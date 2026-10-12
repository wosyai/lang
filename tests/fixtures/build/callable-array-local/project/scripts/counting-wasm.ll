target triple = "wasm32-wasi"

@addresses = global [7 x ptr] zeroinitializer
@freed = global [7 x i1] zeroinitializer
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
  %within = icmp ult i32 %count, 7
  br i1 %within, label %classify, label %failure
classify:
  %same_size = icmp eq i64 %size, 28
  %first_allocation = icmp eq i32 %count, 0
  %later_allocation = icmp ne i32 %count, 0
  %array_size = and i1 %same_size, %first_allocation
  %env_size = icmp eq i64 %size, 52
  %environment_size = and i1 %env_size, %later_allocation
  %cell_size = icmp eq i64 %size, 57
  %kind = or i1 %array_size, %environment_size
  %known = or i1 %kind, %cell_size
  br i1 %known, label %allocate, label %failure
allocate:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %present = icmp ne ptr %pointer, null
  br i1 %present, label %record, label %failure
record:
  %slot = getelementptr [7 x ptr], ptr @addresses, i32 0, i32 %count
  store ptr %pointer, ptr %slot
  %next = add i32 %count, 1
  store i32 %next, ptr @allocation_count
  %old_array = load i32, ptr @array_count
  %more_array = zext i1 %array_size to i32
  %new_array = add i32 %old_array, %more_array
  store i32 %new_array, ptr @array_count
  %old_env = load i32, ptr @environment_count
  %more_env = zext i1 %environment_size to i32
  %new_env = add i32 %old_env, %more_env
  store i32 %new_env, ptr @environment_count
  %old_cell = load i32, ptr @cell_count
  %more_cell = zext i1 %cell_size to i32
  %new_cell = add i32 %old_cell, %more_cell
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
  %within = icmp ult i32 %index, 7
  br i1 %within, label %compare, label %failure
compare:
  %slot = getelementptr [7 x ptr], ptr @addresses, i32 0, i32 %index
  %expected = load ptr, ptr %slot
  %matched = icmp eq ptr %expected, %pointer
  br i1 %matched, label %found, label %advance
advance:
  %next = add i32 %index, 1
  br label %test
found:
  %flag = getelementptr [7 x i1], ptr @freed, i32 0, i32 %index
  %was_freed = load i1, ptr %flag
  %present = icmp ne ptr %expected, null
  %duplicate = xor i1 %was_freed, true
  %valid = and i1 %present, %duplicate
  br i1 %valid, label %release, label %failure
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
  %alloc_ok = icmp eq i32 %allocations, 7
  %free_ok = icmp eq i32 %frees, 7
  %array_ok = icmp eq i32 %arrays, 1
  %env_ok = icmp eq i32 %environments, 4
  %cell_ok = icmp eq i32 %cells, 2
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
