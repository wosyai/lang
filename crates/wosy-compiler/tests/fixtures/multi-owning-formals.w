u64(Packet, u64, Packet, Packet) combine = fn(first, multiplier, second, third) {
    first.length + second.length * multiplier + third.length * 100
};
u64() drive = fn {
    Packet first = make_packet(2);
    Packet second = make_packet(3);
    Packet third = make_packet(5);
    Packet retained = make_packet(7);
    u64 combined = combine(third, 10, first, second);
    combined + retained.length * 1000
};
drive();
%%end
