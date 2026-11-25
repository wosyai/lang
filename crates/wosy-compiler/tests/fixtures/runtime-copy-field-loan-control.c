/* Both checked field loans use the second row of the one original allocation. */
typedef unsigned long long u64;
extern int wosy_authored_main(void);

_Alignas(8) static unsigned char backing[32];
static unsigned issued, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (issued++ || size != sizeof backing || alignment != 8)
        __builtin_trap();
    return backing;
}

void __wosy_core_free(void *address) {
    if (released++ || issued != 1 || address != backing)
        __builtin_trap();
    for (unsigned i = 0; i < sizeof backing; ++i) {
        unsigned char expected = i == 16 ? 7 : i == 24 ? 202 : 0;
        if (backing[i] != expected)
            __builtin_trap();
    }
}

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    return wosy_authored_main() || issued != 1 || released != 1;
}
