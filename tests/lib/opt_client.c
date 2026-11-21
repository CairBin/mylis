#include "liboptlib.h"
#include <stdio.h>
int main(void) {
    optlib_init();
    MylO_oi32 a = maybe(7);
    printf("a=%d,%d\n", a.tag, a.value);
    MylO_oi32 b = maybe(-1);
    printf("b=%d\n", b.tag);
    MylO_ostr n = named(3);
    printf("n=%d,%.*s\n", n.tag, (int)n.value->len, n.value->data);
    MylR_ri32_str r = res(4);
    printf("r=%d,%d\n", r.tag, r.ok);
    MylR_ri32_str e = res(-1);
    printf("e=%d,%.*s\n", e.tag, (int)e.err->len, e.err->data);
    return 0;
}
