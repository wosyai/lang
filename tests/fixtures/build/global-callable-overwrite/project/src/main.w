%%start
struct Dispatch { u8() action; }
u8 old_seed = 3;
Dispatch global_field = { .action = fn { old_seed }; };
unit() invoke = fn {
    u8 marker = 4;
    Dispatch saved = global_field;
    global_field.action = fn { marker };
    u8 old = saved.action();
    u8 current = global_field.action();
    if (old != 3 || current != 4) {
        core.system_panic();
    };
};
invoke();
%%end
