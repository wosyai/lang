target triple = "wasm32-wasi"

@addresses = global [10 x ptr] zeroinitializer
@freed = global [10 x i1] zeroinitializer
@allocation_count = global i32 0
@free_count = global i32 0
@backing_count = global i32 0
@environment_count = global i32 0
@cell_count = global i32 0
@expected_sizes = constant [10 x i64] [i64 52, i64 64, i64 52, i64 64, i64 20, i64 64, i64 72, i64 20, i64 64, i64 72]
@expected_alignments = constant [10 x i64] [i64 8, i64 8, i64 8, i64 8, i64 4, i64 8, i64 8, i64 4, i64 8, i64 8]
@expected_kinds = constant [10 x i32] [i32 1, i32 2, i32 1, i32 2, i32 0, i32 1, i32 2, i32 0, i32 1, i32 2]

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
  %within = icmp ult i32 %count, 10
  br i1 %within, label %classify, label %failure
classify:
  %size_slot = getelementptr [10 x i64], ptr @expected_sizes, i32 0, i32 %count
  %expected_size = load i64, ptr %size_slot
  %size_ok = icmp eq i64 %size, %expected_size
  %align_slot = getelementptr [10 x i64], ptr @expected_alignments, i32 0, i32 %count
  %expected_alignment = load i64, ptr %align_slot
  %align_ok = icmp eq i64 %alignment, %expected_alignment
  %known = and i1 %size_ok, %align_ok
  br i1 %known, label %allocate, label %failure
allocate:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %present = icmp ne ptr %pointer, null
  br i1 %present, label %record, label %failure
record:
  %slot = getelementptr [10 x ptr], ptr @addresses, i32 0, i32 %count
  store ptr %pointer, ptr %slot
  %next = add i32 %count, 1
  store i32 %next, ptr @allocation_count
  %kind_slot = getelementptr [10 x i32], ptr @expected_kinds, i32 0, i32 %count
  %kind = load i32, ptr %kind_slot
  %is_backing = icmp eq i32 %kind, 0
  %is_environment = icmp eq i32 %kind, 1
  %is_cell = icmp eq i32 %kind, 2
  %old_backing = load i32, ptr @backing_count
  %more_backing = zext i1 %is_backing to i32
  %new_backing = add i32 %old_backing, %more_backing
  store i32 %new_backing, ptr @backing_count
  %old_env = load i32, ptr @environment_count
  %more_env = zext i1 %is_environment to i32
  %new_env = add i32 %old_env, %more_env
  store i32 %new_env, ptr @environment_count
  %old_cell = load i32, ptr @cell_count
  %more_cell = zext i1 %is_cell to i32
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
  %within = icmp ult i32 %index, 10
  br i1 %within, label %compare, label %failure
compare:
  %slot = getelementptr [10 x ptr], ptr @addresses, i32 0, i32 %index
  %expected = load ptr, ptr %slot
  %matched = icmp eq ptr %expected, %pointer
  br i1 %matched, label %checkfresh, label %advance
advance:
  %next = add i32 %index, 1
  br label %test
checkfresh:
  %present = icmp ne ptr %expected, null
  br i1 %present, label %checkdup, label %advance
checkdup:
  %flag = getelementptr [10 x i1], ptr @freed, i32 0, i32 %index
  %was_freed = load i1, ptr %flag
  br i1 %was_freed, label %advance, label %release
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
  %backings = load i32, ptr @backing_count
  %environments = load i32, ptr @environment_count
  %cells = load i32, ptr @cell_count
  %exit_ok = icmp eq i32 %result, 0
  %alloc_ok = icmp eq i32 %allocations, 10
  %free_ok = icmp eq i32 %frees, 10
  %backing_ok = icmp eq i32 %backings, 2
  %env_ok = icmp eq i32 %environments, 4
  %cell_ok = icmp eq i32 %cells, 4
  %a = and i1 %exit_ok, %alloc_ok
  %b = and i1 %free_ok, %backing_ok
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
