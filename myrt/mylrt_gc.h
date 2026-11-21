/* ============================================================================
 * mylrt_gc.h —— GC 后端抽象接口
 *
 * mylrt 只通过这四个函数分配/回收/注册根部，因此替换 GC 后端（自研 VM、
 * 引用计数、区域分配、无回收的 malloc 等）不需要改动 myrt_* 的 ABI。
 *
 * 默认后端：Boehm（myrt/gc_boehm.c）
 * 可选后端：simple（myrt/gc_simple.c，malloc + 不回收，供自研 VM 起步）
 * 构建期选择：`make GC_BACKEND=simple`
 * ==========================================================================*/
#ifndef MYLRT_GC_H
#define MYLRT_GC_H

#include <stddef.h>

/* 初始化（幂等；在 main 进入用户代码前调用） */
void myrt_gc_backend_init(void);

/* 分配 n 字节（n=0 时也必须返回有效指针）；失败直接 abort */
void* myrt_gc_backend_alloc(size_t n);

/* 主动触发一次回收（无回收的后端可为空实现） */
void myrt_gc_backend_collect(void);
void myrt_gc_backend_safepoint(void);
unsigned long long myrt_gc_backend_frame_enter(void);
void myrt_gc_backend_frame_set_roots(unsigned long long id, const void* entries, unsigned long long count);
void myrt_gc_backend_frame_exit(unsigned long long id);
void myrt_gc_backend_static_set_roots(const void* entries, unsigned long long count);
void myrt_gc_backend_remove_root_range(void* start);
void myrt_gc_backend_set_precise(int on);

/* 把一个静态内存区间登记为根（无回收后端可为空实现） */
void myrt_gc_backend_add_root_range(void* start, size_t bytes);

/* 可替换底层分配器：宿主默认 libc malloc/free；裸机可传静态 arena 的分配函数。
 * 必须在首次 alloc 之前调用。传 NULL 恢复默认。 */
typedef void* (*myrt_gc_alloc_fn)(size_t);
typedef void (*myrt_gc_free_fn)(void*);
void myrt_gc_backend_set_allocator(myrt_gc_alloc_fn a, myrt_gc_free_fn f);

/* 显式指定栈范围（裸机从链接脚本 _estack/_sstack 传入）。
 * hi 为高地址栈顶；lo 目前仅作记录，扫描仍从当前 sp 到 hi。 */
void myrt_gc_backend_set_stack_range(void* lo, void* hi);

/* 批量注册根区间，返回成功登记的数量 */
typedef struct { void* start; size_t bytes; } MylRootRange;
size_t myrt_gc_backend_add_root_ranges(const MylRootRange* ranges, size_t n);

/* 写屏障：增量标记期间，堆对象里存入新指针时调用（其他后端为空实现）。
 * 只传被写入的值；标记器负责判断它是否指向未标记的堆块。 */
void myrt_gc_backend_write_barrier(void* value);
void myrt_gc_backend_write_barrier_owner(void* owner_slot, void* value);

/* 自动回收阈值（字节）；0 = 关闭自动回收（仍可显式 collect）。
 * 自研 GC 用；其他后端可为空实现。 */
void myrt_gc_backend_set_threshold(size_t bytes);

#endif /* MYLRT_GC_H */
