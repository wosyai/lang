declare i32 @llvm.ctlz.i32(i32, i1 immarg)
declare i32 @llvm.cttz.i32(i32, i1 immarg)

define internal void @__wosy_wasm_block(i32 %h, i32 %span, i32 %prev, i32 %log, i32 %free) {
entry:
  %p = inttoptr i32 %h to ptr
  %slot = getelementptr i8, ptr %p, i32 4
  %log_lo = and i32 %log, 3
  %log_shifted = shl i32 %log_lo, 1
  %flags = or i32 %log_shifted, %free
  %span_word = or i32 %span, %flags
  %log_hi = lshr i32 %log, 2
  %prev_word = or i32 %prev, %log_hi
  store i32 %span_word, ptr %p, align 4
  store i32 %prev_word, ptr %slot, align 4
  ret void
}

define internal void @__wosy_wasm_set_prev(i32 %h, i32 %prev) {
entry:
  %p = inttoptr i32 %h to ptr
  %slot = getelementptr i8, ptr %p, i32 4
  %word = load i32, ptr %slot, align 4
  %align_bits = and i32 %word, 7
  %new = or i32 %prev, %align_bits
  store i32 %new, ptr %slot, align 4
  ret void
}

define internal void @__wosy_wasm_unlink(i32 %h) {
entry:
  %p = inttoptr i32 %h to ptr
  %back_slot = getelementptr i8, ptr %p, i32 8
  %next_slot = getelementptr i8, ptr %p, i32 12
  %back = load i32, ptr %back_slot, align 4
  %next = load i32, ptr %next_slot, align 4
  %has_back = icmp ne i32 %back, 0
  br i1 %has_back, label %update_back, label %update_head

update_back:
  %back_ptr = inttoptr i32 %back to ptr
  %back_next = getelementptr i8, ptr %back_ptr, i32 12
  store i32 %next, ptr %back_next, align 4
  br label %try_next

update_head:
  store i32 %next, ptr @__wosy_core_free_head, align 4
  br label %try_next

try_next:
  %has_next = icmp ne i32 %next, 0
  br i1 %has_next, label %update_next, label %complete

update_next:
  %next_ptr = inttoptr i32 %next to ptr
  %next_back = getelementptr i8, ptr %next_ptr, i32 8
  store i32 %back, ptr %next_back, align 4
  br label %complete

complete:
  ret void
}

define internal void @__wosy_wasm_push(i32 %h) {
entry:
  %p = inttoptr i32 %h to ptr
  %back_slot = getelementptr i8, ptr %p, i32 8
  %next_slot = getelementptr i8, ptr %p, i32 12
  %head = load i32, ptr @__wosy_core_free_head, align 4
  store i32 0, ptr %back_slot, align 4
  store i32 %head, ptr %next_slot, align 4
  store i32 %h, ptr @__wosy_core_free_head, align 4
  %has_head = icmp ne i32 %head, 0
  br i1 %has_head, label %link, label %complete

link:
  %head_ptr = inttoptr i32 %head to ptr
  %head_back = getelementptr i8, ptr %head_ptr, i32 8
  store i32 %h, ptr %head_back, align 4
  br label %complete

complete:
  ret void
}

define internal void @__wosy_wasm_fragment(i32 %h, i32 %span, i32 %prev) {
entry:
  %first_before = add i32 %h, 11
  %last_offset = sub i32 %span, 1
  %last = add i32 %h, %last_offset
  %difference = xor i32 %first_before, %last
  %leading = call i32 @llvm.ctlz.i32(i32 %difference, i1 true)
  %log = sub i32 31, %leading
  call void @__wosy_wasm_block(i32 %h, i32 %span, i32 %prev, i32 %log, i32 1)
  call void @__wosy_wasm_push(i32 %h)
  ret void
}

define ptr @__wosy_core_alloc(i64 %size, i64 %alignment) {
entry:
  %size_fits = icmp ule i64 %size, 4294967295
  %alignment_fits = icmp ule i64 %alignment, 2147483648
  %nonzero_alignment = icmp ne i64 %alignment, 0
  %alignment_minus_one = sub i64 %alignment, 1
  %alignment_power_mask = and i64 %alignment, %alignment_minus_one
  %power_of_two = icmp eq i64 %alignment_power_mask, 0
  %range_valid = and i1 %size_fits, %alignment_fits
  %alignment_valid = and i1 %nonzero_alignment, %power_of_two
  %valid = and i1 %range_valid, %alignment_valid
  br i1 %valid, label %prepare, label %failure

prepare:
  %cursor = load i64, ptr @__wosy_core_heap_cursor
  %zero_size = icmp eq i64 %size, 0
  %extent = select i1 %zero_size, i64 1, i64 %size
  %inverse = xor i64 %alignment_minus_one, -1
  %alignment32 = trunc i64 %alignment to i32
  %log = call i32 @llvm.cttz.i32(i32 %alignment32, i1 true)
  %head = load i32, ptr @__wosy_core_free_head, align 4
  %has_head = icmp ne i32 %head, 0
  br i1 %has_head, label %search, label %extend

search:
  %candidate = phi i32 [ %head, %prepare ], [ %next, %advance ]
  %candidate_ptr = inttoptr i32 %candidate to ptr
  %packed = load i32, ptr %candidate_ptr, align 4
  %candidate_span = and i32 %packed, -8
  %candidate_prev_slot = getelementptr i8, ptr %candidate_ptr, i32 4
  %candidate_prev_packed = load i32, ptr %candidate_prev_slot, align 4
  %stored_lo_shifted = lshr i32 %packed, 1
  %stored_lo = and i32 %stored_lo_shifted, 3
  %stored_hi = and i32 %candidate_prev_packed, 7
  %stored_hi_shifted = shl i32 %stored_hi, 2
  %stored_log = or i32 %stored_lo, %stored_hi_shifted
  %enough_alignment = icmp uge i32 %stored_log, %log
  br i1 %enough_alignment, label %fit, label %advance

fit:
  %candidate64 = zext i32 %candidate to i64
  %span64 = zext i32 %candidate_span to i64
  %limit = add i64 %candidate64, %span64
  %minimum = add i64 %candidate64, 12
  %adjusted = add i64 %minimum, %alignment_minus_one
  %placed = and i64 %adjusted, %inverse
  %used = add i64 %placed, %extent
  %fits = icmp ule i64 %used, %limit
  br i1 %fits, label %reuse, label %advance

advance:
  %next_slot = getelementptr i8, ptr %candidate_ptr, i32 12
  %next = load i32, ptr %next_slot, align 4
  %has_next = icmp ne i32 %next, 0
  br i1 %has_next, label %search, label %extend

reuse:
  %candidate_prev = and i32 %candidate_prev_packed, -8
  %rounded_base = add i64 %used, 7
  %rounded = and i64 %rounded_base, -8
  %splits_tail = sub i64 %limit, %rounded
  %may_split_tail = icmp uge i64 %splits_tail, 16
  %reuse_end = select i1 %may_split_tail, i64 %rounded, i64 %limit
  br label %commit

extend:
  %tail = load i32, ptr @__wosy_wasm_tail, align 4
  %has_tail = icmp ne i32 %tail, 0
  br i1 %has_tail, label %tail_kind, label %extend_first

extend_first:
  %heap_base32 = ptrtoint ptr @__heap_base to i32
  %heap_base = zext i32 %heap_base32 to i64
  ; The block headers pack flags in the low three bits. Round in i64 so a base
  ; near the wasm32 limit cannot wrap back into static data.
  %heap_round = add i64 %heap_base, 7
  %heap_start = and i64 %heap_round, -8
  br label %extend_place

tail_kind:
  %tail_ptr = inttoptr i32 %tail to ptr
  %tail_word = load i32, ptr %tail_ptr, align 4
  %tail_free_bit = and i32 %tail_word, 1
  %tail_is_free = icmp ne i32 %tail_free_bit, 0
  br i1 %tail_is_free, label %extend_free_tail, label %extend_append

extend_free_tail:
  %tail_prev_slot = getelementptr i8, ptr %tail_ptr, i32 4
  %tail_prev_word = load i32, ptr %tail_prev_slot, align 4
  %tail_prev = and i32 %tail_prev_word, -8
  %tail64 = zext i32 %tail to i64
  br label %extend_place

extend_append:
  %tail_span = and i32 %tail_word, -8
  br label %extend_place

extend_place:
  %extension_start = phi i64 [ %heap_start, %extend_first ], [ %tail64, %extend_free_tail ], [ %cursor, %extend_append ]
  %extension_prev = phi i32 [ 0, %extend_first ], [ %tail_prev, %extend_free_tail ], [ %tail_span, %extend_append ]
  %extension_unlink = phi i32 [ 0, %extend_first ], [ %tail, %extend_free_tail ], [ 0, %extend_append ]
  %extension_minimum = add i64 %extension_start, 12
  %extension_adjusted = add i64 %extension_minimum, %alignment_minus_one
  %extension_pointer = and i64 %extension_adjusted, %inverse
  %extension_used = add i64 %extension_pointer, %extent
  %extension_round = add i64 %extension_used, 7
  %extension_end = and i64 %extension_round, -8
  %end_fits = icmp ule i64 %extension_end, 4294967296
  %pointer_fits = icmp ult i64 %extension_pointer, 4294967296
  %extension_fits = and i1 %end_fits, %pointer_fits
  br i1 %extension_fits, label %capacity, label %failure

capacity:
  %pages = call i32 @llvm.wasm.memory.size.i32(i32 0)
  %pages64 = zext i32 %pages to i64
  %bytes = shl i64 %pages64, 16
  %has_capacity = icmp ule i64 %extension_end, %bytes
  br i1 %has_capacity, label %commit, label %grow

grow:
  %needed = sub i64 %extension_end, %bytes
  %needed_rounded = add i64 %needed, 65535
  %needed_pages64 = lshr i64 %needed_rounded, 16
  %needed_pages = trunc i64 %needed_pages64 to i32
  %grown = call i32 @llvm.wasm.memory.grow.i32(i32 0, i32 %needed_pages)
  %grow_failed = icmp eq i32 %grown, -1
  br i1 %grow_failed, label %failure, label %commit

commit:
  %start = phi i64 [ %candidate64, %reuse ], [ %extension_start, %capacity ], [ %extension_start, %grow ]
  %previous = phi i32 [ %candidate_prev, %reuse ], [ %extension_prev, %capacity ], [ %extension_prev, %grow ]
  %old_free = phi i32 [ %candidate, %reuse ], [ %extension_unlink, %capacity ], [ %extension_unlink, %grow ]
  %pointer = phi i64 [ %placed, %reuse ], [ %extension_pointer, %capacity ], [ %extension_pointer, %grow ]
  %reserved_end = phi i64 [ %reuse_end, %reuse ], [ %extension_end, %capacity ], [ %extension_end, %grow ]
  %block_end = phi i64 [ %limit, %reuse ], [ %extension_end, %capacity ], [ %extension_end, %grow ]
  %takes_free = icmp ne i32 %old_free, 0
  br i1 %takes_free, label %unlink, label %prefix

unlink:
  call void @__wosy_wasm_unlink(i32 %old_free)
  br label %prefix

prefix:
  %prefix_end_limit = sub i64 %pointer, 12
  %prefix_end = and i64 %prefix_end_limit, -8
  %prefix_length = sub i64 %prefix_end, %start
  %splits_prefix = icmp uge i64 %prefix_length, 16
  %prefix_span = trunc i64 %prefix_length to i32
  br i1 %splits_prefix, label %save_prefix, label %allocation

save_prefix:
  %prefix32 = trunc i64 %start to i32
  call void @__wosy_wasm_fragment(i32 %prefix32, i32 %prefix_span, i32 %previous)
  br label %allocation

allocation:
  %h = phi i64 [ %start, %prefix ], [ %prefix_end, %save_prefix ]
  %prev = phi i32 [ %previous, %prefix ], [ %prefix_span, %save_prefix ]
  %allocation_span64 = sub i64 %reserved_end, %h
  %span = trunc i64 %allocation_span64 to i32
  %h32 = trunc i64 %h to i32
  call void @__wosy_wasm_block(i32 %h32, i32 %span, i32 %prev, i32 %log, i32 0)
  %pointer32 = trunc i64 %pointer to i32
  %payload = inttoptr i32 %pointer32 to ptr
  %back = getelementptr i8, ptr %payload, i32 -4
  store i32 %h32, ptr %back, align 1
  %has_suffix = icmp ult i64 %reserved_end, %block_end
  br i1 %has_suffix, label %suffix, label %update_boundary

suffix:
  %suffix64 = sub i64 %block_end, %reserved_end
  %suffix_span = trunc i64 %suffix64 to i32
  %suffix32 = trunc i64 %reserved_end to i32
  call void @__wosy_wasm_fragment(i32 %suffix32, i32 %suffix_span, i32 %span)
  br label %update_boundary

update_boundary:
  %boundary_prev = phi i32 [ %span, %allocation ], [ %suffix_span, %suffix ]
  %boundary_tail = phi i32 [ %h32, %allocation ], [ %suffix32, %suffix ]
  %has_successor = icmp ult i64 %block_end, %cursor
  br i1 %has_successor, label %update_successor, label %update_tail

update_successor:
  %successor = trunc i64 %block_end to i32
  call void @__wosy_wasm_set_prev(i32 %successor, i32 %boundary_prev)
  ret ptr %payload

update_tail:
  store i32 %boundary_tail, ptr @__wosy_wasm_tail, align 4
  store i64 %block_end, ptr @__wosy_core_heap_cursor
  ret ptr %payload

failure:
  ret ptr null
}

define void @__wosy_core_free(ptr %pointer) {
entry:
  %is_null = icmp eq ptr %pointer, null
  br i1 %is_null, label %complete, label %release

release:
  %back = getelementptr i8, ptr %pointer, i32 -4
  %h = load i32, ptr %back, align 1
  %p = inttoptr i32 %h to ptr
  %packed = load i32, ptr %p, align 4
  %span = and i32 %packed, -8
  %prev_slot = getelementptr i8, ptr %p, i32 4
  %packed_prev = load i32, ptr %prev_slot, align 4
  %prev = and i32 %packed_prev, -8
  %has_prev = icmp ne i32 %prev, 0
  br i1 %has_prev, label %previous, label %try_next

previous:
  %prev_h = sub i32 %h, %prev
  %prev_p = inttoptr i32 %prev_h to ptr
  %prev_word = load i32, ptr %prev_p, align 4
  %prev_free_bit = and i32 %prev_word, 1
  %prev_is_free = icmp ne i32 %prev_free_bit, 0
  br i1 %prev_is_free, label %merge_prev, label %try_next

merge_prev:
  call void @__wosy_wasm_unlink(i32 %prev_h)
  %prev_prev_slot = getelementptr i8, ptr %prev_p, i32 4
  %prev_prev_word = load i32, ptr %prev_prev_slot, align 4
  %prev_prev_span = and i32 %prev_prev_word, -8
  %left_span = add i32 %span, %prev
  br label %try_next

try_next:
  %left = phi i32 [ %h, %release ], [ %h, %previous ], [ %prev_h, %merge_prev ]
  %left_prev = phi i32 [ %prev, %release ], [ %prev, %previous ], [ %prev_prev_span, %merge_prev ]
  %left_size = phi i32 [ %span, %release ], [ %span, %previous ], [ %left_span, %merge_prev ]
  %left64 = zext i32 %left to i64
  %left_size64 = zext i32 %left_size to i64
  %right64 = add i64 %left64, %left_size64
  %cursor = load i64, ptr @__wosy_core_heap_cursor
  %has_right = icmp ult i64 %right64, %cursor
  br i1 %has_right, label %next, label %finalize

next:
  %right = trunc i64 %right64 to i32
  %right_p = inttoptr i32 %right to ptr
  %right_word = load i32, ptr %right_p, align 4
  %right_free_bit = and i32 %right_word, 1
  %right_is_free = icmp ne i32 %right_free_bit, 0
  br i1 %right_is_free, label %merge_next, label %finalize

merge_next:
  call void @__wosy_wasm_unlink(i32 %right)
  %right_span = and i32 %right_word, -8
  %both_span = add i32 %left_size, %right_span
  br label %finalize

finalize:
  %final_span = phi i32 [ %left_size, %try_next ], [ %left_size, %next ], [ %both_span, %merge_next ]
  call void @__wosy_wasm_fragment(i32 %left, i32 %final_span, i32 %left_prev)
  %final_span64 = zext i32 %final_span to i64
  %end = add i64 %left64, %final_span64
  %successor_exists = icmp ult i64 %end, %cursor
  br i1 %successor_exists, label %successor, label %tail

successor:
  %successor_h = trunc i64 %end to i32
  call void @__wosy_wasm_set_prev(i32 %successor_h, i32 %final_span)
  br label %complete

tail:
  store i32 %left, ptr @__wosy_wasm_tail, align 4
  br label %complete

complete:
  ret void
}
