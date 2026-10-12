target triple = "wasm32-wasi"

@addresses = global [6 x ptr] zeroinitializer
@freed = global [6 x i1] zeroinitializer
@sizes = global [6 x i32] zeroinitializer
@allocation_count = global i32 0
@free_count = global i32 0
@line = global [38 x i8] zeroinitializer

declare ptr @__real___wosy_core_alloc(i64, i64)
declare void @__real___wosy_core_free(ptr)
declare i32 @main()
declare void @proc_exit(i32) #0
declare i32 @fd_write(i32, ptr, i32, ptr) #1

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }
attributes #1 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="fd_write" }

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

define void @trace(i32 %reason) {
entry:
  %iov = alloca { ptr, i32 }
  %written = alloca i32
  store [38 x i8] c"generation=00 size=00 freed=0 site=E\0A\00", ptr @line
  br label %test
test:
  %index = phi i32 [ 0, %entry ], [ %next, %emit ]
  %count = load i32, ptr @allocation_count
  %within = icmp ult i32 %index, %count
  br i1 %within, label %format, label %finish
format:
  %tens = udiv i32 %index, 10
  %units = urem i32 %index, 10
  %digit1 = add i32 %tens, 48
  %digit2 = add i32 %units, 48
  %char1 = trunc i32 %digit1 to i8
  %char2 = trunc i32 %digit2 to i8
  store i8 %char1, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 11)
  store i8 %char2, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 12)
  %size_slot = getelementptr [6 x i32], ptr @sizes, i32 0, i32 %index
  %size = load i32, ptr %size_slot
  %size_tens = udiv i32 %size, 10
  %size_units = urem i32 %size, 10
  %size_digit1 = add i32 %size_tens, 48
  %size_digit2 = add i32 %size_units, 48
  %size_char1 = trunc i32 %size_digit1 to i8
  %size_char2 = trunc i32 %size_digit2 to i8
  store i8 %size_char1, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 19)
  store i8 %size_char2, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 20)
  %flag_slot = getelementptr [6 x i1], ptr @freed, i32 0, i32 %index
  %flag = load i1, ptr %flag_slot
  %flag_digit = zext i1 %flag to i8
  %flag_char = add i8 %flag_digit, 48
  store i8 %flag_char, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 28)
  %first_cell = icmp eq i32 %index, 1
  %second_cell = icmp eq i32 %index, 3
  %cell = or i1 %first_cell, %second_cell
  %array = icmp eq i32 %index, 4
  %environment_or_cell = select i1 %cell, i8 67, i8 69
  %site = select i1 %array, i8 65, i8 %environment_or_cell
  store i8 %site, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 35)
  store { ptr, i32 } { ptr @line, i32 37 }, ptr %iov
  %result = call i32 @fd_write(i32 2, ptr %iov, i32 1, ptr %written)
  br label %emit
emit:
  %next = add i32 %index, 1
  br label %test
finish:
  call void @proc_exit(i32 %reason)
  unreachable
}

define void @__wrap___wosy_core_system_panic() {
entry:
  call void @trace(i32 74)
  unreachable
}

define ptr @__wrap___wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %count = load i32, ptr @allocation_count
  %within = icmp ult i32 %count, 6
  %array = icmp eq i32 %count, 4
  %first_cell = icmp eq i32 %count, 1
  %second_cell = icmp eq i32 %count, 3
  %cell = or i1 %first_cell, %second_cell
  %cell_size = select i1 %cell, i64 57, i64 52
  %expected_size = select i1 %array, i64 28, i64 %cell_size
  %size_ok = icmp eq i64 %size, %expected_size
  %expected_alignment = select i1 %array, i64 4, i64 8
  %alignment_ok = icmp eq i64 %alignment, %expected_alignment
  %valid_size = and i1 %size_ok, %alignment_ok
  %valid = and i1 %within, %valid_size
  br i1 %valid, label %allocate, label %failure
allocate:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %present = icmp ne ptr %pointer, null
  br i1 %present, label %record, label %failure
record:
  %slot = getelementptr [6 x ptr], ptr @addresses, i32 0, i32 %count
  store ptr %pointer, ptr %slot
  %size_slot = getelementptr [6 x i32], ptr @sizes, i32 0, i32 %count
  %short_size = trunc i64 %size to i32
  store i32 %short_size, ptr %size_slot
  %next = add i32 %count, 1
  store i32 %next, ptr @allocation_count
  ret ptr %pointer
failure:
  call void @trace(i32 78)
  unreachable
}

define void @__wrap___wosy_core_free(ptr %pointer) {
entry:
  br label %test
test:
  %index = phi i32 [ 0, %entry ], [ %next, %advance ]
  %count = load i32, ptr @allocation_count
  %within = icmp ult i32 %index, %count
  br i1 %within, label %compare, label %failure
compare:
  %slot = getelementptr [6 x ptr], ptr @addresses, i32 0, i32 %index
  %expected = load ptr, ptr %slot
  %matched = icmp eq ptr %expected, %pointer
  %flag_slot = getelementptr [6 x i1], ptr @freed, i32 0, i32 %index
  %was_freed = load i1, ptr %flag_slot
  %live = xor i1 %was_freed, true
  %matched_live = and i1 %matched, %live
  br i1 %matched_live, label %release, label %advance
advance:
  %next = add i32 %index, 1
  br label %test
release:
  store i1 true, ptr %flag_slot
  %old_count = load i32, ptr @free_count
  %new_count = add i32 %old_count, 1
  store i32 %new_count, ptr @free_count
  call void @__real___wosy_core_free(ptr %pointer)
  ret void
failure:
  call void @trace(i32 79)
  unreachable
}

define void @_start() {
entry:
  %result = call i32 @main()
  %allocations = load i32, ptr @allocation_count
  %frees = load i32, ptr @free_count
  %exit_ok = icmp eq i32 %result, 0
  %alloc_ok = icmp eq i32 %allocations, 6
  %free_ok = icmp eq i32 %frees, 6
  %counts = and i1 %alloc_ok, %free_ok
  %ok = and i1 %exit_ok, %counts
  br i1 %ok, label %success, label %failure
success:
  ret void
failure:
  call void @trace(i32 80)
  unreachable
}
