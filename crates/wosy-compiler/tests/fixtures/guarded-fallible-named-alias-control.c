/* Standalone test allocator for the guarded named-alias fixture.
   The only failure control is returning null from an ordinary allocation. */
typedef unsigned long long u64;

extern u64 wosy_test_shared(u64 input);
extern u64 wosy_test_exclusive(u64 input);

static _Alignas(8) u64 backing;
static u64 final_value;
static unsigned calls, grants, frees, invalid, fail, live;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    ++calls;
    if (size != 8 || alignment != 8 || live) {
        ++invalid;
        return 0;
    }
    if (fail)
        return 0;
    ++grants;
    live = 1;
    return &backing;
}

void __wosy_core_free(void *address) {
    ++frees;
    if (address != &backing || !live) {
        ++invalid;
        return;
    }
    final_value = backing;
    live = 0;
}

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    if (wosy_test_shared(40) != 40 || final_value != 40 ||
        calls != 1 || grants != 1 || frees != 1 || invalid || live)
        return 11;
    if (wosy_test_exclusive(20) != 41 || final_value != 21 ||
        calls != 2 || grants != 2 || frees != 2 || invalid || live)
        return 12;
    fail = 1;
    if (wosy_test_shared(40) != 7 || calls != 3 ||
        grants != 2 || frees != 2 || invalid || live || final_value != 21)
        return 13;
    if (wosy_test_exclusive(20) != 9 || calls != 4 ||
        grants != 2 || frees != 2 || invalid || live || final_value != 21)
        return 14;
    return 0;
}
