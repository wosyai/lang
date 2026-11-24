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
Packet(Packet) first_identity = fn(packet) { packet };
Packet(Packet) second_identity = fn(packet) { packet };
u64(u64, bool) drive = fn(length, take) {
    Packet original = make_packet(length);
    Packet keeper = make_packet(length);
    Packet middle = first_identity(original);
    Packet returned = second_identity(middle);
    Packet trailing = make_packet(length);
    u64 result = returned.length + keeper.length + trailing.length;
    if (take) { result = result + accept_packet(returned); };
    result
};
u64() retain_witness = fn { drive(2, false) };
u64() transfer_witness = fn { drive(2, true) };
retain_witness();
transfer_witness();
%%end
