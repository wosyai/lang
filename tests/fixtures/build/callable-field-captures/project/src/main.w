%%start
std = namespace std "bootstrap.w";

struct Counter {
    i32(i32) advance;
}

struct copy Captured {
    u8(u8) write;
    u8(u64) read;
}

Counter(i32) make_counter = fn(start) {
    i32 total = start;
    Counter counter = { .advance = fn(delta) {
        total = total + delta;
        total
    }; };
    total = total + 2;
    counter
};

Captured(u64) make_captured = fn(count) {
    u8[count] slots;
    slots[0] = 6;
    slots[1] = 7;
    Captured result = {
        .write = fn(value) { slots[0] = value; slots[0] };
        .read = fn(index) { slots[index] };
    };
    u64 longer = count + 1;
    u8[longer] next;
    next[0] = 4;
    next[1] = 5;
    next[2] = 6;
    slots = next;
    result
};

unit() run = fn {
    Counter counter = make_counter(40);
    i32 first = counter.advance(1);
    i32 second = counter.advance(3);
    if (first != 43 || second != 46) { core.system_panic(); };
    std.print("callable field captures: 43 46\n");

    Captured kept = make_captured(2);
    Captured copied = kept;
    u8 before = kept.read(0);
    u8 written = kept.write(9);
    u8 shared = copied.read(0);
    u64 length = 2;
    u8[length] replacement;
    replacement[0] = 5;
    replacement[1] = 8;
    kept.read = fn(index) { replacement[index] };
    u8 old_after_overwrite = copied.read(0);
    u8 new_after_overwrite = kept.read(1);
    if (before != 4 || written != 9 || shared != 9 || old_after_overwrite != 9 || new_after_overwrite != 8) {
        core.system_panic();
    };
    std.print("captured array field overwrite: 4 9 9 9 8\n");
    Captured separate = make_captured(2);
    u8 second_write = separate.write(4);
    u8 second_read = separate.read(0);
    u8 expanded_read = separate.read(2);
    if (second_write != 4 || second_read != 4 || expanded_read != 6 || copied.read(0) != 9) {
        core.system_panic();
    };
    std.print("independent captured array: 4 4 9\n");
};

run();
%%end
