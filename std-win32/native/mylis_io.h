#ifndef MYLIS_IO_H
#define MYLIS_IO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Read one line from stdin (newline stripped); at EOF/error returns an empty
   string, with mylis_io_last_status() giving the reason.
   The returned buffer must be released with mylis_io_free_string. */
char* mylis_io_read_line(void);

/* Read the rest of stdin; returns a malloc-compatible string (may be empty).
   The returned buffer must be released with mylis_io_free_string. */
char* mylis_io_read_all(void);

void  mylis_io_free_string(char* text);

/* Status of the last read_line/read_all call: 1 = success, 0 = EOF/no input,
   negative = read error. */
int64_t mylis_io_last_status(void);

int64_t mylis_io_arg_count(void);

/* 0 is argv[0], i.e. the program name. Returns NULL when out of range. */
char* mylis_io_arg(int64_t index);

/* Returns 1 when the variable exists (even with an empty value), otherwise 0. */
int64_t mylis_io_env_exists(const char* key);

/* Returns an empty string when the variable is unset. */
char* mylis_io_env_get(const char* key);

void  mylis_io_exit(int32_t code);

#ifdef __cplusplus
}
#endif

#endif
