%%start
struct copy Captured {
    u8(u8) write;
    u8(u64) read;
}

Captured(u64) make = fn(count) {
    u8[count] slots;
    slots[0] = 6;
    Captured result = {
        .write = fn(value) { slots[0] = value; slots[0] };
        .read = fn(index) { slots[index] };
    };
    u64 longer = count + 1;
    u8[longer] replacement;
    replacement[0] = 7;
    replacement[2] = 6;
    slots = replacement;
    result
};

unit() run = fn {
    Captured first = make(2);
    Captured kept = first;
    first.read = fn(index) { index; 8 };
    if (kept.read(2) != 6 || kept.read(0) != 7 || kept.write(9) != 9 || kept.read(0) != 9) {
        core.system_panic();
    };
    Captured separate = make(2);
    if (separate.write(4) != 4 || separate.read(0) != 4 || separate.read(2) != 6 || kept.read(0) != 9) {
        core.system_panic();
    };
};

run();
%%end
