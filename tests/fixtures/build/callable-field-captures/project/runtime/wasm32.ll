target triple = "wasm32-wasi"

@__fixture_array_allocations = global i32 0
@__fixture_first_array = global ptr null
@__fixture_second_array = global ptr null
@__fixture_third_array = global ptr null
@__fixture_fourth_array = global ptr null
@__fixture_fifth_array = global ptr null
@__fixture_first_array_live = global i1 false
@__fixture_second_array_live = global i1 false
@__fixture_third_array_live = global i1 false
@__fixture_fourth_array_live = global i1 false
@__fixture_fifth_array_live = global i1 false
@__fixture_first_frees = global i32 0
@__fixture_second_frees = global i32 0
@__fixture_third_frees = global i32 0
@__fixture_fourth_frees = global i32 0
@__fixture_fifth_frees = global i32 0
@__fixture_other_frees = global i32 0
@__fixture_free_invocations = global i32 0
@__fixture_owner_count = global i32 0
@__fixture_owners = global [16 x ptr] zeroinitializer
@__fixture_owner_live = global [16 x i1] zeroinitializer
@__fixture_owner_released = global i32 0

declare ptr @__real___wosy_core_alloc(i64, i64)
declare void @__real___wosy_core_free(ptr)

define ptr @__wrap___wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %pointer = call ptr @__real___wosy_core_alloc(i64 %size, i64 %alignment)
  %is_two_bytes = icmp eq i64 %size, 2
  %is_three_bytes = icmp eq i64 %size, 3
  %is_array_size = or i1 %is_two_bytes, %is_three_bytes
  %is_byte_aligned = icmp eq i64 %alignment, 1
  %is_array = and i1 %is_array_size, %is_byte_aligned
  %succeeded = icmp ne ptr %pointer, null
  %track = and i1 %is_array, %succeeded
  br i1 %track, label %record, label %record_generic

record:
  %count = load i32, ptr @__fixture_array_allocations
  %first = icmp eq i32 %count, 0
  br i1 %first, label %record_first, label %record_second

record_first:
  store ptr %pointer, ptr @__fixture_first_array
  store i1 true, ptr @__fixture_first_array_live
  br label %increment

record_second:
  %second = icmp eq i32 %count, 1
  br i1 %second, label %store_second, label %check_third

store_second:
  store ptr %pointer, ptr @__fixture_second_array
  store i1 true, ptr @__fixture_second_array_live
  br label %increment

check_third:
  %third = icmp eq i32 %count, 2
  br i1 %third, label %store_third, label %check_fourth

store_third:
  store ptr %pointer, ptr @__fixture_third_array
  store i1 true, ptr @__fixture_third_array_live
  br label %increment

check_fourth:
  %fourth = icmp eq i32 %count, 3
  br i1 %fourth, label %store_fourth, label %store_fifth

store_fourth:
  store ptr %pointer, ptr @__fixture_fourth_array
  store i1 true, ptr @__fixture_fourth_array_live
  br label %increment

store_fifth:
  store ptr %pointer, ptr @__fixture_fifth_array
  store i1 true, ptr @__fixture_fifth_array_live
  br label %increment

increment:
  %next = add i32 %count, 1
  store i32 %next, ptr @__fixture_array_allocations
  br label %done

record_generic:
  %owner_index = load i32, ptr @__fixture_owner_count
  %capacity = icmp ult i32 %owner_index, 16
  br i1 %capacity, label %record_owner, label %done

record_owner:
  %owner_slot = getelementptr inbounds [16 x ptr], ptr @__fixture_owners, i32 0, i32 %owner_index
  %owner_live_slot = getelementptr inbounds [16 x i1], ptr @__fixture_owner_live, i32 0, i32 %owner_index
  store ptr %pointer, ptr %owner_slot
  store i1 true, ptr %owner_live_slot
  %owner_next = add i32 %owner_index, 1
  store i32 %owner_next, ptr @__fixture_owner_count
  br label %done

done:
  ret ptr %pointer
}

define void @__wrap___wosy_core_free(ptr %pointer) {
entry:
  %invocations = load i32, ptr @__fixture_free_invocations
  %next_invocations = add i32 %invocations, 1
  store i32 %next_invocations, ptr @__fixture_free_invocations
  br label %check_first_array

check_first_array:
  %first_array = call i1 @__fixture_count_free(ptr %pointer, ptr @__fixture_first_array, ptr @__fixture_first_array_live, ptr @__fixture_first_frees)
  br i1 %first_array, label %done, label %check_second_array

check_second_array:
  %second_array = call i1 @__fixture_count_free(ptr %pointer, ptr @__fixture_second_array, ptr @__fixture_second_array_live, ptr @__fixture_second_frees)
  br i1 %second_array, label %done, label %check_third_array

check_third_array:
  %third_array = call i1 @__fixture_count_free(ptr %pointer, ptr @__fixture_third_array, ptr @__fixture_third_array_live, ptr @__fixture_third_frees)
  br i1 %third_array, label %done, label %check_fourth_array

check_fourth_array:
  %fourth_array = call i1 @__fixture_count_free(ptr %pointer, ptr @__fixture_fourth_array, ptr @__fixture_fourth_array_live, ptr @__fixture_fourth_frees)
  br i1 %fourth_array, label %done, label %check_fifth_array

check_fifth_array:
  %fifth_array = call i1 @__fixture_count_free(ptr %pointer, ptr @__fixture_fifth_array, ptr @__fixture_fifth_array_live, ptr @__fixture_fifth_frees)
  br i1 %fifth_array, label %done, label %check_generic

check_generic:
  %generic_match = call i1 @__fixture_release_owner(ptr %pointer)
  br i1 %generic_match, label %done, label %count_other

count_other:
  %other_count = load i32, ptr @__fixture_other_frees
  %next_other = add i32 %other_count, 1
  store i32 %next_other, ptr @__fixture_other_frees
  br label %done

done:
  call void @__real___wosy_core_free(ptr %pointer)
  ret void
}

define i1 @__fixture_count_free(ptr %actual, ptr %slot, ptr %live_slot, ptr %counter) {
entry:
  %expected = load ptr, ptr %slot
  %live = load i1, ptr %live_slot
  %present = icmp ne ptr %expected, null
  %same = icmp eq ptr %actual, %expected
  %address_matches = and i1 %present, %same
  %matches = and i1 %live, %address_matches
  br i1 %matches, label %increment, label %done

increment:
  %count = load i32, ptr %counter
  %next = add i32 %count, 1
  store i32 %next, ptr %counter
  store i1 false, ptr %live_slot
  br label %done

done:
  ret i1 %matches
}

define i1 @__fixture_release_owner(ptr %actual) {
entry:
  br label %loop

loop:
  %index = phi i32 [ 0, %entry ], [ %next, %continue ]
  %count = load i32, ptr @__fixture_owner_count
  %in_bounds = icmp ult i32 %index, %count
  br i1 %in_bounds, label %inspect, label %missing

inspect:
  %slot = getelementptr inbounds [16 x ptr], ptr @__fixture_owners, i32 0, i32 %index
  %expected = load ptr, ptr %slot
  %same = icmp eq ptr %actual, %expected
  %live_slot = getelementptr inbounds [16 x i1], ptr @__fixture_owner_live, i32 0, i32 %index
  %live = load i1, ptr %live_slot
  %matches = and i1 %same, %live
  br i1 %matches, label %release, label %continue

release:
  store i1 false, ptr %live_slot
  %released = load i32, ptr @__fixture_owner_released
  %one_more = add i32 %released, 1
  store i32 %one_more, ptr @__fixture_owner_released
  ret i1 true

continue:
  %next = add i32 %index, 1
  br label %loop

missing:
  ret i1 false
}
