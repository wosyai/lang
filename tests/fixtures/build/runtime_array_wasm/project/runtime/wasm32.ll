target triple = "wasm32-wasi"

@__wosy_core_heap_cursor = internal global i32 65536

declare i32 @llvm.wasm.memory.size.i32(i32)
declare i32 @llvm.wasm.memory.grow.i32(i32, i32)
declare void @proc_exit(i32) #0

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define ptr @__wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %fits_address_space = icmp ule i64 %size, 4294967295
  br i1 %fits_address_space, label %allocate, label %failure

allocate:
  %size32 = trunc i64 %size to i32
  %alignment32 = trunc i64 %alignment to i32
  %cursor = load i32, ptr @__wosy_core_heap_cursor
  %alignment_mask = sub i32 %alignment32, 1
  %aligned_sum = add i32 %cursor, %alignment_mask
  %alignment_inverse = xor i32 %alignment_mask, -1
  %aligned_pointer = and i32 %aligned_sum, %alignment_inverse
  %allocation_end = add i32 %aligned_pointer, %size32
  %current_pages = call i32 @llvm.wasm.memory.size.i32(i32 0)
  %current_bytes = shl i32 %current_pages, 16
  %has_capacity = icmp ule i32 %allocation_end, %current_bytes
  br i1 %has_capacity, label %commit, label %grow

grow:
  %required_bytes = sub i32 %allocation_end, %current_bytes
  %pages_before_round = add i32 %required_bytes, 65535
  %pages = lshr i32 %pages_before_round, 16
  %grown_pages = call i32 @llvm.wasm.memory.grow.i32(i32 0, i32 %pages)
  %growth_failed = icmp eq i32 %grown_pages, -1
  br i1 %growth_failed, label %failure, label %commit

commit:
  store i32 %allocation_end, ptr @__wosy_core_heap_cursor
  %pointer = inttoptr i32 %aligned_pointer to ptr
  ret ptr %pointer

failure:
  ret ptr null
}

define void @__wosy_core_free(ptr %pointer) {
entry:
  ret void
}

define void @__wosy_core_system_panic() {
entry:
  call void @proc_exit(i32 1)
  unreachable
}
