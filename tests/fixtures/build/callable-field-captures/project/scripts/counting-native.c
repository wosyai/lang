#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <windows.h>

static void *arrays[4];
static int array_allocations;
static int array_frees[4];
static int array_live[4];
static void *capture_allocations[6];
static int capture_count;
static int capture_frees[6];
static int capture_live[6];
static int free_invocations;
static int other_frees;
static int registered;

static void check_allocations(void) {
    int valid = array_allocations == 4 && capture_count == 6 &&
                free_invocations == 10 && other_frees == 0;
    for (int index = 0; index < 4; ++index) {
        valid = valid && arrays[index] != NULL && array_frees[index] == 1 && !array_live[index];
    }
    for (int index = 0; index < 6; ++index) {
        valid = valid && capture_allocations[index] != NULL && capture_frees[index] == 1 && !capture_live[index];
    }
    if (!valid) {
        TerminateProcess(GetCurrentProcess(), 79);
    }
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (!registered) {
        atexit(check_allocations);
        registered = 1;
    }
    size_t native_alignment = (size_t)alignment;
    if (native_alignment < sizeof(void *)) {
        native_alignment = sizeof(void *);
    }
    void *pointer = _aligned_malloc((size_t)size, native_alignment);
    if (pointer != NULL && (size == 2 || size == 3) && alignment == 1) {
        if (array_allocations < 4) {
            arrays[array_allocations] = pointer;
            array_live[array_allocations] = 1;
        }
        ++array_allocations;
    }
    if (pointer != NULL && (size == 64 || size == 72) && alignment == 8) {
        if (capture_count < 6) {
            capture_allocations[capture_count] = pointer;
            capture_live[capture_count] = 1;
        }
        ++capture_count;
    }
    return pointer;
}

void __wosy_core_free(void *pointer) {
    ++free_invocations;
    int found = 0;
    for (int index = 0; index < 4; ++index) {
        if (array_live[index] && pointer == arrays[index]) {
            ++array_frees[index];
            array_live[index] = 0;
            found = 1;
            break;
        }
    }
    if (!found) {
        for (int index = 0; index < 6; ++index) {
            if (capture_live[index] && pointer == capture_allocations[index]) {
                ++capture_frees[index];
                capture_live[index] = 0;
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
