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
u64(*u64) read_length = fn(view) { *view + 1 };
u64(u64) drive = fn(length) {
    Packet packet = make_packet(length);
    *u64 view = &packet.length;
    read_length(view)
};
u64() field_witness = fn { drive(3) };
field_witness();
%%end
