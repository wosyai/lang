#include <malloc.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <windows.h>

static void *allocated[10];
static int allocation_count;
static int freed[10];
static int free_count;
static int backings;
static int environments;
static int cells;

static const uint64_t expected_sizes[10] = {56, 72, 56, 72, 40, 64, 72, 40, 64, 72};

static void check(void) {
    int valid = allocation_count == 10 && free_count == 10 && backings == 2 && environments == 4 && cells == 4;
    for (int i = 0; i < 10; ++i) valid = valid && freed[i] == 1;
    if (!valid) TerminateProcess(GetCurrentProcess(), 79);
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (allocation_count == 0) atexit(check);
    if (alignment != 8) TerminateProcess(GetCurrentProcess(), 78);
    if (allocation_count >= 10 || size != expected_sizes[allocation_count]) {
        fprintf(stderr, "unexpected cycle allocation #%d size=%llu align=%llu\n", allocation_count, size, alignment);
        TerminateProcess(GetCurrentProcess(), 78);
    }
    if (size == 40) ++backings;
    else if (size == 56 || size == 64) ++environments;
    else if (size == 72) ++cells;
    void *pointer = _aligned_malloc((size_t)size, (size_t)alignment);
    if (!pointer) TerminateProcess(GetCurrentProcess(), 76);
    allocated[allocation_count++] = pointer;
    return pointer;
}

void __wosy_core_free(void *pointer) {
    int found = 0;
    for (int i = 0; i < allocation_count; ++i) {
        if (allocated[i] == pointer && freed[i] == 0) {
            freed[i] = 1;
            found = 1;
            break;
        }
    }
    if (!found) TerminateProcess(GetCurrentProcess(), 75);
    ++free_count;
    _aligned_free(pointer);
}

void __wosy_core_system_panic(void) { TerminateProcess(GetCurrentProcess(), 74); }
