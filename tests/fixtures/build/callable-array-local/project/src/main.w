%%start
unit(u64) inspect = fn(count) {
    (u8())[count] items;
    u8 first = 1;
    u8 second = 2;
    items[0] = fn { first };
    items[1] = fn { second };
    u8() kept = items[0];
    items[0] = fn { first };
    items[0] = fn { first };
    u8() sibling = items[1];
    u8 a = kept();
    u8 b = sibling();
    if (a != 1 || b != 2) { core.system_panic(); };
    items;
};
inspect(2);
%%end
