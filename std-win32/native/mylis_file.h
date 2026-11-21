#ifndef MYLIS_FILE_H
#define MYLIS_FILE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* mode: 0 = read, 1 = write/create/truncate. 返回句柄，失败返回负 errno。 */
int64_t mylis_file_open(const char* path, int32_t mode);
int64_t mylis_file_close(int64_t handle);
int64_t mylis_file_exists(const char* path);
int64_t mylis_file_remove(const char* path);
int64_t mylis_file_rename(const char* from, const char* to);
int64_t mylis_file_size(int64_t handle);
int64_t mylis_file_read(int64_t handle, void* data, int64_t offset, int64_t count);
int64_t mylis_file_write(int64_t handle, const void* data, int64_t offset, int64_t count);
int64_t mylis_file_seek(int64_t handle, int64_t offset);
int64_t mylis_file_flush(int64_t handle);

int64_t mylis_file_is_dir(const char* path);
int64_t mylis_file_mkdir(const char* path);
int64_t mylis_file_mkdir_all(const char* path);
int64_t mylis_file_remove_dir(const char* path);
int64_t mylis_file_remove_dir_all(const char* path);
int64_t mylis_file_metadata_size(const char* path);
int64_t mylis_file_read_dir_count(const char* path);
char*   mylis_file_read_dir_entry(const char* path, int64_t index);
void    mylis_file_free_string(char* text);
int64_t mylis_file_tell(int64_t handle);

#ifdef __cplusplus
}
#endif

#endif
