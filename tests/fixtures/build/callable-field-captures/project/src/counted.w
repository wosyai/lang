%%start
struct copy Captured { u8() first; u8() second; }
Captured(bool) make = fn(pick) {
    if (pick) {
        u64 length = 2;
        u8[length] left;
        left[0] = 4;
        Captured chosen = { .first = fn { left[0] }; .second = fn { left[0] }; };
        chosen
    } else {
        u64 length = 2;
        u8[length] right;
        right[0] = 7;
        Captured chosen = { .first = fn { right[0] }; .second = fn { right[0] }; };
        chosen
    }
};
unit() run = fn {
    Captured first = make(true);
    Captured copied = first;
    Captured second = make(false);
    first.first = second.first;
    if (copied.first() != 4 || first.first() != 7 || second.second() != 7) { core.system_panic(); };
    Captured third = make(true);
    Captured fourth = make(false);
    if (third.second() != 4 || fourth.second() != 7) { core.system_panic(); };
};
run();
%%end
