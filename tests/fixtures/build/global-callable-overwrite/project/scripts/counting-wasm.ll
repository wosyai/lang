target triple = "wasm32-wasi"

@addresses = global [3 x ptr] zeroinitializer
@freed = global [3 x i1] zeroinitializer
@release_order = private constant [3 x i32] [i32 0, i32 2, i32 1]
@allocation_count = global i32 0
@free_count = global i32 0

declare ptr @__real___wosy_core_alloc(i64, i64)
declare void @__real___wosy_core_free(ptr)
declare i32 @main()
declare void @proc_exit(i32) #0

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define ptr @__wrap___wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %count = load i32, ptr @allocation_count
  %within = icmp ult i32 %count, 3
  br i1 %within, label %classify, label %failure
classify:
  %first = icmp eq i32 %count, 0
  %first_size = icmp eq i64 %size, 52
  %first_match = and i1 %first, %first_size
  %second = icmp eq i32 %count, 1
  %second_size = icmp eq i64 %size, 52
  %second_match = and i1 %second, %second_size
  %third = icmp eq i32 %count, 2
  %third_size = icmp eq i64 %size, 57
  %third_match = and i1 %third, %third_size
  %known_size = or i1 %first_match, %second_match
  %known = or i1 %known_size, %third_match
  %aligned = icmp eq i64 %alignment, 8
  %sized = and i1 %known, %aligned
  br i1 %sized, label %allocate, label %failure
allocate:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %present = icmp ne ptr %pointer, null
  br i1 %present, label %record, label %failure
record:
  %slot = getelementptr [3 x ptr], ptr @addresses, i32 0, i32 %count
  store ptr %pointer, ptr %slot
  %next = add i32 %count, 1
  store i32 %next, ptr @allocation_count
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
  %within = icmp ult i32 %index, 3
  br i1 %within, label %compare, label %failure
compare:
  %slot = getelementptr [3 x ptr], ptr @addresses, i32 0, i32 %index
  %expected = load ptr, ptr %slot
  %matched = icmp eq ptr %expected, %pointer
  br i1 %matched, label %found, label %advance
advance:
  %next = add i32 %index, 1
  br label %test
found:
  %flag = getelementptr [3 x i1], ptr @freed, i32 0, i32 %index
  %was_freed = load i1, ptr %flag
  %present = icmp ne ptr %expected, null
  %unused = xor i1 %was_freed, true
  %valid = and i1 %present, %unused
  br i1 %valid, label %ordered, label %failure
ordered:
  %release_count = load i32, ptr @free_count
  %release_slot = getelementptr [3 x i32], ptr @release_order, i32 0, i32 %release_count
  %expected_index = load i32, ptr %release_slot
  %in_order = icmp eq i32 %index, %expected_index
  br i1 %in_order, label %release, label %failure
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
  %exit_ok = icmp eq i32 %result, 0
  %alloc_ok = icmp eq i32 %allocations, 3
  %free_ok = icmp eq i32 %frees, 3
  %a = and i1 %exit_ok, %alloc_ok
  %ok = and i1 %a, %free_ok
  br i1 %ok, label %success, label %failure
success:
  ret void
failure:
  call void @proc_exit(i32 80)
  unreachable
}
