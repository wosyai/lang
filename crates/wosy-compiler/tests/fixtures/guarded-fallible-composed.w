%%start
u64(u64) shared = fn(value) {
    u64 held =? value;
    *u64 witness = &held;
    u64 first = if (&held != null) { held } else { 7 };
    if (witness == null) { first } else {
        if (&held != null) { *witness } else { 13 }
    }
};
u64(u64) exclusive = fn(value) {
    u64 held =? value;
    *!u64 witness = &!held;
    u64 before = if (null != witness) { *witness } else { 0 };
    if (witness == null) { 9 } else {
        if (null != witness) {
            *witness = before + 1;
            *witness + before
        } else { 15 }
    }
};
if (shared(40) != 40 || exclusive(20) != 41) { core.system_panic(); };
%%end
