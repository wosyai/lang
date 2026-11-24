%%start
u8(*u8) read_selected = fn(reader) { *reader };
u8(u64, u8, bool) frame = fn(depth, tag, choose) {
    if (depth == 0) {
        tag
    } else {
        u64 extent = depth + 2;
        u8[extent] bytes;
        bytes[0] = tag;
        bytes[1] = tag + 10;
        *u8 selected = if (choose) { &bytes[0] } else { &bytes[1] };
        u8 nested = frame(depth - 1, tag - 1, !choose);
        nested + read_selected(selected)
    }
};
(u8[], u64)(u8[], u64, u64) carry = fn(bytes, depth, count) {
    if (depth != 0) {
        u64 one = 1;
        bytes, count = carry(bytes, depth - one, count);
    };
    bytes, count
};
u8(u64, u8) carry_witness = fn(depth, tag) {
    u64 size = 2;
    u8[size] bytes;
    bytes[0] = tag;
    u64 count = 1;
    bytes, count = carry(bytes, depth, count);
    if (count == 1) { bytes[0] } else { 0 }
};
u8() frame_zero = fn { frame(0, 0, false) };
u8() frame_two = fn { frame(2, 2, true) };
u8() frame_three = fn { frame(3, 3, false) };
u8() carry_zero = fn { carry_witness(0, 30) };
u8() carry_two = fn { carry_witness(2, 40) };
u8() carry_three = fn { carry_witness(3, 50) };
frame_zero();
frame_two();
frame_three();
carry_zero();
carry_two();
carry_three();
%%end
