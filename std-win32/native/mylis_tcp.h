#ifndef MYLIS_TCP_H
#define MYLIS_TCP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* TCP + TCP socket options */

int64_t mylis_tcp_bind(const char* address);
int64_t mylis_tcp_bind_reuse(const char* address);
int64_t mylis_tcp_listener_port(int64_t handle);
int64_t mylis_tcp_accept(int64_t handle);
int64_t mylis_tcp_connect(const char* address);
int64_t mylis_tcp_listener_close(int64_t handle);
int64_t mylis_tcp_stream_close(int64_t handle);
int64_t mylis_tcp_stream_fd(int64_t handle);
int64_t mylis_tcp_listener_fd(int64_t handle);
int64_t mylis_tcp_listener_set_nonblocking(int64_t handle, int32_t enabled);
int64_t mylis_tcp_stream_set_nonblocking(int64_t handle, int32_t enabled);
int64_t mylis_tcp_stream_set_timeout(int64_t handle, int64_t milliseconds);
int64_t mylis_tcp_stream_read(int64_t handle, void* data, int64_t offset, int64_t count);
int64_t mylis_tcp_stream_write(int64_t handle, const void* data, int64_t offset, int64_t count);
int64_t mylis_tcp_stream_set_nodelay(int64_t handle, int32_t enabled);
int64_t mylis_tcp_stream_set_keepalive(int64_t handle, int32_t enabled);
int64_t mylis_tcp_stream_set_recv_buffer(int64_t handle, int32_t bytes);
int64_t mylis_tcp_stream_set_send_buffer(int64_t handle, int32_t bytes);
int64_t mylis_tcp_listener_set_reuse_address(int64_t handle, int32_t enabled);

#ifdef __cplusplus
}
#endif

#endif
