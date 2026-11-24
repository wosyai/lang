%%start
u8(*u8) read_selected = fn(reader) { *reader };
u8(u64) descend = fn(depth) {
    if (depth == 0) { 0 } else { descend(depth - 1) + 1 }
};
(u8[], u64)(u8[], u64, u64) carry = fn(bytes, depth, count) {
    if (depth != 0) {
        u64 one = 1;
        bytes, count = carry(bytes, depth - one, count);
    };
    bytes, count
};
u8(u64, bool) drive = fn(depth, choose) {
    u64 length = 2;
    u8[length] left;
    u8[length] right;
    left[0] = 19;
    right[0] = 37;
    *u8 holder = if (choose) { &left[0] } else { &right[0] };
    u8 nested = descend(depth);
    u8 observed = read_selected(holder);
    u64 left_count = 1;
    u64 right_count = 2;
    left, left_count = carry(left, depth, left_count);
    right, right_count = carry(right, depth, right_count);
    if (left_count == 1 && right_count == 2) {
        observed + nested + left[0] * 2 + right[0] * 3
    } else {
        0
    }
};
u8() choose_left = fn { drive(3, true) };
u8() choose_right = fn { drive(0, false) };
choose_left();
choose_right();
%%end
