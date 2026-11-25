%%start
struct Packet { *?u8 data; u64 length; }
Packet(u64) make_packet = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    packet
};
u64(Packet) accept_packet = fn(incoming) { incoming.length + 1 };
u64(u64, bool, bool) drive = fn(length, take_first, keep_second) {
    Packet first = make_packet(length);
    Packet preserved = make_packet(length);
    u64 before = preserved.length;
    if (take_first) {
        before = accept_packet(first);
    };
    Packet second = make_packet(length);
    Packet exhaustive = make_packet(length);
    u64 after = if (keep_second) {
        accept_packet(exhaustive)
    } else {
        u64 second_value = accept_packet(second);
        u64 final_value = accept_packet(exhaustive);
        second_value + final_value
    };
    before + after
};
u64() case_ff = fn { drive(2, false, false) };
u64() case_ft = fn { drive(2, false, true) };
u64() case_tf = fn { drive(2, true, false) };
u64() case_tt = fn { drive(2, true, true) };
case_ff();
case_ft();
case_tf();
case_tt();
%%end
