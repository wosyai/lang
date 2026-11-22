typedef unsigned long long u64;
typedef unsigned char byte;

extern byte *__real___wosy_core_alloc(u64 size, u64 alignment);
extern void __real___wosy_core_free(byte *address);
extern _Noreturn void __wosy_core_system_panic(void);

enum checkpoint { ENTER, USE, UNWIND, OPEN, CLOSE, COMPLETE };
enum phase { ALLOCATED, USED, UNWINDING };
struct frame { byte *scratch; u64 depth; enum phase phase; };

static struct frame frames[4];
static unsigned active, allocated, released;
static int enter_pending, closed;
static u64 pending_depth;
static const byte *backing;

static void require(int condition) {
    if (!condition) __wosy_core_system_panic();
}

#if defined(__wasm__)
struct output_iovec { const char *data; unsigned length; };
__attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")))
extern int proof_fd_write(unsigned fd, const struct output_iovec *iovecs, unsigned count, unsigned *written);
#else
extern int puts(const char *text);
#endif

static void report_completion(void) {
#if defined(__wasm__)
    static const char marker[] = "recursive frame release verified\n";
    const struct output_iovec iovec = { marker, sizeof(marker) - 1 };
    unsigned written = 0;
    require(proof_fd_write(1, &iovec, 1, &written) == 0 && written == sizeof(marker) - 1);
#else
    require(puts("recursive frame release verified") >= 0);
#endif
}

byte *__wrap___wosy_core_alloc(u64 size, u64 alignment) {
    require(enter_pending && active < 4 && size == 4 && alignment == 1);
    byte *scratch = __real___wosy_core_alloc(size, alignment);
    require(scratch && scratch != backing);
    for (unsigned i = 0; i < active; ++i) require(frames[i].scratch != scratch);
    frames[active++] = (struct frame){ scratch, pending_depth, ALLOCATED };
    ++allocated;
    enter_pending = 0;
    return scratch;
}

void __wrap___wosy_core_free(byte *address) {
    require(active && !enter_pending);
    struct frame *most_recent = &frames[active - 1];
    require(most_recent->phase == UNWINDING && address == most_recent->scratch);
    require(most_recent->depth == released);
    require(address && address[0] == 16 + most_recent->depth && address[3] == 173);
    __real___wosy_core_free(address);
    most_recent->scratch = 0;
    --active;
    ++released;
}

void CHECKPOINT(int event, u64 depth, __UINTPTR_TYPE__ address_bits) {
    const byte *address = (const byte *)address_bits;
    switch (event) {
    case OPEN:
        require(!backing && !closed && !active && !allocated && depth == 3);
        require(address && *address == 11);
        backing = address;
        return;
    case ENTER:
        require(backing && *backing == 11 && !enter_pending && !address);
        require(active < 4 && depth == 3 - active && allocated == active && !released);
        if (active) require(frames[active - 1].phase == USED);
        pending_depth = depth;
        enter_pending = 1;
        return;
    case USE:
    case UNWIND: {
        require(backing && *backing == 11 && active && !enter_pending);
        struct frame *frame = &frames[active - 1];
        require(frame->depth == depth && address == frame->scratch && address);
        require(address[0] == 16 + depth && address[3] == 173);
        if (event == USE) {
            require(frame->phase == ALLOCATED && !released);
            frame->phase = USED;
        } else {
            require(frame->phase == USED && allocated == 4 && released == depth);
            frame->phase = UNWINDING;
        }
        return;
    }
    case CLOSE:
        require(backing && !closed && depth == 3 && address == backing && *address == 11);
        require(!active && !enter_pending && allocated == 4 && released == 4);
        backing = 0;
        closed = 1;
        return;
    case COMPLETE:
        require(closed && !backing && !address && depth == 3 && !active && released == 4);
        report_completion();
        return;
    default:
        __wosy_core_system_panic();
    }
}
