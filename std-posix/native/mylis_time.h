#ifndef MYLIS_TIME_H
#define MYLIS_TIME_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int64_t mylis_time_now_millis(void);
int64_t mylis_time_now_nanos(void);
int64_t mylis_time_monotonic_millis(void);
double  mylis_time_now_seconds(void);
void    mylis_time_sleep_millis(int64_t milliseconds);

#ifdef __cplusplus
}
#endif

#endif
