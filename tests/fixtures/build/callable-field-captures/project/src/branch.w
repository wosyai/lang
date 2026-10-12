%%start
struct copy Captured { u8(u8) write; u8(u64) read; }
Captured(u64) make = fn(count) {
    u8[count] slots;
    slots[0] = 6;
    Captured result = {
        .write = fn(value) { slots[0] = value; slots[0] };
        .read = fn(index) { slots[index] };
    };
    result
};
unit() run = fn {
    Captured first = make(2);
    Captured second = first;
    u64 index = 0;
    while (index < 2) {
        Captured next = make(2);
        if (index == 0) { first = next; } else { second = next; };
        index = index + 1;
    }
    if (false) { Captured skipped = make(2); skipped.read(0); };
    if (true) { Captured taken = make(2); if (taken.write(8) != 8) { core.system_panic(); }; };
    Captured kept = first;
    first.read = fn(position) { position; 8 };
    if (kept.read(0) != 6) {
        core.system_panic();
    };
    if (kept.write(9) != 9) { core.system_panic(); };
    if (kept.read(0) != 9) { core.system_panic(); };
    if (second.read(0) != 6) { core.system_panic(); };
};
run();
%%end
