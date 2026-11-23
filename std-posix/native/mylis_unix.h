#ifndef MYLIS_UNIX_H
#define MYLIS_UNIX_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Unix domain socket (POSIX / Windows AF_UNIX) */

int64_t mylis_unix_bind(const char* path);
int64_t mylis_unix_accept(int64_t handle);
int64_t mylis_unix_connect(const char* path);
int64_t mylis_unix_listener_fd(int64_t handle);
int64_t mylis_unix_stream_fd(int64_t handle);
int64_t mylis_unix_listener_close(int64_t handle);
int64_t mylis_unix_stream_close(int64_t handle);
int64_t mylis_unix_listener_set_nonblocking(int64_t handle, int32_t enabled);
int64_t mylis_unix_stream_set_nonblocking(int64_t handle, int32_t enabled);
int64_t mylis_unix_stream_set_timeout(int64_t handle, int64_t milliseconds);
int64_t mylis_unix_stream_read(int64_t handle, void* data, int64_t offset, int64_t count);
int64_t mylis_unix_stream_write(int64_t handle, const void* data, int64_t offset, int64_t count);
int64_t mylis_unix_remove(const char* path);

#ifdef __cplusplus
}
#endif

#endif
