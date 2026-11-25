%%start
struct copy Record { u8 tag; i32 amount; }
i32(i32) observed = fn(amount) {
    Record held =? { .tag = 11; .amount = amount; };
    if (&held != null) {
        Record before = held;
        Record replacement = { .tag = 19; .amount = 700; };
        held = replacement;
        Record after = held;
        core.int_extend<i32>(before.tag) + before.amount
            + core.int_extend<i32>(after.tag) + after.amount
    } else {
        7
    }
};
if (observed(100) != 830) { core.system_panic(); };
%%end
