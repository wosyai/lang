%%start
u64(u64) frame = fn(depth) {
    if (depth == 0) {
        0
    } else {
        u64 value =? depth;
        if (&value == null) {
            50
        } else {
            u64 child = frame(depth - 1);
            child * 10 + value
        }
    }
};
u64() no_allocation = fn { frame(0) };
u64() three_frames = fn { frame(3) };
no_allocation();
three_frames();
%%end
