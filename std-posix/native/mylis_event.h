#ifndef MYLIS_EVENT_H
#define MYLIS_EVENT_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int64_t mylis_event_new(void);
int64_t mylis_event_add(int64_t handle, int64_t fd);
int64_t mylis_event_modify(int64_t handle, int64_t fd, int32_t write_enabled);
int64_t mylis_event_remove(int64_t handle, int64_t fd);
int32_t mylis_event_wait(int64_t handle, int32_t timeout_ms,
                         int64_t* fds, int64_t* revents, int32_t max);
int64_t mylis_event_close(int64_t handle);

#ifdef __cplusplus
}
#endif

#endif
