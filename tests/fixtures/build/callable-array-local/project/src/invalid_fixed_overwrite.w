%%start
unit(u64) inspect = fn(count) {
    u8 first = 3;
    u8 second = 7;
    (u8())[2] fixed;
    fixed[0] = fn { first };
    fixed[1] = fn { second };
    *(u8()) borrowed = &fixed[0];
    fixed[0] = fn { second };
    borrowed;
    count;
};
inspect(2);
%%end
