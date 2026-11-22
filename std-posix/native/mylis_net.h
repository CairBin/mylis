#ifndef MYLIS_NET_H
#define MYLIS_NET_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

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

int64_t mylis_udp_bind(const char* address);
int64_t mylis_udp_port(int64_t handle);
int64_t mylis_udp_send(int64_t handle, const char* address, const void* data, int64_t offset, int64_t count);
int64_t mylis_udp_recv(int64_t handle, void* data, int64_t offset, int64_t count);
int64_t mylis_udp_recv_from(int64_t handle, void* data, int64_t offset, int64_t count,
                            void* from, int64_t* from_meta);
int64_t mylis_udp_set_nonblocking(int64_t handle, int32_t enabled);
int64_t mylis_udp_fd(int64_t handle);
int64_t mylis_udp_close(int64_t handle);

/* socket options */
int64_t mylis_tcp_stream_set_nodelay(int64_t handle, int32_t enabled);
int64_t mylis_tcp_stream_set_keepalive(int64_t handle, int32_t enabled);
int64_t mylis_tcp_stream_set_recv_buffer(int64_t handle, int32_t bytes);
int64_t mylis_tcp_stream_set_send_buffer(int64_t handle, int32_t bytes);
int64_t mylis_tcp_listener_set_reuse_address(int64_t handle, int32_t enabled);
int64_t mylis_udp_set_reuse_address(int64_t handle, int32_t enabled);
int64_t mylis_udp_set_broadcast(int64_t handle, int32_t enabled);
int64_t mylis_udp_set_ttl(int64_t handle, int32_t ttl);

/* Unix domain socket (POSIX; Windows returns -95) */
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
