/* Ordinary successful allocations only. Match every released address against
   the original backing and verify exact allocation and cleanup counts. */
typedef unsigned long long u64;
extern int wosy_authored_main(void);

static unsigned char backing[3][9];
static const u64 extents[3] = {5, 9, 7};
static unsigned issued, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (issued >= 3 || issued != released || alignment != 1 ||
        size != extents[issued])
        __builtin_trap();
    return backing[issued++];
}

void __wosy_core_free(void *address) {
    if (released >= issued || address != backing[released])
        __builtin_trap();
    ++released;
}

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    return wosy_authored_main() || issued != 3 || released != 3;
}
