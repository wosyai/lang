typedef unsigned long long u64;
extern int wosy_authored_main(void);

static u64 backing[3];
static unsigned issued, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (issued != released || issued >= 2 || alignment != 8 ||
        size != (issued == 0 ? sizeof backing : 0))
        __builtin_trap();
    ++issued;
    backing[0] = backing[1] = backing[2] = 200;
    return backing;
}

void __wosy_core_free(void *address) {
    if (issued != released + 1 || address != backing)
        __builtin_trap();
    for (unsigned i = 0; i < 3; ++i) {
        if (backing[i] != (released == 0 ? 10 + i : 200))
            __builtin_trap();
    }
    ++released;
}

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    return wosy_authored_main() || issued != 2 || released != 2;
}
