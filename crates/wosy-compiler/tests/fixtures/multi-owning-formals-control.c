typedef unsigned long long u64;
static _Alignas(64) unsigned char pool[4][64];
static unsigned allocated, released;
static const u64 lengths[] = {2, 3, 5, 7};
/* Input source order is [third, first, second]. The callee cleans in
   reverse formal declaration order. The caller alone releases retained. */
static const unsigned expected_release[] = {1, 0, 2, 3};

void *__wosy_core_alloc(u64 size, u64 align) {
    if (allocated == 4 || size != lengths[allocated] ||
        !align || align > 64 || 64 % align)
        __builtin_trap();
    return pool[allocated++];
}

void __wosy_core_free(void *base) {
    if (released == 4 || expected_release[released] >= allocated ||
        base != pool[expected_release[released]] ||
        pool[expected_release[released]][0] != 19)
        __builtin_trap();
    ++released;
}

void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_test_multi_owning_formals(void);

int check(void) {
    return wosy_test_multi_owning_formals() != 7325 ||
        allocated != 4 || released != 4;
}
#ifndef WASI
int main(void) { return check(); }
#endif
