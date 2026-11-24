%%start
struct Packet { *?u8 data; u64 length; }
(Packet, Packet)(u64, u64) pair = fn(left_size, right_size) {
    u8[left_size] left_backing;
    u8[right_size] right_backing;
    left_backing[0] = 19;
    right_backing[0] = 37;
    *?u8 left_data = null;
    *?u8 right_data = null;
    unsafe { left_data = &?left_backing[0]; };
    unsafe { right_data = &?right_backing[0]; };
    Packet first_value = { .data = left_data; .length = left_size; };
    Packet second_value = { .data = right_data; .length = right_size; };
    second_value, first_value
};
u64() drive = fn {
    Packet front, Packet back = pair(2, 5);
    front.length * 100 + back.length
};
if (drive() != 502) { core.system_panic(); };
%%end
