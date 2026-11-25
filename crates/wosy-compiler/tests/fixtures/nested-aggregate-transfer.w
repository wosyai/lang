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
u64(u64, bool, bool) drive = fn(length, outer, inner) {
    Packet first = make_packet(length);
    Packet preserved = make_packet(length);
    Packet exhaustive = make_packet(length);
    Packet right = make_packet(length);
    Packet shared = make_packet(length);
    if (outer) {
        if (inner) {
            u64 left_value = accept_packet(first);
            u64 common_value = accept_packet(exhaustive);
            left_value + common_value
        } else {
            u64 common_value = accept_packet(exhaustive);
            u64 shared_value = accept_packet(shared);
            common_value + shared_value
        }
    } else {
        if (inner) {
            u64 right_value = accept_packet(right);
            u64 shared_value = accept_packet(shared);
            right_value + shared_value
        } else {
            preserved.length
        }
    }
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
