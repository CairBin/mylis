/* Runtime ABI probe: link myrt + mylgc with the current C toolchain.
 * It only verifies library format/CRT/symbol compatibility, not business logic. */
#include <stddef.h>

extern void myrt_init(void);
extern void* myrt_gc_backend_alloc(size_t n);
extern void myrt_gc_backend_collect(void);
extern void myrt_gc_backend_init(void);

int main(void) {
    myrt_gc_backend_init();
    void* p = myrt_gc_backend_alloc(32);
    myrt_gc_backend_collect();
    myrt_init();
    return p == NULL ? 1 : 0;
}
