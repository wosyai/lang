#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <windows.h>

static void *generation;
static unsigned int allocations;
static unsigned int frees;

static void check_generation(void) {
    if (allocations != 1 || frees != 1 || generation != NULL) {
        TerminateProcess(GetCurrentProcess(), 79);
    }
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (!allocations) {
        atexit(check_generation);
    }
    size_t align = (size_t)alignment;
    if (align < sizeof(void *)) align = sizeof(void *);
    void *pointer = _aligned_malloc((size_t)size, align);
    if (pointer) {
        ++allocations;
        if (generation != NULL || allocations != 1) TerminateProcess(GetCurrentProcess(), 79);
        generation = pointer;
    }
    return pointer;
}

void __wosy_core_free(void *pointer) {
    if (pointer != generation || frees != 0) TerminateProcess(GetCurrentProcess(), 79);
    ++frees;
    generation = NULL;
    _aligned_free(pointer);
}

void __wosy_core_system_panic(void) {
    abort();
}
