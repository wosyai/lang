typedef unsigned long long u64;
extern int wosy_authored_main(void);
static unsigned char first[3], second[5];
static unsigned issued, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (alignment != 1 || issued > 1 || size != (issued == 0 ? 3 : 5))
        __builtin_trap();
    return issued++ == 0 ? first : second;
}

void __wosy_core_free(void *pointer) {
    unsigned bit = pointer == first ? 1 : pointer == second ? 2 : 0;
    if (issued != 2 || !bit || (released & bit) || first[2] != 17 || second[4] != 23)
        __builtin_trap();
    released |= bit;
}

void __wosy_core_system_panic(void) { __builtin_trap(); }

int main(void) { return wosy_authored_main() || issued != 2 || released != 3; }
