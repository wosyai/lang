typedef __UINTPTR_TYPE__ word;
typedef unsigned char byte;
typedef unsigned int u32;
typedef unsigned long long u64;

extern _Noreturn void __wosy_core_system_panic(void);
extern void *__real___wosy_core_alloc(u64 size, u64 alignment);
extern void __real___wosy_core_free(void *address);

struct iovec { word destination; u32 length; };
struct read_site { word destination, iovec, count, stack; };

enum phase {
    IDLE, DISCARD, RECOVERY, RELEASING, DONE, DESCRIPTOR, DESCRIPTOR_RELEASING,
    FORMAT, FORMAT_WRITTEN, FORMAT_RELEASED, FINISHED
};
static enum phase phase;
static u32 which, next_case, reads, recovery_reads, allocations, releases;
static u32 probe, next_probe, probe_reads;
static struct read_site public_site, discard_site, repeat_site, recovery_public_site, recovery_tail_site;
static void *backing, *record;

#ifdef __wasm__
__attribute__((import_module("wasi_snapshot_preview1"), import_name("proc_exit")))
#endif
extern _Noreturn void exit(int code);
#define require(condition) do { if (!(condition)) exit(1 + ((__LINE__ - 1) % 255)); } while (0)

static void announce_complete(void) {
#ifdef __wasm__
    typedef struct { const char *data; u32 length; } output_vector;
    __attribute__((import_module("wasi_snapshot_preview1"), import_name("fd_write")))
    extern int host_write(int fd, const output_vector *vectors, int count, u32 *written);
    static const char marker[] = "numeric formatter owner verified\n";
    output_vector bytes = { marker, sizeof(marker) - 1 };
    u32 written = 0;
    require(host_write(1, &bytes, 1, &written) == 0 && written == bytes.length);
#else
    extern int puts(const char *);
    require(puts("numeric formatter owner verified") >= 0);
#endif
}

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
    if (phase == DESCRIPTOR) {
        require(!releases);
        u32 index = probe_reads++;
        if (probe == 0) {
            require(index < 3 && allocations == (index == 0 ? 0u : 1u));
            *destination = ((const byte *)"OK\n")[index];
            *reported = 1;
        } else if (probe == 3) {
            require(index < 2 && allocations == index);
            *destination = ((const byte *)"\r\n")[index];
            *reported = 1;
        } else {
            require(index == 0 && !allocations && probe < 3);
            if (probe == 1) {
                *destination = '\n';
                *reported = 1;
            }
        }
        return 0;
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
    if (phase == FORMAT) {
        require(!allocations && !releases && !backing && size >= 2 && alignment == 1);
        backing = __real___wosy_core_alloc(size, alignment);
        require(backing);
        ++allocations;
        return backing;
    }
    if (phase == DESCRIPTOR) {
        if (probe == 3) {
            if (!allocations) {
                require(probe_reads == 1 && !releases && size == 8 && alignment == 1);
                backing = __real___wosy_core_alloc(size, alignment);
                require(backing);
                ++allocations;
                return backing;
            }
            require(probe_reads == 2 && allocations == 1 && releases == 1 && size == 16 && alignment == 8);
            record = __real___wosy_core_alloc(size, alignment);
            require(record);
            ++allocations;
            return record;
        }
        require(!releases && probe < 2);
        if (probe == 0 && allocations == 0) {
            require(probe_reads == 1 && size == 8 && alignment == 1);
            backing = __real___wosy_core_alloc(size, alignment);
            require(backing);
            ++allocations;
            return backing;
        }
        require(allocations == (probe == 0 ? 1u : 0u));
        require(probe_reads == (probe == 0 ? 3u : 1u) && size == 16 && alignment == 8);
        record = __real___wosy_core_alloc(size, alignment);
        require(record && record != backing);
        ++allocations;
        return record;
    }
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
    if (phase == FORMAT_WRITTEN) {
        require(allocations == 1 && !releases && backing && address == backing);
        require(((const byte *)backing)[0] == '4' && ((const byte *)backing)[1] == '2');
        ++releases;
        __real___wosy_core_free(address);
        backing = 0;
        phase = FORMAT_RELEASED;
        return;
    }
    if (phase == DESCRIPTOR && probe == 3) {
        require(probe_reads == 2 && allocations == 1 && !releases && address == backing);
        ++releases;
        __real___wosy_core_free(address);
        backing = 0;
        return;
    }
    if (phase == DESCRIPTOR_RELEASING) {
        if (probe == 3) {
            require(releases == 1 && address == record);
            ++releases;
            __real___wosy_core_free(address);
            return;
        }
        require(probe < 2 && releases < (probe == 0 ? 2u : 1u));
        require(address == (probe == 0 && releases == 0 ? backing : record));
        ++releases;
        __real___wosy_core_free(address);
        return;
    }
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
    if (event == 6) {
        if (probe == 3) {
            require(phase == DESCRIPTOR && sequence == 3 && !slot && !data);
            require(probe_reads == 2 && allocations == 2 && releases == 1 && !backing && record);
            phase = DESCRIPTOR_RELEASING;
            return;
        }
        require(phase == DESCRIPTOR && sequence == (int)probe && !slot && !releases);
        require(probe_reads == (probe == 0 ? 3u : 1u));
        require(allocations == (probe == 0 ? 2u : probe == 1 ? 1u : 0u));
        if (probe == 0) {
            require(data == (word)backing && record);
            require(((const byte *)data)[0] == 'O' && ((const byte *)data)[1] == 'K');
        } else {
            require(!data && !backing && (probe == 1 ? record != 0 : record == 0));
        }
        phase = DESCRIPTOR_RELEASING;
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
    case 5:
        require(phase == DONE && sequence == (int)next_probe && next_probe < 4);
        probe = next_probe++;
        allocations = releases = probe_reads = 0;
        backing = record = 0;
        phase = DESCRIPTOR;
        return;
    case 7:
        require(phase == DESCRIPTOR_RELEASING && sequence == (int)probe);
        require(releases == allocations);
        phase = DONE;
        return;
    case 8:
        require(sequence == 0 && phase == DONE && next_probe == 4);
        backing = 0;
        allocations = releases = 0;
        phase = FORMAT;
        return;
    case 9:
        require(sequence == 0 && phase == FORMAT_RELEASED && !backing);
        require(allocations == 1 && releases == 1);
        phase = FINISHED;
        announce_complete();
        return;
    }
    __wosy_core_system_panic();
}

int FD_WRITE(int fd, word iovec, int vectors, word count) {
    require(phase == FORMAT && allocations == 1 && !releases && backing);
    require(fd == 1 && vectors == 1 && iovec && count && *(u32 *)count == 0);
    const struct iovec *buffer = (const struct iovec *)iovec;
    require(iovec != (word)backing && buffer->destination == (word)backing && buffer->length == 2);
    require(((const byte *)backing)[0] == '4' && ((const byte *)backing)[1] == '2');
    *(u32 *)count = 2;
    phase = FORMAT_WRITTEN;
    return 0;
}

int POLL_ONEOFF(word in, word out, int count, word result) {
    (void)in; (void)out; (void)count; (void)result;
    __wosy_core_system_panic();
}

void PROC_EXIT(int code) {
    (void)code;
    __wosy_core_system_panic();
}
