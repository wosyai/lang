target triple = "wasm32-wasi"

declare ptr @__wosy_core_alloc(i64, i64)
declare void @__wosy_core_free(ptr)
declare i32 @llvm.wasm.memory.size.i32(i32)
declare void @proc_exit(i32) #0

attributes #0 = { "wasm-import-module"="wasi_snapshot_preview1" "wasm-import-name"="proc_exit" }

define internal void @require(i1 %condition, i32 %code) {
entry:
  br i1 %condition, label %pass, label %fail

fail:
  call void @proc_exit(i32 %code)
  unreachable

pass:
  ret void
}

define internal ptr @allocate(i64 %size, i64 %alignment, i32 %code) {
entry:
  %pointer = call ptr @__wosy_core_alloc(i64 %size, i64 %alignment)
  %nonnull = icmp ne ptr %pointer, null
  call void @require(i1 %nonnull, i32 %code)
  %address = ptrtoint ptr %pointer to i32
  %a = trunc i64 %alignment to i32
  %mask = sub i32 %a, 1
  %bits = and i32 %address, %mask
  %aligned = icmp eq i32 %bits, 0
  call void @require(i1 %aligned, i32 %code)
  ret ptr %pointer
}

define void @_start() {
entry:
  call void @__wosy_core_free(ptr null)
  %invalid_alignment = call ptr @__wosy_core_alloc(i64 1, i64 3)
  %invalid_failed = icmp eq ptr %invalid_alignment, null
  call void @require(i1 %invalid_failed, i32 30)
  %tail = call ptr @allocate(i64 1024, i64 8, i32 31)
  call void @__wosy_core_free(ptr %tail)
  %oversized = call ptr @__wosy_core_alloc(i64 196608, i64 8)
  %failed = icmp eq ptr %oversized, null
  call void @require(i1 %failed, i32 32)
  %tail_again = call ptr @allocate(i64 1024, i64 8, i32 33)
  %tail_reused = icmp eq ptr %tail_again, %tail
  call void @require(i1 %tail_reused, i32 34)
  call void @__wosy_core_free(ptr %tail_again)
  %aligned = call ptr @allocate(i64 800, i64 128, i32 35)
  %prefix = call ptr @allocate(i64 60, i64 1, i32 36)
  %second = call ptr @allocate(i64 60, i64 2, i32 37)
  %prefix_available = icmp ult ptr %prefix, %aligned
  %suffix_available = icmp ult ptr %second, %aligned
  %brackets = xor i1 %prefix_available, %suffix_available
  call void @require(i1 %brackets, i32 38)
  call void @__wosy_core_free(ptr %prefix)
  call void @__wosy_core_free(ptr %second)
  call void @__wosy_core_free(ptr %aligned)

  %a = call ptr @allocate(i64 64, i64 8, i32 39)
  %b = call ptr @allocate(i64 64, i64 8, i32 40)
  %c = call ptr @allocate(i64 64, i64 8, i32 41)
  %d = call ptr @allocate(i64 64, i64 8, i32 42)
  %e = call ptr @allocate(i64 64, i64 8, i32 43)
  store volatile i8 91, ptr %e
  call void @__wosy_core_free(ptr %b)
  call void @__wosy_core_free(ptr %a)
  %joined = call ptr @allocate(i64 106, i64 8, i32 44)
  %joins_the_pair = icmp eq ptr %joined, %a
  call void @require(i1 %joins_the_pair, i32 45)
  call void @__wosy_core_free(ptr %d)
  call void @__wosy_core_free(ptr %c)
  %survivors_half = call ptr @allocate(i64 150, i64 8, i32 46)
  %starts_before_c = icmp ult ptr %survivors_half, %c
  %starts_after_joined = icmp ugt ptr %survivors_half, %joined
  %joins_remaining = and i1 %starts_before_c, %starts_after_joined
  call void @require(i1 %joins_remaining, i32 47)
  %survivors_last = getelementptr i8, ptr %survivors_half, i32 149
  store volatile i8 87, ptr %survivors_last
  %sentinel = load volatile i8, ptr %e
  %ends_at_live = icmp eq i8 %sentinel, 91
  call void @require(i1 %ends_at_live, i32 48)
  call void @__wosy_core_free(ptr %survivors_half)
  call void @__wosy_core_free(ptr %joined)
  call void @__wosy_core_free(ptr %e)

  %zero_one = call ptr @allocate(i64 0, i64 1, i32 49)
  %zero_two = call ptr @allocate(i64 0, i64 2, i32 50)
  %zero_distinct = icmp ne ptr %zero_one, %zero_two
  call void @require(i1 %zero_distinct, i32 51)
  call void @__wosy_core_free(ptr %zero_two)
  call void @__wosy_core_free(ptr %zero_one)
  %probe = call ptr @allocate(i64 1, i64 8, i32 52)
  call void @__wosy_core_free(ptr %probe)
  %probe_addr = ptrtoint ptr %probe to i32
  %probe_addr64 = zext i32 %probe_addr to i64
  %pages = call i32 @llvm.wasm.memory.size.i32(i32 0)
  %pages64 = zext i32 %pages to i64
  %limit = shl i64 %pages64, 16
  %available = sub i64 %limit, %probe_addr64
  %at_limit = call ptr @allocate(i64 %available, i64 8, i32 53)
  %actual_limit_start = icmp eq ptr %at_limit, %probe
  call void @require(i1 %actual_limit_start, i32 54)
  %last = sub i64 %available, 1
  %last_ptr = getelementptr i8, ptr %at_limit, i64 %last
  store volatile i8 127, ptr %last_ptr
  %beyond_limit = call ptr @__wosy_core_alloc(i64 0, i64 1)
  %beyond_failed = icmp eq ptr %beyond_limit, null
  call void @require(i1 %beyond_failed, i32 55)
  %last_value = load volatile i8, ptr %last_ptr
  %still_live = icmp eq i8 %last_value, 127
  call void @require(i1 %still_live, i32 56)
  call void @__wosy_core_free(ptr %at_limit)
  ret void
}
