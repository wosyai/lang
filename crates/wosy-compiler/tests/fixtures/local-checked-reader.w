%%start
u8(*u8) read_plain = fn(reader) { *reader };
u8(*u8) read_bonus = fn(reader) { *reader + 1 };
u8(u64, u8, bool) frame = fn(depth, tag, choose) {
    if (depth == 0) {
        tag
    } else {
        u64 extent = depth + 2;
        u8[extent] bytes;
        bytes[0] = tag;
        bytes[1] = tag + 10;
        u8(*u8) read_slot = read_plain;
        if (choose) { read_slot = read_bonus; };
        *u8 selected = if (choose) { &bytes[0] } else { &bytes[1] };
        u8 nested = frame(depth - 1, tag - 1, !choose);
        u8 observed = read_slot(selected);
        bytes[0] = tag + 20;
        bytes[1] = tag + 30;
        nested + observed + bytes[0]
    }
};
u8(bool) read_joined = fn(choose) {
    u8 first = 7;
    u8 second = 9;
    *!u8 parent = null;
    if (choose) { parent = &!first; } else { parent = &!second; };
    u8(*u8) read_slot = read_plain;
    if (choose) { read_slot = read_bonus; };
    *u8 child = &(*parent);
    u8 observed = read_slot(child);
    first = 11;
    second = 12;
    observed + first + second
};
u8() joined_left = fn { read_joined(true) };
u8() joined_right = fn { read_joined(false) };
u8() frame_zero = fn { frame(0, 0, false) };
u8() frame_two = fn { frame(2, 2, true) };
u8() frame_three = fn { frame(3, 3, false) };
joined_left();
joined_right();
frame_zero();
frame_two();
frame_three();
%%end
