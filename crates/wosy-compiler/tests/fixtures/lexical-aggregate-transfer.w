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
u64(u64, bool, bool) drive = fn(length, left, take) {
    Packet parent = make_packet(length);
    u64 block_value = unsafe {
        Packet plain = make_packet(length);
        if (take) { accept_packet(plain) } else { plain.length }
    };
    u64 branch_value = if (left) {
        Packet child = make_packet(length);
        Packet kept = make_packet(length);
        u64 child_value = if (take) { accept_packet(child) } else { child.length };
        u64 parent_value = accept_packet(parent);
        child_value + parent_value + kept.length
    } else {
        Packet child = make_packet(length);
        Packet kept = make_packet(length);
        u64 child_value = if (take) { accept_packet(child) } else { accept_packet(child) };
        child_value + kept.length
    };
    block_value + branch_value
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
