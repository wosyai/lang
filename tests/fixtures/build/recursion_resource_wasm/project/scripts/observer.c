typedef unsigned long long u64;
typedef unsigned char byte;

extern byte *__real___wosy_core_alloc(u64 size, u64 alignment);
extern void __real___wosy_core_free(byte *address);
extern _Noreturn void __wosy_core_system_panic(void);

enum phase {
    EXPECT_WRAPPER, BETWEEN_STEPS, EXPECT_SCRATCH, SCRATCH_ALLOCATED,
    SCRATCH_USED, SCRATCH_RELEASED, BASE_REACHED, FINISHED
};
enum checkpoint { ENTER_STEP, USE_SCRATCH, LEAVE_STEP, BASE, AFTER_WRAPPER, OPEN, CLOSE };

struct allocation {
    byte *address;
    u64 generation;
};

static enum phase phase;
static const byte *backing;
static struct allocation scratch;
static u64 generation, released_generation, completed;
static const byte expected_backing[3][3] = {{19, 7, 91}, {19, 20, 91}, {21, 20, 91}};

static void require(int condition) {
    if (!condition) __wosy_core_system_panic();
}

byte *__wrap___wosy_core_alloc(u64 size, u64 alignment) {
    require(alignment == 1 && size == 4);
    require(phase == EXPECT_SCRATCH && !scratch.address);
    byte *address = __real___wosy_core_alloc(size, alignment);
    require(address && address != backing);
    scratch.address = address;
    scratch.generation = ++generation;
    phase = SCRATCH_ALLOCATED;
    return address;
}

void __wrap___wosy_core_free(byte *address) {
    require(phase == SCRATCH_USED && address && address == scratch.address);
    require(scratch.generation == completed + 1);
    __real___wosy_core_free(address);
    released_generation = scratch.generation;
    scratch.address = 0;
    phase = SCRATCH_RELEASED;
}

void CHECKPOINT(int event, u64 iteration, __UINTPTR_TYPE__ address_bits) {
    const byte *address = (const byte *)address_bits;
    require(iteration == completed);
    if (event == OPEN) {
        require(phase == EXPECT_WRAPPER && address);
        require(address[0] == 5 && address[1] == 7 && address[2] == 91);
        backing = address;
        phase = BETWEEN_STEPS;
        return;
    }
    if (event == AFTER_WRAPPER) {
        require(phase == FINISHED && completed == 3 && generation == 3);
        require(!backing && !scratch.address && released_generation == 3);
        return;
    }
    require(backing != 0);
    switch (event) {
    case ENTER_STEP:
        require(phase == BETWEEN_STEPS && completed < 3 && !scratch.address);
        phase = EXPECT_SCRATCH;
        break;
    case USE_SCRATCH:
        require(phase == SCRATCH_ALLOCATED && address && address == scratch.address);
        require(scratch.generation == completed + 1);
        require(address[0] == 19 + completed && address[3] == 173);
        phase = SCRATCH_USED;
        break;
    case LEAVE_STEP: {
        require(phase == SCRATCH_RELEASED && !scratch.address && completed < 3);
        require(released_generation == completed + 1);
        const byte *expected = expected_backing[completed++];
        require(backing[0] == expected[0] && backing[1] == expected[1] && backing[2] == expected[2]);
        phase = BETWEEN_STEPS;
        break;
    }
    case BASE:
        require(phase == BETWEEN_STEPS && completed == 3 && !scratch.address);
        phase = BASE_REACHED;
        break;
    case CLOSE:
        require(phase == BASE_REACHED && address == backing && !scratch.address);
        backing = 0;
        phase = FINISHED;
        break;
    default:
        __wosy_core_system_panic();
    }
}
