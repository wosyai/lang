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
Packet(u64) inner_owner = fn(length) {
    Packet original = make_packet(length);
    Packet independent = make_packet(length);
    Packet shipped = first_identity(original);
    independent.length;
    shipped
};
Packet(u64) outer_owner = fn(length) {
    Packet incoming = inner_owner(length);
    Packet outgoing = second_identity(incoming);
    outgoing
};
u64(u64, bool) drive = fn(length, take) {
    Packet received = outer_owner(length);
    Packet trailing = make_packet(length);
    u64 result = received.length + trailing.length;
    if (take) { result = result + accept_packet(received); };
    result
};
u64() retain_witness = fn { drive(2, false) };
u64() transfer_witness = fn { drive(2, true) };
retain_witness();
transfer_witness();
%%end
