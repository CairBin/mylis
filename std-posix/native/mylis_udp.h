#ifndef MYLIS_UDP_H
#define MYLIS_UDP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* UDP + UDP socket options */

int64_t mylis_udp_bind(const char* address);
int64_t mylis_udp_port(int64_t handle);
int64_t mylis_udp_send(int64_t handle, const char* address, const void* data, int64_t offset, int64_t count);
int64_t mylis_udp_recv(int64_t handle, void* data, int64_t offset, int64_t count);
int64_t mylis_udp_recv_from(int64_t handle, void* data, int64_t offset, int64_t count,
                            void* from, int64_t* from_meta);
int64_t mylis_udp_set_nonblocking(int64_t handle, int32_t enabled);
int64_t mylis_udp_fd(int64_t handle);
int64_t mylis_udp_close(int64_t handle);
int64_t mylis_udp_set_reuse_address(int64_t handle, int32_t enabled);
int64_t mylis_udp_set_broadcast(int64_t handle, int32_t enabled);
int64_t mylis_udp_set_ttl(int64_t handle, int32_t ttl);

#ifdef __cplusplus
}
#endif

#endif
