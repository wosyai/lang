%%start
u8(u8) stack_call = fn(value) {
    u8 local = value + 1;
    local
};
%%end
