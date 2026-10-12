target triple = "wasm32-wasi"

@addresses = global [3 x ptr] zeroinitializer
@freed = global [3 x i1] zeroinitializer
@sizes = global [3 x i32] zeroinitializer
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
  store [38 x i8] c"generation=00 size=000 freed=0\0A\00\00\00\00\00\00\00", ptr @line
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
  %size_slot = getelementptr [3 x i32], ptr @sizes, i32 0, i32 %index
  %size = load i32, ptr %size_slot
  %hundreds = udiv i32 %size, 100
  %rest = urem i32 %size, 100
  %size_tens = udiv i32 %rest, 10
  %size_units = urem i32 %rest, 10
  %hundred_digit = add i32 %hundreds, 48
  %tens_digit = add i32 %size_tens, 48
  %units_digit = add i32 %size_units, 48
  %hundred_char = trunc i32 %hundred_digit to i8
  %tens_char = trunc i32 %tens_digit to i8
  %units_char = trunc i32 %units_digit to i8
  store i8 %hundred_char, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 18)
  store i8 %tens_char, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 19)
  store i8 %units_char, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 20)
  %flag_slot = getelementptr [3 x i1], ptr @freed, i32 0, i32 %index
  %flag = load i1, ptr %flag_slot
  %flag_digit = zext i1 %flag to i8
  %flag_char = add i8 %flag_digit, 48
  store i8 %flag_char, ptr getelementptr ([38 x i8], ptr @line, i32 0, i32 28)
  store { ptr, i32 } { ptr @line, i32 30 }, ptr %iov
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
  %within = icmp ult i32 %count, 3
  br i1 %within, label %classify, label %failure
classify:
  %is_first = icmp eq i32 %count, 0
  %backing_size = icmp eq i64 %size, 2
  %backing_align = icmp eq i64 %alignment, 1
  %backing_size_ok = and i1 %backing_size, %backing_align
  %backing = and i1 %is_first, %backing_size_ok
  %is_second = icmp eq i32 %count, 1
  %env_size = icmp eq i64 %size, 64
  %env_align = icmp eq i64 %alignment, 8
  %env_size_ok = and i1 %env_size, %env_align
  %environment = and i1 %is_second, %env_size_ok
  %is_third = icmp eq i32 %count, 2
  %cell_size = icmp eq i64 %size, 72
  %cell_align = icmp eq i64 %alignment, 8
  %cell_size_ok = and i1 %cell_size, %cell_align
  %cell = and i1 %is_third, %cell_size_ok
  %known_backing = or i1 %backing, %environment
  %known = or i1 %known_backing, %cell
  br i1 %known, label %allocate, label %failure
allocate:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %present = icmp ne ptr %pointer, null
  br i1 %present, label %record, label %failure
record:
  %slot = getelementptr [3 x ptr], ptr @addresses, i32 0, i32 %count
  store ptr %pointer, ptr %slot
  %size_slot = getelementptr [3 x i32], ptr @sizes, i32 0, i32 %count
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
  call void @trace(i32 79)
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
  call void @trace(i32 80)
  unreachable
}
