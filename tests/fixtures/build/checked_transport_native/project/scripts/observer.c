typedef unsigned char byte;
typedef __UINTPTR_TYPE__ address_bits;

extern _Noreturn void exit(int status);

enum phase { INITIAL, REGISTERED, WRITTEN, FINISHED, EXPECT_CHECKED_FAILURE };
enum event { REGISTER, VERIFY, ARM_CHECKED_FAILURE };

static enum phase phase;
static address_bits registered;

void CHECKPOINT(int event, address_bits address) {
    const byte *bytes = (const byte *)address;
    switch (event) {
    case REGISTER:
        if (phase != INITIAL || !address || bytes[0] != 5 || bytes[1] != 91) exit(3);
        registered = address;
        phase = REGISTERED;
        return;
    case VERIFY:
        if (phase != WRITTEN || address != registered || bytes[0] != 173 || bytes[1] != 91) exit(3);
        registered = 0;
        phase = FINISHED;
        return;
    case ARM_CHECKED_FAILURE:
        if (phase != INITIAL || address) exit(3);
        phase = EXPECT_CHECKED_FAILURE;
        return;
    default:
        exit(3);
    }
}

void WRITE_BYTE(address_bits address) {
    if (phase == EXPECT_CHECKED_FAILURE) exit(2);
    if (phase != REGISTERED || address != registered) exit(3);
    *(byte *)address = 173;
    phase = WRITTEN;
}
