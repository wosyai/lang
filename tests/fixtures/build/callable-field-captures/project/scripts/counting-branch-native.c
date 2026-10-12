#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <windows.h>

static void *arrays[4];
static int array_live[4];
static int array_frees[4];
static int array_count;
static void *owners[12];
static int owner_live[12];
static int owner_frees[12];
static int owner_count;
static int free_count;
static int other_frees;
static int registered;

static void check(void) {
    int valid = array_count == 4 && owner_count == 12 && free_count == 16 && other_frees == 0;
    for (int i = 0; i < 4; ++i) {
        valid = valid && arrays[i] != NULL && array_frees[i] == 1 && !array_live[i];
    }
    for (int i = 0; i < 12; ++i) {
        valid = valid && owners[i] != NULL && owner_frees[i] == 1 && !owner_live[i];
    }
    if (!valid) {
        TerminateProcess(GetCurrentProcess(), 79);
    }
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (!registered) {
        atexit(check);
        registered = 1;
    }
    size_t align = (size_t)alignment;
    if (align < sizeof(void *)) {
        align = sizeof(void *);
    }
    void *pointer = _aligned_malloc((size_t)size, align);
    if (pointer != NULL && size == 2 && alignment == 1) {
        if (array_count < 4) {
            arrays[array_count] = pointer;
            array_live[array_count] = 1;
        }
        ++array_count;
    }
    if (pointer != NULL && (size == 64 || size == 72) && alignment == 8) {
        if (owner_count < 12) {
            owners[owner_count] = pointer;
            owner_live[owner_count] = 1;
        }
        ++owner_count;
    }
    return pointer;
}

void __wosy_core_free(void *pointer) {
    ++free_count;
    int found = 0;
    for (int i = 0; i < 4; ++i) {
        if (array_live[i] && arrays[i] == pointer) {
            array_live[i] = 0;
            ++array_frees[i];
            found = 1;
            break;
        }
    }
    if (!found) {
        for (int i = 0; i < 12; ++i) {
            if (owner_live[i] && owners[i] == pointer) {
                owner_live[i] = 0;
                ++owner_frees[i];
                found = 1;
                break;
            }
        }
    }
    if (!found) {
        ++other_frees;
    }
    _aligned_free(pointer);
}

void __wosy_core_system_panic(void) {
    abort();
}
