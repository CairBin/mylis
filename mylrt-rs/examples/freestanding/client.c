#include "mylrt.h"

static void sys_exit(int code) {
    __asm__ volatile("syscall" :: "a"(60), "D"(code) : "rcx", "r11", "memory");
    __builtin_unreachable();
}

void *memcpy(void *d, const void *s, unsigned long n) {
    unsigned char *dd = d; const unsigned char *ss = s;
    for (unsigned long i = 0; i < n; i++) dd[i] = ss[i];
    return d;
}

void *memset(void *d, int c, unsigned long n) {
    unsigned char *dd = d;
    for (unsigned long i = 0; i < n; i++) dd[i] = (unsigned char)c;
    return d;
}


int memcmp(const void *a, const void *b, unsigned long n) {
    const unsigned char *x = a; const unsigned char *y = b;
    for (unsigned long i = 0; i < n; i++) {
        if (x[i] != y[i]) return x[i] < y[i] ? -1 : 1;
    }
    return 0;
}

int bcmp(const void *a, const void *b, unsigned long n) { return memcmp(a, b, n); }

unsigned long strlen(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    return n;
}

void _start(void) {
    myrt_init();
    MyStr *s = mylrt_str_lit("freestanding ok", 15);
    myrt_println(s);
    sys_exit(0);
}
