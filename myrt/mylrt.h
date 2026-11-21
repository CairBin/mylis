/* ============================================================================
 * mylrt —— Mylis 语言运行时 ABI
 *
 * 本头文件是「语言 → C 后端」的唯一接口：生成的 C 只 include 它，
 * 不包含任何 C 标准库头（stdint/stdbool/stddef/stdlib/string 均不出现）。
 * 这样生成物只依赖 Mylis 运行时 ABI，而不是某个 libc 的具体头文件；
 * libc 依赖被收在 mylrt.c 与 GC 后端的 .c 里。
 *
 * 类型别名 myrt_i8..myrt_u64 由本文件定义（C11，带尺寸静态断言）。
 * 若将来接 IR 后端，这组别名就是 IR 的基本类型 ABI。
 *
 * 分两组：
 *   1) myrt_*    : 语言层可见的 unsafe 运行时 API（第 11 章固定 API），
 *                 在 .myl 侧只能通过 unsafe 域调用。
 *   2) mylrt_*   : 编译器内部发射使用的辅助符号，不属于语言 API，
 *                 用户代码禁止直接引用。
 *
 * ---------------------------------------------------------------------------
 * 【core 层 ABI 承诺】
 * 本文件导出的 myrt_* 符号是 core 层的稳定 ABI；std/core 下的 .myl 只允许
 * 依赖这些符号，**禁止任何操作系统调用**（文件 / 目录 / 进程 / 线程 /
 * socket / 环境变量）。带 OS 能力的 API 一律放在 std/ 层。
 *
 * 【线程模型】运行时单线程：语言层不提供 thread/spawn/锁，GC 只有一个堆
 * 与一份根集合。需要并发时先做协程（单线程调度），并行走 isolate。
 * ---------------------------------------------------------------------------
 * ==========================================================================*/
#ifndef MYLRT_H
#define MYLRT_H

/* u8[] 的生成类型；仅用于字节数组 ABI，结构定义由生成代码给出。 */
typedef struct MylA_au8 MylA_au8;

/* Mylis 基本类型：不依赖 <stdint.h>，用 C11 静态断言保证宽度。 */
typedef signed char        myrt_i8;
typedef short              myrt_i16;
typedef int                myrt_i32;
typedef long long          myrt_i64;
typedef unsigned char      myrt_u8;
typedef unsigned short     myrt_u16;
typedef unsigned int       myrt_u32;
typedef unsigned long long myrt_u64;
typedef int                myrt_bool;

_Static_assert(sizeof(myrt_i8) == 1, "myrt_i8 must be 8-bit");
_Static_assert(sizeof(myrt_i16) == 2, "myrt_i16 must be 16-bit");
_Static_assert(sizeof(myrt_i32) == 4, "myrt_i32 must be 32-bit");
_Static_assert(sizeof(myrt_i64) == 8, "myrt_i64 must be 64-bit");
_Static_assert(sizeof(myrt_u8) == 1, "myrt_u8 must be 8-bit");
_Static_assert(sizeof(myrt_u64) == 8, "myrt_u64 must be 64-bit");

/* Mylis string：GC 托管的不变字节串（UTF-8），data 恒以 '\0' 结尾。
 * 引用语义，永不拷贝（除显式构造）。 */
typedef struct MyStr MyStr;
struct MyStr {
    myrt_u64 len;
    char*    data;
};

#if defined(__GNUC__)
#define MYLRT_NORETURN __attribute__((noreturn))
#elif defined(_MSC_VER)
#define MYLRT_NORETURN __declspec(noreturn)
#else
#define MYLRT_NORETURN
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* ---------------------------------------------------------------- 生命周期 */
void myrt_init(void);          /* 编译器在 main 中自动调用 */
void myrt_gc_collect(void);
void myrt_gc_safepoint(void);
void myrt_gc_add_root_range(void* start, myrt_u64 bytes);

/* 每帧根表：编译器在安全点填入当前活跃根，运行时只扫描这份快照。 */
typedef myrt_u64 myrt_gc_frame_id;
typedef struct myrt_gc_root_desc {
    void* start;
    myrt_u64 bytes;
} myrt_gc_root_desc;
myrt_gc_frame_id myrt_gc_frame_enter(void);
void myrt_gc_frame_set_roots(myrt_gc_frame_id id, const myrt_gc_root_desc* entries, myrt_u64 count);
void myrt_gc_frame_exit(myrt_gc_frame_id id);
/* 静态字段根表：entries 必须长期有效（通常为全局数组）。 */
void myrt_gc_static_set_roots(const myrt_gc_root_desc* entries, myrt_u64 count);
void myrt_gc_remove_root_range(void* start);
void myrt_gc_set_precise(myrt_bool on);
/* 诊断：把 GC 统计（回收次数/存活字节/累计分配/阈值）打到 stderr。
 * 仅 unsafe 可用；无回收后端打印零值。 */
void myrt_gc_stats_dump(void);
/* 编译器内部使用：堆指针写入后的写屏障（增量 GC 需要；无增量后端为空实现） */
void myrt_gc_write_barrier(void* value);
void myrt_gc_write_barrier_owner(void* owner_slot, void* value);

/* ------------------------------------------------------- 第 11 章：内存 API */
char*    myrt_alloc(myrt_u64 size);
void     myrt_register_root(char* obj);
void     myrt_unregister_root(char* obj);

/* ------------------------------------------------- 第 11 章：字符串/IO API */
MyStr*    myrt_str_cstr(char* p);        /* C 字面串 -> Mylis string（复制） */
char*     myrt_str_ptr(MyStr* s);        /* Mylis string -> 以 \0 结尾的缓冲区  */
myrt_u64  myrt_str_len(MyStr* s);
void      myrt_print(MyStr* s);
void      myrt_println(MyStr* s);
void      myrt_panic(MyStr* msg);

/* ------------------------------------------------ 第 11 章：格式化 / 计时 */
MyStr*  myrt_i64_to_string(myrt_i64 v);
MyStr*  myrt_u64_to_string(myrt_u64 v);
MyStr*  myrt_f64_to_string(double v);
MyStr*  myrt_f64_to_string_prec(double v, myrt_i32 precision);
/* flags：1=左对齐，2=强制正号，4=零填充，8=正数前空格 */
MyStr*  myrt_f64_to_string_fmt(double v, myrt_i32 width, myrt_i32 precision, myrt_i32 flags);
double  myrt_clock(void);

/* ============================================================================
 * 编译器内部辅助符号（mylrt_ 前缀）：不对外暴露为语言 API
 * ==========================================================================*/
MyStr*  mylrt_str_lit(const char* s, myrt_u64 len);     /* 字面串驻留 */
MyStr*  mylrt_str_concat(MyStr* a, MyStr* b);
myrt_bool mylrt_str_eq(MyStr* a, MyStr* b);
myrt_i32  mylrt_str_cmp(MyStr* a, MyStr* b);
myrt_u64  mylrt_str_hash(MyStr* a);
myrt_u64  mylrt_hash_u64(myrt_u64 x);
myrt_u64  mylrt_hash_f64(double x);
myrt_u64  mylrt_str_char_len(MyStr* s);
myrt_i64  mylrt_str_char_index_of(MyStr* s, MyStr* needle);
MyStr*    mylrt_str_char_substring(MyStr* s, myrt_u64 start, myrt_u64 count);
MyStr*    mylrt_str_char_at(MyStr* s, myrt_u64 index);
myrt_u32  mylrt_str_char_code_at(MyStr* s, myrt_u64 index);
MylA_au8* mylrt_str_to_bytes(MyStr* s);
MyStr*    mylrt_str_from_bytes(MylA_au8* arr);
myrt_bool mylrt_str_starts_with(MyStr* s, MyStr* prefix);
myrt_bool mylrt_str_ends_with(MyStr* s, MyStr* suffix);
myrt_i64  mylrt_str_index_of(MyStr* s, MyStr* needle);
myrt_bool mylrt_str_contains(MyStr* s, MyStr* needle);
MyStr*    mylrt_str_substring(MyStr* s, myrt_u64 start, myrt_u64 length);
MyStr*    mylrt_str_trim(MyStr* s);
MyStr*    mylrt_str_repeat(MyStr* s, myrt_u64 count);
MyStr*    mylrt_str_replace(MyStr* s, MyStr* from, MyStr* to);
MyStr*  mylrt_str_from_buf(const char* s, myrt_u64 len); /* 无驻留分配 */
MYLRT_NORETURN void mylrt_panic_msg(const char* what);   /* 固定 ASCII 消息 panic */
MYLRT_NORETURN void mylrt_panic_unwrap(void);           /* unwrap 失败 */
void    mylrt_assert(myrt_bool cond, const char* what);

/* -------------------------------------------------- 数组（GC 托管，定长） */
/* 返回已清零的块，前 8 字节写入长度；elem_size 为元素字节数 */
void*    mylrt_arr_alloc(myrt_u64 elem_size, myrt_u64 len);
/* 越界检查：返回合法下标，否则 panic */
myrt_u64 mylrt_arr_index(myrt_u64 len, myrt_i64 idx);

#ifdef __cplusplus
}
#endif
#endif /* MYLRT_H */
