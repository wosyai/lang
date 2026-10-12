%%start
struct copy Pair {
    u8(u64) read;
    u8(u64) sibling;
    u8() copied;
}

Pair(u64) make = fn(count) {
    (u8())[count] items;
    u8 first = 3;
    u8 second = 7;
    items[0] = fn { first };
    items[1] = fn { second };
    u8() saved = items[0];
    Pair result = {
        .read = fn(index) { u8() copy = items[index]; copy() };
        .sibling = fn(index) { u8() copy = items[index]; copy() };
        .copied = saved;
    };
    (u8())[3] incoming;
    incoming[0] = fn { first };
    incoming[1] = fn { second };
    incoming[2] = fn { first + second };
    items = incoming;
    result
};

unit() inspect = fn {
    Pair result = make(2);
    if (result.copied() != 3 || result.read(0) != 3 || result.sibling(1) != 7 || result.sibling(2) != 10) {
        core.system_panic();
    };
    u8() outside = result.copied;
    result.copied = fn { 0 };
    result.read = fn(index) { index; 0 };
    result.sibling = fn(index) { index; 0 };
    if (outside() != 3) { core.system_panic(); };
};
inspect();
%%end
