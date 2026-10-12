%%start
struct Dispatch { u8() action; }
u8 seed = 7;
Dispatch global_field = { .action = fn { seed }; };
unit() run = fn {
    u8 first = global_field.action();
    u8 second = global_field.action();
    Dispatch copied = global_field;
    u8 third = copied.action();
    if (first != 7 || second != 7 || third != 7) { core.system_panic(); };
};
run();
%%end
