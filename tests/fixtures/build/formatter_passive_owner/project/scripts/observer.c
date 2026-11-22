typedef __UINTPTR_TYPE__ word;
typedef unsigned char byte;
typedef unsigned int u32;
typedef unsigned long long u64;

extern void *__real___wosy_core_alloc(u64 size, u64 alignment);
extern void __real___wosy_core_free(void *address);

struct iovec { word data; u32 length; };

#ifdef __wasm__
__attribute__((import_module("wasi_snapshot_preview1"), import_name("proc_exit")))
#endif
extern _Noreturn void exit(int code);

#ifdef __wasm__
__attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")))
extern int host_write(int fd, const struct iovec *vectors, int count, u32 *written);
#elif defined(_WIN32)
extern int _write(int fd, const void *data, unsigned int length);
#else
extern __PTRDIFF_TYPE__ write(int fd, const void *data, __SIZE_TYPE__ length);
#endif

#define require(condition) do { if (!(condition)) exit(1 + ((__LINE__ - 1) % 255)); } while (0)

static int stage = -1;
static u32 allocations, releases;
static word bases[2];
static unsigned live;
static int wrote;

static void check_bytes(int slot, word address, u64 length, const char *expected, u32 size) {
    require((live & (1u << slot)) && address == bases[slot] && length == size);
    const byte *data = (const byte *)address;
    for (u32 i = 0; i != size; ++i) require(data[i] == (byte)expected[i]);
}

static void forward(int fd, const struct iovec *vector, u32 *written) {
#ifdef __wasm__
    require(host_write(fd, vector, 1, written) == 0);
#else
#ifdef _WIN32
    int received = _write(fd, (const void *)vector->data, vector->length);
#else
    __PTRDIFF_TYPE__ received = write(fd, (const void *)vector->data, vector->length);
#endif
    require(received >= 0 && (__SIZE_TYPE__)received <= vector->length);
    *written = (u32)received;
#endif
    require(*written == vector->length);
}

static void announce_complete(void) {
    static const char text[] = "formatter owners verified";
    const struct iovec vector = {(word)text, sizeof(text) - 1};
    u32 written = 0;
    forward(2, &vector, &written);
}

void *__wrap___wosy_core_alloc(u64 size, u64 alignment) {
    require(size == 3 && alignment == 1);
    require(((stage == 0 || stage == 10) && allocations == 0 && live == 0)
            || (stage == 11 && allocations == 1 && live == 1));
    word address = (word)__real___wosy_core_alloc(size, alignment);
    require(address && (allocations == 0 || address != bases[0]));
    bases[allocations] = address;
    live |= 1u << allocations;
    ++allocations;
    return (void *)address;
}

void __wrap___wosy_core_free(void *address) {
    int slot;
    if (stage == 0) {
        require(wrote && allocations == 1 && releases == 0 && live == 1);
        slot = 0;
    } else if (stage == 13) {
        require(allocations == 2 && releases == 0 && live == 3);
        slot = 1;
    } else {
        require(stage == 15 && allocations == 2 && releases == 1 && live == 1);
        slot = 0;
    }
    require((word)address == bases[slot]);
    __real___wosy_core_free(address);
    live &= ~(1u << slot);
    ++releases;
}

int FD_WRITE(int fd, word iovec, int vectors, word count) {
    require(stage == 0 && !wrote && allocations == 1 && releases == 0 && live == 1);
    require(fd == 1 && iovec && vectors == 1 && count && *(u32 *)count == 0);
    const struct iovec *vector = (const struct iovec *)iovec;
    check_bytes(0, vector->data, vector->length, "203", 3);
    forward(fd, vector, (u32 *)count);
    wrote = 1;
    return 0;
}

void CHECKPOINT(int mark) {
    switch (mark) {
    case 0: case 10:
        require(stage == -1 && !live && !allocations && !releases && !wrote);
        break;
    case 1: case 2: case 3:
        require(stage == mark - 1 && wrote && allocations == 1 && releases == 1 && !live);
        if (mark == 3) announce_complete();
        break;
    case 11:
        require(stage == 10 && allocations == 1 && releases == 0 && live == 1);
        check_bytes(0, bases[0], 2, "17", 2);
        break;
    case 12:
        require(stage == 11 && allocations == 2 && releases == 0 && live == 3);
        check_bytes(1, bases[1], 3, "245", 3);
        break;
    case 13:
        require(stage == 12 && allocations == 2 && releases == 0 && live == 3);
        break;
    case 14:
        require(stage == 13 && allocations == 2 && releases == 1 && live == 1);
        check_bytes(0, bases[0], 2, "17", 2);
        break;
    case 15:
        require(stage == 14 && allocations == 2 && releases == 1 && live == 1);
        break;
    case 16: case 17:
        require(stage == mark - 1 && allocations == 2 && releases == 2 && !live && !wrote);
        if (mark == 17) announce_complete();
        break;
    default:
        require(0);
    }
    stage = mark;
}

int FD_READ(int fd, word vectors, int count, word read) {
    (void)fd; (void)vectors; (void)count; (void)read;
    require(0);
    return 0;
}

int POLL_ONEOFF(word in, word out, int count, word result) {
    (void)in; (void)out; (void)count; (void)result;
    require(0);
    return 0;
}

void PROC_EXIT(int code) {
    (void)code;
    require(0);
}
