%%start
u64(u64) shared = fn(value) {
    u64 held =? value;
    *u64 witness = &held;
    if (witness == null) { 7 } else { *witness }
};
u64(u64) exclusive = fn(value) {
    u64 held =? value;
    *!u64 witness = &!held;
    if (null != witness) {
        u64 before = *witness;
        *witness = before + 1;
        *witness + before
    } else {
        9
    }
};
if (shared(40) != 40 || exclusive(20) != 41) { core.system_panic(); };
%%end
