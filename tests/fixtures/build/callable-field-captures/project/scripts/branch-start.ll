target triple = "wasm32-wasi"

declare i32 @main()
declare void @proc_exit(i32) #0
@__branch_arrays = external global [4 x ptr]
@__branch_owners = external global [12 x ptr]
@__branch_array_frees = external global [4 x i32]
@__branch_owner_frees = external global [12 x i32]
@__branch_array_count = external global i32
@__branch_owner_count = external global i32
@__branch_free_count = external global i32
@__branch_unmatched = external global i32

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define void @_start() {
entry:
  call i32 @main()
  %arrays = load i32, ptr @__branch_array_count
  %owners = load i32, ptr @__branch_owner_count
  %frees = load i32, ptr @__branch_free_count
  %unmatched = load i32, ptr @__branch_unmatched
  %arrays_ok = icmp eq i32 %arrays, 4
  %owners_ok = icmp eq i32 %owners, 12
  %frees_ok = icmp eq i32 %frees, 16
  %unmatched_ok = icmp eq i32 %unmatched, 0
  %first = and i1 %arrays_ok, %owners_ok
  %second = and i1 %frees_ok, %unmatched_ok
  %counts_ok = and i1 %first, %second
  br i1 %counts_ok, label %array_loop, label %fail

array_loop:
  %array_index = phi i32 [ 0, %entry ], [ %next_array, %array_continue ]
  %arrays_done = icmp eq i32 %array_index, 4
  br i1 %arrays_done, label %owner_loop, label %array_check

array_check:
  %array_slot = getelementptr [4 x ptr], ptr @__branch_arrays, i32 0, i32 %array_index
  %array_pointer = load ptr, ptr %array_slot
  %released = icmp eq ptr %array_pointer, null
  %count_slot = getelementptr [4 x i32], ptr @__branch_array_frees, i32 0, i32 %array_index
  %count = load i32, ptr %count_slot
  %once = icmp eq i32 %count, 1
  %valid = and i1 %released, %once
  br i1 %valid, label %array_continue, label %fail

array_continue:
  %next_array = add i32 %array_index, 1
  br label %array_loop

owner_loop:
  %owner_index = phi i32 [ 0, %array_loop ], [ %next_owner, %owner_continue ]
  %owners_done = icmp eq i32 %owner_index, 12
  br i1 %owners_done, label %success, label %owner_check

owner_check:
  %owner_slot = getelementptr [12 x ptr], ptr @__branch_owners, i32 0, i32 %owner_index
  %owner_pointer = load ptr, ptr %owner_slot
  %released_owner = icmp eq ptr %owner_pointer, null
  %owner_count_slot = getelementptr [12 x i32], ptr @__branch_owner_frees, i32 0, i32 %owner_index
  %owner_count = load i32, ptr %owner_count_slot
  %owner_once = icmp eq i32 %owner_count, 1
  %owner_valid = and i1 %released_owner, %owner_once
  br i1 %owner_valid, label %owner_continue, label %fail

owner_continue:
  %next_owner = add i32 %owner_index, 1
  br label %owner_loop

fail:
  call void @proc_exit(i32 79)
  unreachable

success:
  ret void
}
