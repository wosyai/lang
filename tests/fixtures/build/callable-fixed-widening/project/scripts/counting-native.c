#include <malloc.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <windows.h>

static void *allocated[64];
static uint64_t sizes[64];
static uint64_t alignments[64];
static int freed[64];
static int allocation_count;
static int free_count;
static const uint64_t expected_sizes[6] = {56, 57, 56, 57, 56, 56};
static const char *sites[6] = {
    "fixed[0] environment", "first capture cell", "fixed[1] environment",
    "second capture cell", "copied backing", "replacement environment"
};

static void fail(int code) {
    fprintf(stderr, "allocations=%d frees=%d failure=%d\n", allocation_count, free_count, code);
    for (int i = 0; i < allocation_count; ++i)
        fprintf(stderr, "generation=%d site=%s size=%llu alignment=%llu freed=%d owners=%llu\n", i,
                sites[i], (unsigned long long)sizes[i], (unsigned long long)alignments[i], freed[i],
                freed[i] || sizes[i] == 56 ? 0 : (unsigned long long)*(uint64_t *)allocated[i]);
    fflush(stderr);
    TerminateProcess(GetCurrentProcess(), code);
}

static void check(void) {
    if (allocation_count != 6 || free_count != 6) fail(79);
    for (int i = 0; i < allocation_count; ++i) if (freed[i] != 1) fail(79);
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (allocation_count == 0) atexit(check);
    if (allocation_count >= 6) fail(77);
    if (size != expected_sizes[allocation_count] || alignment != 8) fail(78);
    void *pointer = _aligned_malloc((size_t)size, (size_t)alignment);
    if (!pointer) fail(76);
    int i = allocation_count++;
    allocated[i] = pointer;
    sizes[i] = size;
    alignments[i] = alignment;
    return pointer;
}

void __wosy_core_free(void *pointer) {
    for (int i = 0; i < allocation_count; ++i) {
        if (allocated[i] == pointer && !freed[i]) {
            freed[i] = 1;
            ++free_count;
            _aligned_free(pointer);
            return;
        }
    }
    fail(75);
}

void __wosy_core_system_panic(void) { fail(74); }
