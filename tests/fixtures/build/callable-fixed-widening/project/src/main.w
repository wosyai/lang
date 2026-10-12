%%start
unit() inspect = fn {
    u8 first = 11;
    u8 second = 22;
    (u8())[2] fixed = [fn { first }, fn { second }];
    (u8())[] copied = fixed;
    u8() from_source = fixed[1];
    fixed[0] = fn { first };
    u8() from_copy = copied[0];
    u8 a = from_copy();
    u8 b = from_source();
    if (a != 11 || b != 22) { core.system_panic(); };
    copied;
};
inspect();
%%end
