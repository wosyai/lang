%%start
struct Holder { *u8 head; *u8 sibling; }
u8(*u8) read_selected = fn(reader) { *reader };
u8(u64, u8) frame = fn(depth, tag) {
    if (depth == 0) {
        tag
    } else {
        u64 extent = depth + 2;
        u8[extent] bytes;
        bytes[0] = tag;
        bytes[1] = tag + 1;
        bytes[2] = tag + 2;
        Holder held = { .head = null; .sibling = null; };
        held.head = &bytes[0];
        held.sibling = &bytes[1];
        held.head = &bytes[2];
        bytes[0] = tag + 3;
        u8 nested = frame(depth - 1, tag - 1);
        u8 head_value = read_selected(held.head);
        u8 sibling_value = read_selected(held.sibling);
        bytes[2] = tag + 4;
        nested + head_value * 2 + sibling_value + bytes[0] + bytes[2]
    }
};
u8() frame_zero = fn { frame(0, 0) };
u8() frame_two = fn { frame(2, 2) };
u8() frame_three = fn { frame(3, 3) };
frame_zero();
frame_two();
frame_three();
%%end
