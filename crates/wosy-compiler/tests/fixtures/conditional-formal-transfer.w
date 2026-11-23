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
u64(Packet, bool) relay = fn(packet, take) {
    if (take) { accept_packet(packet) } else { packet.length }
};
u64(u64, bool) frame = fn(depth, take) {
    if (depth == 0) {
        0
    } else {
        u64 length = depth + 1;
        Packet packet = make_packet(length);
        u64 nested = frame(depth - 1, !take);
        u64 observed = relay(packet, take);
        nested * 10 + observed
    }
};
u64() case_zero = fn { frame(0, true) };
u64() case_two = fn { frame(2, false) };
u64() case_three = fn { frame(3, true) };
case_zero();
case_two();
case_three();
%%end
