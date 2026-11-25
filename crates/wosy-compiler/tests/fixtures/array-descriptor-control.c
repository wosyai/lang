typedef unsigned long long u64;
extern int wosy_authored_main(void);

static unsigned char original[2], replacement[7] = {[3] = 37};
static unsigned issued, released;

void *__wosy_core_alloc(u64 size, u64 alignment) {
    if (alignment != 1)
        __builtin_trap();
    if (issued == 0 && released == 0 && size == 2) {
        ++issued;
        return original;
    }
    if (issued == 1 && released == 0 && size == 7) {
        ++issued;
        return replacement;
    }
    __builtin_trap();
}

void __wosy_core_free(void *address) {
    if (issued != 2 ||
        !((released == 0 && address == original) ||
          (released == 1 && address == replacement)))
        __builtin_trap();
    ++released;
}

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    return wosy_authored_main() || issued != 2 || released != 2;
}
