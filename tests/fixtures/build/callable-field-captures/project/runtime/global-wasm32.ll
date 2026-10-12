target triple = "wasm32-wasi"

@__global_allocations = global i32 0
@__global_frees = global i32 0
@__global_live = global ptr null

declare ptr @__real___wosy_core_alloc(i64, i64)
declare void @__real___wosy_core_free(ptr)
declare void @proc_exit(i32) #0

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define ptr @__wrap___wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %count = load i32, ptr @__global_allocations
  %next = add i32 %count, 1
  store i32 %next, ptr @__global_allocations
  %first = icmp eq i32 %next, 1
  %present = icmp ne ptr %pointer, null
  %valid = and i1 %first, %present
  br i1 %valid, label %record, label %fail

record:
  store ptr %pointer, ptr @__global_live
  ret ptr %pointer

fail:
  call void @proc_exit(i32 79)
  unreachable
}

define void @__wrap___wosy_core_free(ptr %pointer) {
entry:
  %live = load ptr, ptr @__global_live
  %matches = icmp eq ptr %pointer, %live
  %present = icmp ne ptr %live, null
  %valid = and i1 %matches, %present
  br i1 %valid, label %release, label %fail

release:
  store ptr null, ptr @__global_live
  %count = load i32, ptr @__global_frees
  %next = add i32 %count, 1
  store i32 %next, ptr @__global_frees
  call void @__real___wosy_core_free(ptr %pointer)
  ret void

fail:
  call void @proc_exit(i32 79)
  unreachable
}
