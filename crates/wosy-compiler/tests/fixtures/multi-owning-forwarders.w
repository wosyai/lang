u64(Packet, u64, Packet, Packet) relay_first = fn(front, multiplier, middle, back) {
    relay_second(middle, multiplier, back, front)
};
u64(Packet, u64, Packet, Packet) relay_second = fn(front, multiplier, middle, back) {
    combine(back, multiplier, front, middle)
};
