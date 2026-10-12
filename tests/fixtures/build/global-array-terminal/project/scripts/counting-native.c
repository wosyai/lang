#include <malloc.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <windows.h>

static void *allocated[3];
static int freed[3];
static int allocation_count;
static int free_count;
static uint64_t sizes[3];
static const uint64_t expected_sizes[3] = {2, 64, 72};
static const uint64_t expected_alignments[3] = {1, 8, 8};
static const char *sites[3] = {
    "global backing", "capture environment", "remainder cell"
};

static void fail(int code) {
    fprintf(stderr, "allocations=%d frees=%d failure=%d\n", allocation_count, free_count, code);
    for (int i = 0; i < allocation_count; ++i)
        fprintf(stderr, "generation=%d site=%s size=%llu freed=%d\n", i,
                sites[i], (unsigned long long)sizes[i], freed[i]);
    fflush(stderr);
    TerminateProcess(GetCurrentProcess(), code);
}

static void check(void) {
    if (allocation_count != 3 || free_count != 3) fail(79);
    for (int i = 0; i < allocation_count; ++i) if (freed[i] != 1) fail(79);
}

void *__wosy_core_alloc(uint64_t size, uint64_t alignment) {
    if (allocation_count == 0) atexit(check);
    if (allocation_count >= 3) fail(77);
    if (size != expected_sizes[allocation_count] || alignment != expected_alignments[allocation_count]) fail(78);
    void *pointer = _aligned_malloc((size_t)size, (size_t)alignment);
    if (!pointer) fail(76);
    int i = allocation_count++;
    allocated[i] = pointer;
    sizes[i] = size;
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
