#ifndef MYLIS_PROCESS_H
#define MYLIS_PROCESS_H

#include <stdint.h>

/*
 * run 同步执行 shell 命令并捕获输出；spawn 返回子进程句柄。
 * run 返回 0 表示命令已启动并结束，负数为 -errno。
 * Run 的退出码/信号和输出保存在“最后一次 run”中；Spawn 的句柄用 wait/kill/close 管理。
 * Popen/Popen2 的句柄用 read_all/read_copy/write/close_input/close 管理；Popen2 同时有 stdin/stdout。
 * last_stdout/last_stderr 返回新分配的 C 字符串，用 free_string 释放。
 */
int64_t mylis_process_run(const char* command);
int64_t mylis_process_spawn(const char* command);
int64_t mylis_process_popen(const char* command, const char* mode);
int64_t mylis_process_popen2(const char* command);
int64_t mylis_process_pid(int64_t handle);
int64_t mylis_process_wait(int64_t handle);
int64_t mylis_process_last_wait_exit_code(void);
int64_t mylis_process_last_wait_signal(void);
int64_t mylis_process_kill(int64_t handle);
int64_t mylis_process_close(int64_t handle);
int64_t mylis_process_pipe_pid(int64_t handle);
int64_t mylis_process_pipe_read_all(int64_t handle);
char*   mylis_process_pipe_read_line(int64_t handle);
char*   mylis_process_pipe_read_line_timeout(int64_t handle, int64_t timeout_ms);
int64_t mylis_process_pipe_last_status(void);
int64_t mylis_process_pipe_read_copy(int64_t handle, void* data, int64_t offset, int64_t count);
int64_t mylis_process_pipe_write(int64_t handle, const void* data, int64_t offset, int64_t count);
int64_t mylis_process_pipe_close_input(int64_t handle);
int64_t mylis_process_pipe_close(int64_t handle);
int64_t mylis_process_pipe_terminate(int64_t handle);
int64_t mylis_process_last_exit_code(void);
int64_t mylis_process_last_signal(void);
char* mylis_process_last_stdout(void);
char* mylis_process_last_stderr(void);
void mylis_process_free_string(char* text);

#endif
