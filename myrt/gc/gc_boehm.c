/* GC 后端：Boehm-Demers-Weiser（默认） */
#include "mylrt_gc.h"
#include <gc.h>
#include <stdio.h>
#include <stdlib.h>

void myrt_gc_backend_init(void) {
    GC_INIT();
}

void* myrt_gc_backend_alloc(size_t n) {
    void* p = GC_MALLOC(n ? n : 1);
    if (!p) {
        fputs("mylrt: out of memory\n", stderr);
        abort();
    }
    return p;
}

void myrt_gc_backend_safepoint(void) {}

void myrt_gc_backend_remove_root_range(void* start) {
    (void)start;
}

void myrt_gc_backend_set_precise(int on) {
    (void)on;
}

unsigned long long myrt_gc_backend_frame_enter(void) { return 0; }
void myrt_gc_backend_frame_set_roots(unsigned long long id, const void* entries, unsigned long long count) {
    (void)id; (void)entries; (void)count;
}
void myrt_gc_backend_static_set_roots(const void* entries, unsigned long long count) {
    (void)entries; (void)count;
}

void myrt_gc_backend_frame_exit(unsigned long long id) { (void)id; }

void myrt_gc_backend_collect(void) {
    GC_gcollect();
}

void myrt_gc_backend_add_root_range(void* start, size_t bytes) {
    GC_add_roots(start, (char*)start + bytes);
}

void myrt_gc_backend_write_barrier_owner(void* owner_slot, void* value) {
    (void)owner_slot; (void)value;
}

void myrt_gc_backend_write_barrier(void* value) {
    (void)value;
}

void myrt_gc_backend_set_allocator(myrt_gc_alloc_fn a, myrt_gc_free_fn f) {
    (void)a;
    (void)f;
}

void myrt_gc_backend_set_threshold(size_t bytes) {
    (void)bytes;  /* Boehm 自带增量回收阈值 */
}

typedef struct { size_t collections, live_bytes, total_allocated, threshold; } MylGcStats;
void myrt_gc_backend_stats(MylGcStats* out) {
    if (!out) return;
    out->collections = 0; out->live_bytes = 0; out->total_allocated = 0; out->threshold = 0;
}
