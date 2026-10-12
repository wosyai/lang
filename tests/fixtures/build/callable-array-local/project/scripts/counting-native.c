#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <windows.h>

static void *allocated[7];
static int allocation_count;
static int freed[7];
static int free_count;
static int arrays;
static int environments;
static int cells;

static void check(void) {
    int valid = allocation_count == 7 && free_count == 7 && arrays == 1 && environments == 4 && cells == 2;
    for (int i = 0; i < 7; ++i) valid = valid && freed[i] == 1;
    if (!valid) TerminateProcess(GetCurrentProcess(), 79);
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (allocation_count == 0) atexit(check);
    if (size == 56 && alignment == 8 && allocation_count == 0) ++arrays;
    else if (size == 56 && alignment == 8) ++environments;
    else if (size == 57 && alignment == 8) ++cells;
    else TerminateProcess(GetCurrentProcess(), 78);
    if (allocation_count >= 7) TerminateProcess(GetCurrentProcess(), 77);
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
