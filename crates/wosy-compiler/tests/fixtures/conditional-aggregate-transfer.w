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
u64(u64, bool) drive = fn(length, take) {
    Packet packet = make_packet(length);
    u64 result = packet.length;
    if (take) { result = accept_packet(packet); };
    result
};
u64() retain_witness = fn { drive(2, false) };
u64() transfer_witness = fn { drive(2, true) };
retain_witness();
transfer_witness();
%%end
