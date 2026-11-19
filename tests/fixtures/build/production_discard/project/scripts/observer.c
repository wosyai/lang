typedef __UINTPTR_TYPE__ word;
typedef unsigned char byte;
typedef unsigned int u32;
typedef unsigned long long u64;

extern _Noreturn void __wosy_core_system_panic(void);
extern void *__real___wosy_core_alloc(u64 size, u64 alignment);
extern void __real___wosy_core_free(void *address);

struct iovec { word destination; u32 length; };
struct read_site { word destination, iovec, count, stack; };

enum phase { IDLE, DISCARD, RECOVERY, RELEASING, DONE };
static enum phase phase;
static u32 which, next_case, reads, recovery_reads, allocations, releases;
static struct read_site public_site, discard_site, repeat_site, recovery_public_site, recovery_tail_site;
static void *backing, *record;

#ifdef __wasm__
__attribute__((import_module("wasi_snapshot_preview1"), import_name("proc_exit")))
#endif
extern _Noreturn void exit(int code);
#define require(condition) do { if (!(condition)) exit(__LINE__); } while (0)

static u32 final_read(void) {
    return which == 0 ? 1000001 : 3;
}

static void equal_site(struct read_site first, struct read_site current) {
    require(first.destination == current.destination && first.iovec == current.iovec
            && first.count == current.count && first.stack == current.stack);
}

static void separate_site(struct read_site first, struct read_site current) {
    require(first.destination != current.destination);
}

int FD_READ(int descriptor, word address, int vectors, word count) {
    byte frame;
    require(descriptor == 0 && vectors == 1 && address && count);
    const struct iovec *buffer = (const struct iovec *)address;
    require(buffer->length == 1 && buffer->destination);
    require(*(u32 *)count == 0);
    struct read_site current = {buffer->destination, address, count, (word)&frame};
    byte *destination = (byte *)current.destination;
    u32 *reported = (u32 *)count;
    if (phase == DISCARD) {
        require(!allocations && !releases && reads <= final_read());
        u32 index = reads++;
        if (index == 0) public_site = current;
        else if (index == 1) {
            separate_site(public_site, current);
            discard_site = current;
        } else {
            require(current.destination == discard_site.destination);
            if (index == 2) repeat_site = current;
            else equal_site(repeat_site, current);
        }
        if (index != final_read()) {
            *destination = (byte)('a' + index % 26);
            *reported = 1;
            return 0;
        }
        if (which == 2) return 0;
        *destination = '\n';
        *reported = 1;
        return which == 3 ? 5 : 0;
    }
    require(phase == RECOVERY && recovery_reads < 3 && !releases);
    u32 index = recovery_reads++;
    if (index == 0) {
        require(!allocations);
        separate_site(discard_site, current);
        recovery_public_site = current;
    } else {
        require(allocations == 1 && backing);
        if (index == 1) {
            separate_site(recovery_public_site, current);
            recovery_tail_site = current;
        } else equal_site(recovery_tail_site, current);
    }
    *destination = ((const byte *)"OK\n")[index];
    *reported = 1;
    return 0;
}

void *__wrap___wosy_core_alloc(u64 size, u64 alignment) {
    require(phase == RECOVERY && !releases);
    if (allocations == 0) {
        require(recovery_reads == 1 && size == 8 && alignment == 1);
        backing = __real___wosy_core_alloc(size, alignment);
        require(backing != 0);
        ++allocations;
        return backing;
    }
    require(allocations == 1 && recovery_reads == 3 && size == 16 && alignment == 8);
    record = __real___wosy_core_alloc(size, alignment);
    require(record && record != backing);
    ++allocations;
    return record;
}

void __wrap___wosy_core_free(void *address) {
    require(phase == RELEASING && allocations == 2 && releases < 2);
    require(address == (releases == 0 ? backing : record));
    ++releases;
    __real___wosy_core_free(address);
}

void CHECKPOINT(int event, int sequence, word data, word slot) {
    if (event == 2) {
        require(sequence == 0 && phase == RECOVERY && recovery_reads == 3 && allocations == 2 && releases == 0);
        require(data == (word)backing && slot && *(const word *)slot == (word)record);
        require(((const byte *)data)[0] == 'O' && ((const byte *)data)[1] == 'K');
        phase = RELEASING;
        return;
    }
    require(!data && !slot);
    switch (event) {
    case 0:
        require(phase == IDLE && sequence == (int)next_case && next_case < 4);
        which = next_case++;
        reads = allocations = releases = 0;
        phase = DISCARD;
        return;
    case 1:
        require(phase == DISCARD && sequence == (int)which && reads == final_read() + 1);
        require(!allocations && !releases);
        phase = which == 0 ? RECOVERY : IDLE;
        return;
    case 3:
        require(sequence == 0 && phase == RELEASING && releases == 1);
        __wrap___wosy_core_free(record);
        require(releases == 2);
        phase = IDLE;
        return;
    case 4:
        require(sequence == 3 && phase == IDLE && next_case == 4 && recovery_reads == 3);
        phase = DONE;
        return;
    }
    __wosy_core_system_panic();
}

int FD_WRITE(int fd, word iovec, int vectors, word count) {
    (void)fd; (void)iovec; (void)vectors; (void)count;
    __wosy_core_system_panic();
}

int POLL_ONEOFF(word in, word out, int count, word result) {
    (void)in; (void)out; (void)count; (void)result;
    __wosy_core_system_panic();
}

void PROC_EXIT(int code) {
    (void)code;
    __wosy_core_system_panic();
}
