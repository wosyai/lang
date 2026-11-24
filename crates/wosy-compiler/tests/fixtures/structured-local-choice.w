%%start
struct Packet { *?u8 data; u64 length; }
Packet(u64) make_short = fn(length) {
    u8[length] backing;
    backing[0] = 19;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length; };
    packet
};
Packet(u64) make_long = fn(length) {
    u8[length] backing;
    backing[0] = 29;
    *?u8 data = null;
    unsafe { data = &?backing[0]; };
    Packet packet = { .data = data; .length = length + 10; };
    packet
};
u64(u64, bool) drive = fn(depth, choose) {
    Packet chosen = if (choose != (depth == 3)) { make_short(2) } else { make_long(2) };
    chosen.length
};
u64() case_zero = fn { drive(0, false) };
u64() case_two = fn { drive(2, true) };
u64() case_three = fn { drive(3, true) };
case_zero();
case_two();
case_three();
%%end
