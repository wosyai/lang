typedef unsigned long long u64;
static _Alignas(8) unsigned char storage[2][8];
static unsigned allocations, releases;
static const u64 sizes[] = {2, 5};
static const unsigned char written[] = {19, 37};

void *__wosy_core_alloc(u64 size, u64 align) {
    if (allocations == 2 || size != sizes[allocations] || align != 1)
        __builtin_trap();
    return storage[allocations++];
}
void __wosy_core_free(void *base) {
    /* Callee returns reversed lanes; caller releases reverse lexical binding order. */
    if (releases == 2 || releases >= allocations ||
        base != storage[releases] || storage[releases][0] != written[releases])
        __builtin_trap();
    ++releases;
}
void __wosy_core_system_panic(void) { __builtin_trap(); }
extern u64 wosy_test_multi_returned_owners(void);

int check(void) {
    return wosy_test_multi_returned_owners() != 502 || allocations != 2 || releases != 2;
}
#ifndef WASI
int main(void) { return check(); }
#endif
