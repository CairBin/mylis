#include "libmathlib.h"
#include "mylrt.h"
#include <stdio.h>
#include <string.h>

int main(void) {
    mathlib_init();
    mylis_ping();
    if (mylis_add(2, 3) != 5) return 1;
    if (mylis_scaled(4) != 12) return 2;   /* 静态字段 rate = 3 */
    MyStr* g = mylis_greet(mylrt_str_lit("library", 7));
    const char* p = myrt_str_ptr(g);
    if (strcmp(p, "hello, library!") != 0) return 3;
    printf("add=5 scaled=12 greet=%s\n", p);
    return 0;
}
