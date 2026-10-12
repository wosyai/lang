#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <windows.h>

static void *allocated[3];
static int freed[3];
static int allocation_count;
static int free_count;
static const uint64_t expected_size[3] = {56, 56, 57};
static const int release_order[3] = {0, 2, 1};

static void check(void) {
    int valid = allocation_count == 3 && free_count == 3;
    for (int i = 0; i < 3; ++i) {
        valid = valid && freed[i] == 1;
    }
    if (!valid) {
        TerminateProcess(GetCurrentProcess(), 79);
    }
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (allocation_count == 0) {
        atexit(check);
    }
    if (allocation_count >= 3 || alignment != 8 || size != expected_size[allocation_count]) {
        TerminateProcess(GetCurrentProcess(), 78);
    }
    void *pointer = _aligned_malloc((size_t)size, (size_t)alignment);
    if (!pointer) {
        TerminateProcess(GetCurrentProcess(), 76);
    }
    allocated[allocation_count++] = pointer;
    return pointer;
}

void __wosy_core_free(void *pointer) {
    int found = 0;
    for (int i = 0; i < allocation_count; ++i) {
        if (allocated[i] == pointer && freed[i] == 0) {
            if (i != release_order[free_count]) {
                TerminateProcess(GetCurrentProcess(), 70);
            }
            freed[i] = 1;
            found = 1;
            break;
        }
    }
    if (!found) {
        TerminateProcess(GetCurrentProcess(), 75);
    }
    ++free_count;
    _aligned_free(pointer);
}

void __wosy_core_system_panic(void) { TerminateProcess(GetCurrentProcess(), 74); }
