typedef unsigned long long u64;
extern int wosy_authored_main(void);

_Alignas(8) static unsigned char backing[48];
static unsigned issued, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (issued != released || issued >= 2 || alignment != 8 ||
        size != (issued == 0 ? sizeof backing : 0))
        __builtin_trap();
    ++issued;
    for (unsigned i = 0; i < sizeof backing; ++i)
        backing[i] = 0xA5;
    return backing;
}

void __wosy_core_free(void *address) {
    if (issued != released + 1 || address != backing)
        __builtin_trap();
    for (unsigned i = 0; i < sizeof backing; ++i) {
        if (released == 1 && backing[i] != 0xA5)
            __builtin_trap();
    }
    ++released;
}

void __wosy_core_system_panic(void) { __builtin_trap(); }

int main(void) {
    return wosy_authored_main() || issued != 2 || released != 2;
}
