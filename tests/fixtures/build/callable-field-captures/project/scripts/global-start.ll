target triple = "wasm32-wasi"

declare i32 @main()
declare void @proc_exit(i32) #0
@__global_allocations = external global i32
@__global_frees = external global i32
@__global_live = external global ptr

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define void @_start() {
entry:
  call i32 @main()
  %allocations = load i32, ptr @__global_allocations
  %frees = load i32, ptr @__global_frees
  %live = load ptr, ptr @__global_live
  %allocated_once = icmp eq i32 %allocations, 1
  %freed_once = icmp eq i32 %frees, 1
  %all_released = icmp eq ptr %live, null
  %counts = and i1 %allocated_once, %freed_once
  %valid = and i1 %counts, %all_released
  br i1 %valid, label %done, label %fail

fail:
  call void @proc_exit(i32 79)
  unreachable

done:
  ret void
}
