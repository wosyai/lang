typedef unsigned long long u64;
extern int wosy_authored_main(void);

_Alignas(8) static unsigned char backing[32];
static unsigned issued, released;
static const unsigned char values[4] = {101, 102, 202, 203};

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (issued != released || issued >= 4 || size != sizeof backing || alignment != 8)
        __builtin_trap();
    ++issued;
    for (unsigned i = 0; i < sizeof backing; ++i)
        backing[i] = 0;
    return backing;
}

void __wosy_core_free(void *address) {
    if (issued != released + 1 || address != backing)
        __builtin_trap();
    for (unsigned i = 0; i < sizeof backing; ++i) {
        unsigned char expected = i == 16 && released == 3 ? 8 : i == 24 ? values[released] : 0;
        if (backing[i] != expected)
            __builtin_trap();
    }
    ++released;
}

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    return wosy_authored_main() || issued != 4 || released != 4;
}
