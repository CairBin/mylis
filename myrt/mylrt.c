/* ============================================================================
 * mylrt —— Mylis 运行时实现
 *
 * 内存模型（第 2.5 节）：
 *   - 全部堆对象（class 实例、string）由 Boehm GC 分配，引用语义、无拷贝；
 *   - 值类型（i8..f64 / myrt_bool / Result / Option）在栈上；
 *   - C 侧长期持有 GC 对象必须调用 myrt_register_root。
 * ==========================================================================*/
#include "mylrt.h"
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>
#include <math.h>
#include "mylrt_gc.h"     /* GC 后端抽象（默认 Boehm） */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <time.h>

/* ------------------------------------------------------------------ 工具 */
static void mylrt_oom(void) {
    fputs("mylrt: out of memory\n", stderr);
    abort();
}

static void* mylrt_gc_alloc(size_t n) {
    void* p = myrt_gc_backend_alloc(n);
    if (!p) mylrt_oom();
    return p;
}

static myrt_u64 mylrt_hash(const char* s, myrt_u64 len) {
    myrt_u64 h = 1469598103934665603ULL;      /* FNV-1a 64 */
    for (myrt_u64 i = 0; i < len; i++) {
        h ^= (myrt_u64)(unsigned char)s[i];
        h *= 1099511628211ULL;
    }
    return h;
}

/* ------------------------------------------------- 字符串构造（恒 NUL 结尾） */
static MyStr* mylrt_str_new(const char* s, myrt_u64 len) {
    MyStr* m = (MyStr*)mylrt_gc_alloc(sizeof(MyStr) + (size_t)len + 1);
    char* d  = (char*)(m + 1);
    if (len && s) memcpy(d, s, (size_t)len);
    d[len] = '\0';
    m->len  = len;
    m->data = d;
    return m;
}

/* ------------------------------------------------------- 字面串驻留表 */
#define MYLRT_INTERN_BUCKETS 4096
static MyStr** g_intern[MYLRT_INTERN_BUCKETS];
static MyStr*** g_intern_next;   /* 链接桶（GC 数组，由静态指针引导扫描） */

MyStr* mylrt_str_from_buf(const char* s, myrt_u64 len) {
    return mylrt_str_new(s, len);
}

MyStr* mylrt_str_lit(const char* s, myrt_u64 len) {
    myrt_u64 h = mylrt_hash(s, len) % MYLRT_INTERN_BUCKETS;
    /* 桶内线性扫描：桶数固定，链长度极小（字面串规模有限） */
    if (g_intern[h]) {
        for (size_t i = 0; g_intern[h][i]; i++) {
            MyStr* m = g_intern[h][i];
            if (m->len == len && memcmp(m->data, s, (size_t)len) == 0) return m;
        }
    } else {
        g_intern[h] = (MyStr**)mylrt_gc_alloc(sizeof(MyStr*) * 64);
        g_intern[h][0] = NULL;
    }
    size_t n = 0;
    while (g_intern[h][n]) n++;
    if (n >= 63) { /* 溢出：直接新建，不驻留 */
        return mylrt_str_new(s, len);
    }
    MyStr* m = mylrt_str_new(s, len);
    g_intern[h][n]     = m;
    g_intern[h][n + 1] = NULL;
    return m;
}

/* --------------------------------------------------------------- 字符串操作 */
MyStr* mylrt_str_concat(MyStr* a, MyStr* b) {
    if (!a) a = mylrt_str_lit("", 0);
    if (!b) b = mylrt_str_lit("", 0);
    myrt_u64 len = a->len + b->len;
    MyStr* m = (MyStr*)mylrt_gc_alloc(sizeof(MyStr) + (size_t)len + 1);
    char* d = (char*)(m + 1);
    memcpy(d, a->data, (size_t)a->len);
    memcpy(d + a->len, b->data, (size_t)b->len);
    d[len] = '\0';
    m->len  = len;
    m->data = d;
    return m;
}

myrt_bool mylrt_str_eq(MyStr* a, MyStr* b) {
    if (a == b) return true;
    if (!a || !b) return false;
    if (a->len != b->len) return false;
    return memcmp(a->data, b->data, (size_t)a->len) == 0;
}

myrt_i32 mylrt_str_cmp(MyStr* a, MyStr* b) {
    if (a == b) return 0;
    if (!a) return -1;
    if (!b) return 1;
    myrt_u64 n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->data, b->data, (size_t)n);
    if (c) return c < 0 ? -1 : 1;
    if (a->len == b->len) return 0;
    return a->len < b->len ? -1 : 1;
}

myrt_u64 mylrt_hash_u64(myrt_u64 x) {
    x ^= x >> 30;
    x *= 0xbf58476d1ce4e5b9ULL;
    x ^= x >> 27;
    x *= 0x94d049bb133111ebULL;
    x ^= x >> 31;
    return x;
}

myrt_u64 mylrt_hash_f64(double x) {
    union { double f; myrt_u64 u; } bits;
    bits.f = x;
    return mylrt_hash_u64(bits.u);
}

myrt_u64 mylrt_str_hash(MyStr* a) {
    if (!a) return 0;
    myrt_u64 h = 1469598103934665603ULL;
    myrt_u64 i = 0;
    while (i < a->len) {
        h ^= (myrt_u64)a->data[i];
        h *= 1099511628211ULL;
        i = i + 1;
    }
    return h;
}
 
myrt_bool mylrt_str_starts_with(MyStr* s, MyStr* prefix) {
    if (!s || !prefix) return false;
    if (prefix->len > s->len) return false;
    return memcmp(s->data, prefix->data, (size_t)prefix->len) == 0;
}

myrt_bool mylrt_str_ends_with(MyStr* s, MyStr* suffix) {
    if (!s || !suffix) return false;
    if (suffix->len > s->len) return false;
    return memcmp(s->data + (s->len - suffix->len), suffix->data, (size_t)suffix->len) == 0;
}

myrt_i64 mylrt_str_index_of(MyStr* s, MyStr* needle) {
    if (!s || !needle) return -1;
    if (needle->len == 0) return 0;
    if (needle->len > s->len) return -1;
    myrt_u64 n = s->len - needle->len;
    for (myrt_u64 i = 0; i <= n; i++) {
        if (memcmp(s->data + i, needle->data, (size_t)needle->len) == 0) {
            return (myrt_i64)i;
        }
    }
    return -1;
}

myrt_bool mylrt_str_contains(MyStr* s, MyStr* needle) {
    return mylrt_str_index_of(s, needle) >= 0;
}

MyStr* mylrt_str_substring(MyStr* s, myrt_u64 start, myrt_u64 length) {
    if (!s) return mylrt_str_lit("", 0);
    if (start > s->len) start = s->len;
    if (length > s->len - start) length = s->len - start;
    return mylrt_str_new(s->data + start, length);
}

MyStr* mylrt_str_trim(MyStr* s) {
    if (!s) return mylrt_str_lit("", 0);
    myrt_u64 begin = 0;
    myrt_u64 end = s->len;
    myrt_u64 step = 1;
    while (begin < end) {
        myrt_u32 c = mylrt_utf8_decode((const unsigned char*)s->data, s->len, begin, &step);
        if (!mylrt_is_space(c)) break;
        begin += step;
    }
    while (end > begin) {
        /* 从 begin 重新扫描到 end 的上一个字符，保证找到字符起点 */
        myrt_u64 i = begin;
        myrt_u64 last = begin;
        myrt_u64 last_step = 1;
        while (i < end) {
            last = i;
            last_step = mylrt_utf8_step((const unsigned char*)s->data, s->len, i);
            i += last_step;
        }
        myrt_u32 c = mylrt_utf8_decode((const unsigned char*)s->data, s->len, last, &step);
        if (!mylrt_is_space(c)) break;
        end = last;
    }
    return mylrt_str_new(s->data + begin, end - begin);
}

static myrt_u32 mylrt_utf8_decode(const unsigned char* s, myrt_u64 len, myrt_u64 i, myrt_u64* step) {
    *step = 1;
    if (i >= len) return 0;
    unsigned char b = s[i];
    if ((b & 0x80) == 0) return b;
    if ((b & 0xE0) == 0xC0 && i + 1 < len && (s[i + 1] & 0xC0) == 0x80) {
        *step = 2;
        return ((myrt_u32)(b & 0x1F) << 6) | (myrt_u32)(s[i + 1] & 0x3F);
    }
    if ((b & 0xF0) == 0xE0 && i + 2 < len && (s[i + 1] & 0xC0) == 0x80 &&
        (s[i + 2] & 0xC0) == 0x80) {
        *step = 3;
        return ((myrt_u32)(b & 0x0F) << 12) | ((myrt_u32)(s[i + 1] & 0x3F) << 6) |
               (myrt_u32)(s[i + 2] & 0x3F);
    }
    if ((b & 0xF8) == 0xF0 && i + 3 < len && (s[i + 1] & 0xC0) == 0x80 &&
        (s[i + 2] & 0xC0) == 0x80 && (s[i + 3] & 0xC0) == 0x80) {
        *step = 4;
        return ((myrt_u32)(b & 0x07) << 18) | ((myrt_u32)(s[i + 1] & 0x3F) << 12) |
               ((myrt_u32)(s[i + 2] & 0x3F) << 6) | (myrt_u32)(s[i + 3] & 0x3F);
    }
    return b; /* 非法序列按一个字节字符处理 */
}

static myrt_u64 mylrt_utf8_step(const unsigned char* s, myrt_u64 len, myrt_u64 i) {
    myrt_u64 step = 1;
    (void)mylrt_utf8_decode(s, len, i, &step);
    return step;
}

static myrt_bool mylrt_is_space(myrt_u32 c) {
    return c == 0x09 || c == 0x0A || c == 0x0B || c == 0x0C || c == 0x0D ||
           c == 0x20 || c == 0x85 || c == 0xA0 || c == 0x1680 ||
           (c >= 0x2000 && c <= 0x200A) || c == 0x2028 || c == 0x2029 ||
           c == 0x202F || c == 0x205F || c == 0x3000;
}

myrt_u64 mylrt_str_char_len(MyStr* s) {
    if (!s) return 0;
    myrt_u64 count = 0;
    myrt_u64 i = 0;
    while (i < s->len) {
        i += mylrt_utf8_step((const unsigned char*)s->data, s->len, i);
        count++;
    }
    return count;
}

myrt_i64 mylrt_str_char_index_of(MyStr* s, MyStr* needle) {
    myrt_i64 byte_pos = mylrt_str_index_of(s, needle);
    if (byte_pos < 0) return -1;
    myrt_u64 count = 0;
    myrt_u64 i = 0;
    while (i < (myrt_u64)byte_pos) {
        i += mylrt_utf8_step((const unsigned char*)s->data, s->len, i);
        count++;
    }
    return (myrt_i64)count;
}

MyStr* mylrt_str_char_substring(MyStr* s, myrt_u64 start, myrt_u64 count) {
    if (!s) return mylrt_str_lit("", 0);
    myrt_u64 begin = 0;
    myrt_u64 chars = 0;
    while (begin < s->len && chars < start) {
        begin += mylrt_utf8_step((const unsigned char*)s->data, s->len, begin);
        chars++;
    }
    myrt_u64 end = begin;
    myrt_u64 n = 0;
    while (end < s->len && n < count) {
        end += mylrt_utf8_step((const unsigned char*)s->data, s->len, end);
        n++;
    }
    return mylrt_str_new(s->data + begin, end - begin);
}

MyStr* mylrt_str_char_at(MyStr* s, myrt_u64 index) {
    if (!s) return mylrt_str_lit("", 0);
    myrt_u64 i = 0;
    myrt_u64 n = 0;
    while (i < s->len && n < index) {
        i += mylrt_utf8_step((const unsigned char*)s->data, s->len, i);
        n++;
    }
    if (i >= s->len || n < index) return mylrt_str_lit("", 0);
    myrt_u64 step = mylrt_utf8_step((const unsigned char*)s->data, s->len, i);
    return mylrt_str_new(s->data + i, step);
}

myrt_u32 mylrt_str_char_code_at(MyStr* s, myrt_u64 index) {
    if (!s) return 0xFFFFFFFFu;
    myrt_u64 i = 0;
    myrt_u64 n = 0;
    while (i < s->len && n < index) {
        i += mylrt_utf8_step((const unsigned char*)s->data, s->len, i);
        n++;
    }
    if (i >= s->len || n < index) return 0xFFFFFFFFu;
    myrt_u64 step = 1;
    return mylrt_utf8_decode((const unsigned char*)s->data, s->len, i, &step);
}

MylA_au8* mylrt_str_to_bytes(MyStr* s) {
    myrt_u64 len = s ? s->len : 0;
    MylA_au8* arr = (MylA_au8*)mylrt_arr_alloc(1, len);
    unsigned char* data = (unsigned char*)arr + sizeof(myrt_u64);
    if (s && len) memcpy(data, s->data, (size_t)len);
    return arr;
}

MyStr* mylrt_str_from_bytes(MylA_au8* arr) {
    if (!arr) return mylrt_str_lit("", 0);
    myrt_u64 len = *(myrt_u64*)arr;
    unsigned char* data = (unsigned char*)arr + sizeof(myrt_u64);
    return mylrt_str_new((const char*)data, len);
}

MyStr* mylrt_str_replace(MyStr* s, MyStr* from, MyStr* to) {
    if (!s || !from || !to) return s;
    if (from->len == 0) return s;
    myrt_u64 count = 0;
    for (myrt_u64 i = 0; i + from->len <= s->len; ) {
        if (memcmp(s->data + i, from->data, (size_t)from->len) == 0) {
            count++;
            i += from->len;
        } else {
            i++;
        }
    }
    if (count == 0) return s;
    if (to->len > from->len && count > (SIZE_MAX - s->len) / (to->len - from->len)) {
        return mylrt_str_lit("", 0);
    }
    myrt_u64 total = s->len + count * (to->len - from->len);
    if (to->len < from->len) {
        total = s->len - count * (from->len - to->len);
    }
    MyStr* m = (MyStr*)mylrt_gc_alloc(sizeof(MyStr) + (size_t)total + 1);
    char* d = (char*)(m + 1);
    myrt_u64 out = 0;
    myrt_u64 i = 0;
    while (i + from->len <= s->len) {
        if (memcmp(s->data + i, from->data, (size_t)from->len) == 0) {
            memcpy(d + out, to->data, (size_t)to->len);
            out += to->len;
            i += from->len;
        } else {
            d[out++] = s->data[i++];
        }
    }
    while (i < s->len) d[out++] = s->data[i++];
    d[out] = '\0';
    m->len = out;
    m->data = d;
    return m;
}

MyStr* mylrt_str_repeat(MyStr* s, myrt_u64 count) {
    if (!s || count == 0 || s->len == 0) return mylrt_str_lit("", 0);
    if (count > (myrt_u64)(SIZE_MAX / s->len)) {
        return mylrt_str_lit("", 0);
    }
    myrt_u64 total = s->len * count;
    MyStr* m = (MyStr*)mylrt_gc_alloc(sizeof(MyStr) + (size_t)total + 1);
    char* d = (char*)(m + 1);
    for (myrt_u64 i = 0; i < count; i++) {
        memcpy(d + (size_t)(i * s->len), s->data, (size_t)s->len);
    }
    d[total] = '\0';
    m->len = total;
    m->data = d;
    return m;
}

/* --------------------------------------------------------------- 运行时入口 */
static char** g_roots;          /* GC 数组：C 侧长期持有的对象 */
static myrt_u64 g_root_n;
static myrt_u64 g_root_cap;

void myrt_init(void) {
    /* GC 后端在 main 内初始化，保证静态数据段被扫描 */
    myrt_gc_backend_init();
    {
        /* 自研 GC 的自动回收阈值：MYLIS_GC_THRESHOLD=字节数（0 关闭） */
        const char* th = getenv("MYLIS_GC_THRESHOLD");
        if (th && *th) {
            long v = strtol(th, NULL, 10);
            if (v >= 0) {
                myrt_gc_backend_set_threshold((size_t)v);
            }
        }
    }
    if (g_roots == NULL) {
        g_root_cap = 256;
        g_roots = (char**)mylrt_gc_alloc(sizeof(char*) * (size_t)g_root_cap);
        g_root_n = 0;
        myrt_gc_backend_add_root_range(&g_roots, sizeof(char*));
        /* 字面量驻留表本身也是 GC 对象（桶数组），必须作为根扫描 */
        myrt_gc_backend_add_root_range((void*)g_intern, sizeof(g_intern));
    }
}

typedef struct {
    size_t collections;
    size_t live_bytes;
    size_t total_allocated;
    size_t threshold;
    size_t max_mark_cycles;
    size_t max_sweep_cycles;
} MylGcStats;

/* GC 统计由后端实现；simple/boehm 没有该符号时弱引用回退为 0 */
extern void myrt_gc_backend_stats(MylGcStats* out);

void myrt_gc_stats_dump(void) {
    MylGcStats st;
    st.collections = 0; st.live_bytes = 0; st.total_allocated = 0; st.threshold = 0;
    st.max_mark_cycles = 0; st.max_sweep_cycles = 0;
    myrt_gc_backend_stats(&st);
    fprintf(stderr, "[mylgc] collections=%lu live=%lu total=%lu threshold=%lu mark_max=%llu sweep_max=%llu\n",
            (unsigned long)st.collections, (unsigned long)st.live_bytes,
            (unsigned long)st.total_allocated, (unsigned long)st.threshold,
            (unsigned long long)st.max_mark_cycles, (unsigned long long)st.max_sweep_cycles);
}

void myrt_gc_write_barrier(void* value) {
    myrt_gc_backend_write_barrier(value);
}

void myrt_gc_collect(void) {
    myrt_gc_backend_collect();
}

/* 语言/编译器可见的静态根登记入口；底层后端用 size_t，这里做 ABI 转换。 */
void myrt_gc_add_root_range(void* start, myrt_u64 bytes) {
    myrt_gc_backend_add_root_range(start, (size_t)bytes);
}

/* ------------------------------------------------------------- 内存 API */
char* myrt_alloc(myrt_u64 size) {
    return (char*)mylrt_gc_alloc((size_t)size);
}

void myrt_register_root(char* obj) {
    if (!obj) return;
    if (g_root_n == g_root_cap) {
        myrt_u64 ncap = g_root_cap * 2;
        char** n = (char**)mylrt_gc_alloc(sizeof(char*) * (size_t)ncap);
        for (myrt_u64 i = 0; i < g_root_n; i++) n[i] = g_roots[i];
        g_roots = n;
        g_root_cap = ncap;
    }
    g_roots[g_root_n++] = obj;
}

void myrt_unregister_root(char* obj) {
    if (!obj) return;
    for (myrt_u64 i = 0; i < g_root_n; i++) {
        if (g_roots[i] == obj) {
            g_roots[i] = g_roots[--g_root_n];
            g_roots[g_root_n] = NULL;
            return;
        }
    }
}

/* --------------------------------------------------------- 字符串 / IO API */
MyStr* myrt_str_cstr(char* p) {
    if (!p) return mylrt_str_lit("", 0);
    return mylrt_str_new((const char*)p, (myrt_u64)strlen((const char*)p));
}

char* myrt_str_ptr(MyStr* s) {
    return s ? s->data : (char*)"";
}

myrt_u64 myrt_str_len(MyStr* s) {
    return s ? s->len : 0;
}

void myrt_print(MyStr* s) {
    if (!s) return;
    fwrite(s->data, 1, (size_t)s->len, stdout);
}

void myrt_println(MyStr* s) {
    if (s) fwrite(s->data, 1, (size_t)s->len, stdout);
    fputc('\n', stdout);
}

static MYLRT_NORETURN void mylrt_abort_with(const char* msg);

static void mylrt_abort_with(const char* msg) {
    fputs("panic: ", stderr);
    fputs(msg, stderr);
    fputc('\n', stderr);
    fflush(NULL);
    abort();
}

void myrt_panic(MyStr* msg) {
    fputs("panic: ", stderr);
    if (msg) fwrite(msg->data, 1, (size_t)msg->len, stderr);
    fputc('\n', stderr);
    fflush(NULL);
    abort();
}

void mylrt_panic_msg(const char* what)   { mylrt_abort_with(what); }
void mylrt_panic_unwrap(void)            { mylrt_abort_with("unwrap() called on a failed value"); }
void* mylrt_arr_alloc(myrt_u64 elem_size, myrt_u64 len) {
    myrt_u64 total = sizeof(myrt_u64) + elem_size * len;
    char* p = (char*)myrt_alloc(total);
    if (!p) {
        fprintf(stderr, "panic: out of memory allocating array\n");
        abort();
    }
    *(myrt_u64*)p = len;
    /* 语言规范要求 new T[n] 的元素全部零初始化；不能依赖分配器的偶然行为。 */
    memset(p + sizeof(myrt_u64), 0, (size_t)(elem_size * len));
    return (void*)p;
}

myrt_u64 mylrt_arr_index(myrt_u64 len, myrt_i64 idx) {
    if (idx < 0 || (myrt_u64)idx >= len) {
        fprintf(stderr, "panic: array index %lld out of bounds (len %llu)\n",
                (long long)idx, (unsigned long long)len);
        abort();
    }
    return (myrt_u64)idx;
}

void mylrt_assert(myrt_bool cond, const char* what) {
    if (!cond) mylrt_abort_with(what);
}

/* ------------------------------------------------------- 格式化 / 计时 API */
MyStr* myrt_i64_to_string(myrt_i64 v) {
    char b[32];
    snprintf(b, sizeof b, "%lld", (long long)v);
    return mylrt_str_new(b, (myrt_u64)strlen(b));
}

MyStr* myrt_u64_to_string(myrt_u64 v) {
    char b[32];
    snprintf(b, sizeof b, "%llu", (unsigned long long)v);
    return mylrt_str_new(b, (myrt_u64)strlen(b));
}

MyStr* myrt_f64_to_string(double v) {
    char b[64];
    if (isnan(v))            return mylrt_str_lit("NaN", 3);
    if (isinf(v))            return v > 0 ? mylrt_str_lit("inf", 3) : mylrt_str_lit("-inf", 4);
    snprintf(b, sizeof b, "%.10g", v);
    return mylrt_str_new(b, (myrt_u64)strlen(b));
}

MyStr* myrt_f64_to_string_prec(double v, myrt_i32 precision) {
    char b[64];
    if (isnan(v))            return mylrt_str_lit("NaN", 3);
    if (isinf(v))            return v > 0 ? mylrt_str_lit("inf", 3) : mylrt_str_lit("-inf", 4);
    if (precision < 0) precision = 0;
    if (precision > 17) precision = 17;
    snprintf(b, sizeof b, "%.*f", (int)precision, v);
    return mylrt_str_new(b, (myrt_u64)strlen(b));
}
MyStr* myrt_f64_to_string_fmt(double v, myrt_i32 width, myrt_i32 precision, myrt_i32 flags) {
    char b[128];
    char fmt[32];
    int n = 0;
    if (width < 0) width = 0;
    if (width > 100) width = 100;
    if (precision < 0) precision = 0;
    if (precision > 17) precision = 17;
    fmt[n++] = '%';
    if (flags & 1) fmt[n++] = '-';       /* left */
    if (flags & 2) fmt[n++] = '+';       /* plus */
    else if (flags & 8) fmt[n++] = ' ';  /* space */
    if (flags & 4) fmt[n++] = '0';        /* zero pad */
    n += snprintf(fmt + n, sizeof(fmt) - (size_t)n, "%d.%d", (int)width, (int)precision);
    fmt[n++] = 'f';
    fmt[n] = '\0';
    snprintf(b, sizeof b, fmt, v);
    return mylrt_str_new(b, (myrt_u64)strlen(b));
}

double myrt_clock(void) {
    return (double)clock() / (double)CLOCKS_PER_SEC;
}
