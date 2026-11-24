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
Packet(u64) make_alt = fn(length) {
    u8[length] backing;
    backing[0] = 29;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    packet
};
Packet(u64, u64, bool) factory = fn(depth, length, choose) {
    if (depth == 0) {
        if (choose) { make_packet(length) } else { make_alt(length) }
    } else {
        u64 extent = length + depth;
        Packet frame = make_packet(extent);
        frame.length;
        factory(depth - 1, length, !choose)
    }
};
u64(u64, bool) drive = fn(depth, choose) {
    Packet received = factory(depth, 2, choose);
    received.length
};
u64() case_zero = fn { drive(0, false) };
u64() case_two = fn { drive(2, true) };
u64() case_three = fn { drive(3, true) };
case_zero();
case_two();
case_three();
%%end
