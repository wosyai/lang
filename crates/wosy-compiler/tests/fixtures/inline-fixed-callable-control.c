extern int wosy_authored_main(void);

void __wosy_core_system_panic(void) {
    __builtin_trap();
}

int main(void) {
    return wosy_authored_main();
}
