#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <windows.h>

static void *allocated[12];
static int freed[12];
static int allocation_count;
static int free_count;
static int arrays;
static int environments;
static int cells;
static const int release_order[12] = {3, 0, 5, 9, 10, 4, 11, 8, 6, 7, 2, 1};

static void check(void) {
    int valid = allocation_count == 12 && free_count == 12 && arrays == 2 && environments == 7 && cells == 3;
    for (int i = 0; i < 12; ++i) valid = valid && freed[i] == 1;
    if (!valid) TerminateProcess(GetCurrentProcess(), 79);
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (allocation_count == 0) atexit(check);
    if ((allocation_count == 0 && size == 56 || allocation_count == 8 && size == 72) && alignment == 8) ++arrays;
    else if ((allocation_count == 1 || allocation_count == 3 || allocation_count == 9 || allocation_count == 10) && size == 56 && alignment == 8
        || (allocation_count == 5 || allocation_count == 7 || allocation_count == 11) && size == 64 && alignment == 8) ++environments;
    else if ((allocation_count == 2 || allocation_count == 4) && size == 57 && alignment == 8
        || allocation_count == 6 && size == 72 && alignment == 8) ++cells;
    else TerminateProcess(GetCurrentProcess(), 78);
    if (allocation_count >= 12) TerminateProcess(GetCurrentProcess(), 77);
    void *pointer = _aligned_malloc((size_t)size, (size_t)alignment);
    if (!pointer) TerminateProcess(GetCurrentProcess(), 76);
    allocated[allocation_count++] = pointer;
    return pointer;
}

void __wosy_core_free(void *pointer) {
    int found = 0;
    for (int i = 0; i < allocation_count; ++i) {
        if (allocated[i] == pointer && freed[i] == 0) {
            if (i != release_order[free_count]) TerminateProcess(GetCurrentProcess(), 70);
            if (i == 0 && *(uint64_t *)((char *)allocated[1] + 24) != 2) TerminateProcess(GetCurrentProcess(), 71);
            if ((i == 5 || i == 8 || i == 6 || i == 7) && (freed[1] || *(uint64_t *)((char *)allocated[1] + 24) != 1)) TerminateProcess(GetCurrentProcess(), 71);
            if (i == 1 && (!freed[0] || !freed[6] || !freed[8])) TerminateProcess(GetCurrentProcess(), 73);
            if (i == 1 && *(uint64_t *)((char *)pointer + 24) != 0) TerminateProcess(GetCurrentProcess(), 72);
            if (i == 3 && *(uint64_t *)((char *)pointer + 24) != 0) TerminateProcess(GetCurrentProcess(), 72);
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
