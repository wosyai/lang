target triple = "wasm32-wasi"

declare i32 @main()
declare void @proc_exit(i32) #0
@__fixture_array_allocations = external global i32
@__fixture_first_frees = external global i32
@__fixture_second_frees = external global i32
@__fixture_third_frees = external global i32
@__fixture_fourth_frees = external global i32
@__fixture_fifth_frees = external global i32
@__fixture_other_frees = external global i32
@__fixture_free_invocations = external global i32
@__fixture_first_array_live = external global i1
@__fixture_second_array_live = external global i1
@__fixture_third_array_live = external global i1
@__fixture_fourth_array_live = external global i1
@__fixture_fifth_array_live = external global i1
@__fixture_owner_count = external global i32
@__fixture_owner_released = external global i32

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define void @_start() {
entry:
  call i32 @main()
  %allocations = load i32, ptr @__fixture_array_allocations
  %first = load i32, ptr @__fixture_first_frees
  %second = load i32, ptr @__fixture_second_frees
  %third = load i32, ptr @__fixture_third_frees
  %fourth = load i32, ptr @__fixture_fourth_frees
  %fifth = load i32, ptr @__fixture_fifth_frees
  %other = load i32, ptr @__fixture_other_frees
  %invocations = load i32, ptr @__fixture_free_invocations
  %first_array_live = load i1, ptr @__fixture_first_array_live
  %second_array_live = load i1, ptr @__fixture_second_array_live
  %third_array_live = load i1, ptr @__fixture_third_array_live
  %fourth_array_live = load i1, ptr @__fixture_fourth_array_live
  %fifth_array_live = load i1, ptr @__fixture_fifth_array_live
  %owner_count = load i32, ptr @__fixture_owner_count
  %owner_released = load i32, ptr @__fixture_owner_released
  %ten_owners = icmp eq i32 %owner_count, 10
  %ten_released = icmp eq i32 %owner_released, 10
  %generic_owners = and i1 %ten_owners, %ten_released
  %five_allocations = icmp eq i32 %allocations, 5
  %first_once = icmp eq i32 %first, 1
  %second_once = icmp eq i32 %second, 1
  %third_once = icmp eq i32 %third, 1
  %fourth_once = icmp eq i32 %fourth, 1
  %fifth_once = icmp eq i32 %fifth, 1
  %no_other = icmp eq i32 %other, 0
  %fifteen_frees = icmp eq i32 %invocations, 15
  %live_arrays_a = or i1 %first_array_live, %second_array_live
  %live_arrays_b = or i1 %live_arrays_a, %third_array_live
  %live_arrays_c = or i1 %live_arrays_b, %fourth_array_live
  %live_arrays = or i1 %live_arrays_c, %fifth_array_live
  %all_released = xor i1 %live_arrays, true
  %all_tracked_and_released = and i1 %generic_owners, %all_released
  br i1 %five_allocations, label %check_first, label %failure_allocations

failure_allocations:
  %allocation_exit = add i32 %allocations, 20
  call void @proc_exit(i32 %allocation_exit)
  unreachable

check_first:
  br i1 %first_once, label %check_second, label %failure_first

failure_first:
  %first_exit = add i32 %first, 40
  call void @proc_exit(i32 %first_exit)
  unreachable

check_second:
  br i1 %second_once, label %check_third, label %failure_second

failure_second:
  %second_exit = add i32 %second, 60
  call void @proc_exit(i32 %second_exit)
  unreachable

check_third:
  br i1 %third_once, label %check_fourth, label %failure_third

failure_third:
  %third_exit = add i32 %third, 70
  call void @proc_exit(i32 %third_exit)
  unreachable

check_fourth:
  br i1 %fourth_once, label %check_fifth, label %failure_fourth

failure_fourth:
  call void @proc_exit(i32 71)
  unreachable

check_fifth:
  br i1 %fifth_once, label %check_tracked, label %failure_fifth

failure_fifth:
  call void @proc_exit(i32 72)
  unreachable

check_tracked:
  br i1 %all_tracked_and_released, label %check_other, label %failure_tracked

failure_tracked:
  call void @proc_exit(i32 79)
  unreachable

check_other:
  br i1 %no_other, label %check_invocations, label %failure_other

failure_other:
  %other_exit = add i32 %other, 80
  call void @proc_exit(i32 %other_exit)
  unreachable

check_invocations:
  br i1 %fifteen_frees, label %success, label %failure_invocations

failure_invocations:
  %invocations_exit = add i32 %invocations, 100
  call void @proc_exit(i32 %invocations_exit)
  unreachable

success:
  ret void
}
