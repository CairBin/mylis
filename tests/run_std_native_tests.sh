#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"
export MYLIS_PATH="${MYLIS_PATH:-$ROOT:$ROOT/std/src}"
OUT="$ROOT/build/std-posix/native-tests"
mkdir -p "$OUT"
mkdir -p "$ROOT/build/tests"
rm -f "$ROOT/build/tests/file_api.txt" "$OUT/FileApi.actual"

bash tools/build_std_native.sh >/dev/null
LIB="$ROOT/build/std-posix/native/libmylis_file.a"
NET="$ROOT/build/std-posix/native/libmylis_net.a"
EVENT="$ROOT/build/std-posix/native/libmylis_event.a"
IO="$ROOT/build/std-posix/native/libmylis_io.a"
TIME="$ROOT/build/std-posix/native/libmylis_time.a"
PROCESS="$ROOT/build/std-posix/native/libmylis_process.a"

run_case() {
  local backend=$1
  local exe="$OUT/FileApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/FileApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$LIB" -o "$exe" >"$OUT/$backend.build.log" 2>&1; then
    echo "FAIL FileApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/FileApi.actual" 2>"$OUT/$backend.err"; then
    echo "FAIL FileApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/FileApi.expected "$OUT/FileApi.actual"; then
    echo "FAIL FileApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   FileApi（$backend）"
}

run_tcp_case() {
  local backend=$1
  local exe="$OUT/TcpApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/TcpApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$LIB" --link "$NET" -o "$exe" >"$OUT/tcp-$backend.build.log" 2>&1; then
    echo "FAIL TcpApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/tcp-$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/TcpApi.actual" 2>"$OUT/tcp-$backend.err"; then
    echo "FAIL TcpApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/tcp-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/TcpApi.expected "$OUT/TcpApi.actual"; then
    echo "FAIL TcpApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   TcpApi（$backend）"
}

run_event_case() {
  local backend=$1
  local exe="$OUT/EventLoopApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/EventLoopApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$LIB" --link "$NET" --link "$EVENT" -o "$exe" >"$OUT/event-$backend.build.log" 2>&1; then
    echo "FAIL EventLoopApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/event-$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/EventLoopApi.actual" 2>"$OUT/event-$backend.err"; then
    echo "FAIL EventLoopApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/event-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/EventLoopApi.expected "$OUT/EventLoopApi.actual"; then
    echo "FAIL EventLoopApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   EventLoopApi（$backend）"
}

run_file_ops_case() {
  local backend=$1
  local exe="$OUT/FileOpsApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/FileOpsApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$LIB" -o "$exe" >"$OUT/fileops-$backend.build.log" 2>&1; then
    echo "FAIL FileOpsApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/fileops-$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/FileOpsApi.actual" 2>"$OUT/fileops-$backend.err"; then
    echo "FAIL FileOpsApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/fileops-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/FileOpsApi.expected "$OUT/FileOpsApi.actual"; then
    echo "FAIL FileOpsApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   FileOpsApi（$backend）"
}

run_std_flag_case() {
  local exe="$OUT/FileApi_stdflag"
  if ! "$ROOT/build/mylc" --std --backend mir tests/std_native/FileApi.myl -o "$exe" >"$OUT/stdflag.build.log" 2>&1; then
    echo "FAIL FileApi（--std 编译失败）"
    sed -n '1,20p' "$OUT/stdflag.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/FileApi_stdflag.actual" 2>"$OUT/stdflag.err"; then
    echo "FAIL FileApi（--std 运行失败）"
    sed -n '1,10p' "$OUT/stdflag.err"
    return 1
  fi
  if ! diff -u tests/std_native/FileApi.expected "$OUT/FileApi_stdflag.actual"; then
    echo "FAIL FileApi（--std 输出不一致）"
    return 1
  fi
  echo "  ok   FileApi（--std 自动链接）"
}

run_std_flag_stdio_case() {
  local exe="$OUT/StdioApi_stdflag"
  if ! "$ROOT/build/mylc" --std --backend mir tests/std_native/StdioApi.myl -o "$exe" >"$OUT/stdflag_stdio.build.log" 2>&1; then
    echo "FAIL StdioApi（--std 编译失败）"
    sed -n '1,20p' "$OUT/stdflag_stdio.build.log"
    return 1
  fi
  if ! printf 'hello\nworld\n' | MYLIS_STDIO_TEST=ok "$exe" alpha beta >"$OUT/StdioApi_stdflag.actual" 2>"$OUT/stdflag_stdio.err"; then
    echo "FAIL StdioApi（--std 运行失败）"
    sed -n '1,10p' "$OUT/stdflag_stdio.err"
    return 1
  fi
  if ! diff -u tests/std_native/StdioApi.expected "$OUT/StdioApi_stdflag.actual"; then
    echo "FAIL StdioApi（--std 输出不一致）"
    return 1
  fi
  echo "  ok   StdioApi（--std 自动链接）"
}

run_platform_stdflag_case() {
  local exe="$OUT/PlatformApi_stdflag"
  if ! "$ROOT/build/mylc" --std --backend mir tests/std_native/PlatformApi.myl -o "$exe" >"$OUT/stdflag_platform.build.log" 2>&1; then
    echo "FAIL PlatformApi（--std 编译失败）"
    sed -n '1,20p' "$OUT/stdflag_platform.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/PlatformApi_stdflag.actual" 2>"$OUT/stdflag_platform.err"; then
    echo "FAIL PlatformApi（--std 运行失败）"
    sed -n '1,10p' "$OUT/stdflag_platform.err"
    return 1
  fi
  if ! diff -u tests/std_native/PlatformApi.expected "$OUT/PlatformApi_stdflag.actual"; then
    echo "FAIL PlatformApi（--std 输出不一致）"
    return 1
  fi
  echo "  ok   PlatformApi（--std 自动链接）"
}

run_auto_std_case() {
  local proj="$OUT/auto_std"
  rm -rf "$proj"
  mkdir -p "$proj/src"
  cat > "$proj/mylis.toml" <<'TOML'
[project]
name = "auto_std"
[build]
backend = "mir"
[dependencies]
std = { path = "../../../../std" }
[targets.app]
kind = "exe"
entry = "src/main.myl"
out = "build/app"
TOML
  cat > "$proj/src/Helper.myl" <<'MYL'
import { Path } from "std/Path";
import { File } from "std/File";
import { Io } from "std/Io";

pub class Helper {
    pub static fn Run() -> void {
        Io::Println(Path::Join("a", "b"));
        Io::PrintlnBool(File::Exists("build/std-posix/native-tests/auto_std/none.txt"));
    }
}
MYL
  cat > "$proj/src/main.myl" <<'MYL'
import { Helper } from "Helper.myl";

fn main() -> i32 {
    Helper::Run();
    return 0;
}
MYL
  if ! (cd "$proj" && "$ROOT/build/mylc" build >"$OUT/auto_std.build.log" 2>&1); then
    echo "FAIL AutoStd（编译失败）"
    sed -n '1,20p' "$OUT/auto_std.build.log"
    return 1
  fi
  if ! "$proj/build/app" >"$OUT/auto_std.actual" 2>"$OUT/auto_std.err"; then
    echo "FAIL AutoStd（运行失败）"
    sed -n '1,10p' "$OUT/auto_std.err"
    return 1
  fi
  if ! diff -u <(printf 'a/b\nfalse\n') "$OUT/auto_std.actual"; then
    echo "FAIL AutoStd（输出不一致）"
    return 1
  fi
  echo "  ok   AutoStd（清单依赖 std 自动启用）"
}

run_transitive_std_case() {
  local proj="$OUT/transitive_std"
  rm -rf "$proj"
  mkdir -p "$proj/app/src" "$proj/app/lib/helper/src"
  cat > "$proj/app/mylis.toml" <<'TOML'
[project]
name = "app"
[build]
backend = "mir"
[dependencies]
helper = { path = "lib/helper" }
[targets.app]
kind = "exe"
entry = "src/main.myl"
out = "build/app"
TOML
  cat > "$proj/app/lib/helper/mylis.toml" <<TOML
[package]
name = "helper"
version = "0.1.0"
[dependencies]
std = { path = "$ROOT/std" }
TOML
  cat > "$proj/app/lib/helper/src/Helper.myl" <<'MYL'
import { Path } from "std/Path";
import { File } from "std/File";

pub class Helper {
    pub static fn Run() -> void {
        string p = Path::Join("a", "b");
        File::Exists(p);
    }
}
MYL
  cat > "$proj/app/src/main.myl" <<'MYL'
import "helper/Helper";

fn main() -> i32 {
    helper::Helper::Run();
    return 0;
}
MYL
  if ! (cd "$proj/app" && "$ROOT/build/mylc" build >"$OUT/transitive_std.build.log" 2>&1); then
    echo "FAIL TransitiveStd（编译失败）"
    sed -n '1,20p' "$OUT/transitive_std.build.log"
    return 1
  fi
  echo "  ok   TransitiveStd（间接依赖 std 自动启用）"
}

run_file_dir_case() {
  local backend=$1
  local exe="$OUT/FileDirApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/FileDirApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$LIB" -o "$exe" >"$OUT/filedir-$backend.build.log" 2>&1; then
    echo "FAIL FileDirApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/filedir-$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/FileDirApi.actual" 2>"$OUT/filedir-$backend.err"; then
    echo "FAIL FileDirApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/filedir-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/FileDirApi.expected "$OUT/FileDirApi.actual"; then
    echo "FAIL FileDirApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   FileDirApi（$backend）"
}

run_stdio_case() {
  local backend=$1
  local exe="$OUT/StdioApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/StdioApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$IO" -o "$exe" >"$OUT/stdio-$backend.build.log" 2>&1; then
    echo "FAIL StdioApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/stdio-$backend.build.log"
    return 1
  fi
  if ! printf 'hello\nworld\n' | MYLIS_STDIO_TEST=ok "$exe" alpha beta >"$OUT/StdioApi.actual" 2>"$OUT/stdio-$backend.err"; then
    echo "FAIL StdioApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/stdio-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/StdioApi.expected "$OUT/StdioApi.actual"; then
    echo "FAIL StdioApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   StdioApi（$backend）"
}

run_std_case() {
  local name=$1
  local backend=$2
  local exe="$OUT/${name}_$backend"
  if ! "$ROOT/build/mylc" "tests/std_native/$name.myl" \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      -o "$exe" >"$OUT/$name-$backend.build.log" 2>&1; then
    echo "FAIL $name（$backend 编译失败）"
    sed -n '1,20p' "$OUT/$name-$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/$name.actual" 2>"$OUT/$name-$backend.err"; then
    echo "FAIL $name（$backend 运行失败）"
    sed -n '1,10p' "$OUT/$name-$backend.err"
    return 1
  fi
  if ! diff -u "tests/std_native/$name.expected" "$OUT/$name.actual"; then
    echo "FAIL $name（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   $name（$backend）"
}

run_process_case() {
  local name=$1
  local backend=$2
  local exe="$OUT/${name}_$backend"
  if ! "$ROOT/build/mylc" "tests/std_native/$name.myl" \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$PROCESS" -o "$exe" >"$OUT/$name-$backend.build.log" 2>&1; then
    echo "FAIL $name（$backend 编译失败）"
    sed -n '1,20p' "$OUT/$name-$backend.build.log"
    return 1
  fi
  if ! timeout 30 "$exe" >"$OUT/$name.actual" 2>"$OUT/$name-$backend.err"; then
    echo "FAIL $name（$backend 运行失败）"
    sed -n '1,10p' "$OUT/$name-$backend.err"
    return 1
  fi
  if ! diff -u "tests/std_native/$name.expected" "$OUT/$name.actual"; then
    echo "FAIL $name（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   $name（$backend）"
}

run_process_stdflag_case() {
  local exe="$OUT/ProcessApi_stdflag"
  if ! "$ROOT/build/mylc" --std --backend mir tests/std_native/ProcessApi.myl -o "$exe" >"$OUT/process-stdflag.build.log" 2>&1; then
    echo "FAIL ProcessApi（--std 编译失败）"
    sed -n '1,20p' "$OUT/process-stdflag.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/ProcessApi_stdflag.actual" 2>"$OUT/process-stdflag.err"; then
    echo "FAIL ProcessApi（--std 运行失败）"
    sed -n '1,10p' "$OUT/process-stdflag.err"
    return 1
  fi
  if ! diff -u tests/std_native/ProcessApi.expected "$OUT/ProcessApi_stdflag.actual"; then
    echo "FAIL ProcessApi（--std 输出不一致）"
    return 1
  fi
  echo "  ok   ProcessApi（--std 自动链接）"
}

run_worker_case() {
  local name=$1
  local backend=$2
  local helper_name=$3
  local helper="$OUT/$helper_name"
  local exe="$OUT/${name}_$backend"
  if ! "$ROOT/build/mylc" --std --backend mir "tests/std_native/$helper_name.myl" -o "$helper" >"$OUT/$helper_name.build.log" 2>&1; then
    echo "FAIL $helper_name（编译失败）"
    sed -n '1,20p' "$OUT/$helper_name.build.log"
    return 1
  fi
  if ! "$ROOT/build/mylc" "tests/std_native/$name.myl" \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$PROCESS" --link "$IO" -o "$exe" >"$OUT/worker-$name-$backend.build.log" 2>&1; then
    echo "FAIL $name（$backend 编译失败）"
    sed -n '1,20p' "$OUT/worker-$name-$backend.build.log"
    return 1
  fi
  if ! WORKER_CMD="$helper" timeout 30 "$exe" >"$OUT/$name.actual" 2>"$OUT/worker-$name-$backend.err"; then
    echo "FAIL $name（$backend 运行失败）"
    sed -n '1,10p' "$OUT/worker-$name-$backend.err"
    return 1
  fi
  if ! diff -u "tests/std_native/$name.expected" "$OUT/$name.actual"; then
    echo "FAIL $name（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   $name（$backend）"
}

run_platform_case() {
  local backend=$1
  local exe="$OUT/PlatformApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/PlatformApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$TIME" -o "$exe" >"$OUT/platform-$backend.build.log" 2>&1; then
    echo "FAIL PlatformApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/platform-$backend.build.log"
    return 1
  fi
  if ! "$exe" >"$OUT/PlatformApi.actual" 2>"$OUT/platform-$backend.err"; then
    echo "FAIL PlatformApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/platform-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/PlatformApi.expected "$OUT/PlatformApi.actual"; then
    echo "FAIL PlatformApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   PlatformApi（$backend）"
}

run_async_file_case() {
  local backend=$1
  local exe="$OUT/AsyncFileApi_$backend"
  if ! "$ROOT/build/mylc" --std --backend "$backend" tests/std_native/AsyncFileApi.myl -o "$exe" >"$OUT/async-file-$backend.build.log" 2>&1; then
    echo "FAIL AsyncFileApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/async-file-$backend.build.log"
    return 1
  fi
  if ! timeout 30 "$exe" >"$OUT/AsyncFileApi.actual" 2>"$OUT/async-file-$backend.err"; then
    echo "FAIL AsyncFileApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/async-file-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/AsyncFileApi.expected "$OUT/AsyncFileApi.actual"; then
    echo "FAIL AsyncFileApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   AsyncFileApi（$backend）"
}

run_timer_case() {
  local backend=$1
  local exe="$OUT/TimerApi_$backend"
  if ! "$ROOT/build/mylc" --std --backend "$backend" tests/std_native/TimerApi.myl -o "$exe" >"$OUT/timer-$backend.build.log" 2>&1; then
    echo "FAIL TimerApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/timer-$backend.build.log"
    return 1
  fi
  if ! timeout 30 "$exe" >"$OUT/TimerApi.actual" 2>"$OUT/timer-$backend.err"; then
    echo "FAIL TimerApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/timer-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/TimerApi.expected "$OUT/TimerApi.actual"; then
    echo "FAIL TimerApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   TimerApi（$backend）"
}

run_stdflag_named_case() {
  local name=$1
  local backend=$2
  local exe="$OUT/${name}_$backend"
  if ! "$ROOT/build/mylc" --std --backend "$backend" "tests/std_native/$name.myl" -o "$exe" >"$OUT/$name-$backend.build.log" 2>&1; then
    echo "FAIL $name（$backend 编译失败）"
    sed -n '1,20p' "$OUT/$name-$backend.build.log"
    return 1
  fi
  if ! timeout 30 "$exe" >"$OUT/$name.actual" 2>"$OUT/$name-$backend.err"; then
    echo "FAIL $name（$backend 运行失败）"
    sed -n '1,10p' "$OUT/$name-$backend.err"
    return 1
  fi
  if ! diff -u "tests/std_native/$name.expected" "$OUT/$name.actual"; then
    echo "FAIL $name（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   $name（$backend）"
}

run_case mir
run_case ast
run_std_flag_case
run_std_flag_stdio_case
run_platform_stdflag_case
run_auto_std_case
run_transitive_std_case
run_file_ops_case mir
run_file_ops_case ast
run_file_dir_case mir
run_file_dir_case ast
run_stdio_case mir
run_stdio_case ast
for name in ContainerIterApi PriorityQueueApi Containers2Api BTreeEdgeApi TextParseApi CsvApi CsvEdgeApi Base64Api Base64EdgeApi JsonApi JsonEdgeApi; do
  run_std_case "$name" mir
  run_std_case "$name" ast
done
run_process_case ProcessApi mir
run_process_case ProcessApi ast
run_process_case ProcessSpawnApi mir
run_process_case ProcessSpawnApi ast
run_process_case ProcessPopenApi mir
run_process_case ProcessPopenApi ast
run_process_case ProcessPopen2Api mir
run_process_case ProcessPopen2Api ast
run_process_case ProcessWriteTimeoutApi mir
run_process_case ProcessWriteTimeoutApi ast
run_process_case ProcessLineApi mir
run_process_case ProcessLineApi ast
run_process_case ProcessTimeoutApi mir
run_process_case ProcessTimeoutApi ast
run_process_case WorkerTimeoutApi mir
run_process_case WorkerTimeoutApi ast
run_process_case WorkerWriteTimeoutApi mir
run_process_case WorkerWriteTimeoutApi ast
run_process_case WorkerTerminateApi mir
run_process_case WorkerTerminateApi ast
run_process_case WorkerPoolTimeoutApi mir
run_process_case WorkerPoolTimeoutApi ast
run_process_case WorkerPoolWriteTimeoutApi mir
run_process_case WorkerPoolWriteTimeoutApi ast
run_process_stdflag_case
run_worker_case WorkerApi mir WorkerEcho
run_worker_case WorkerApi ast WorkerEcho
run_worker_case WorkerPoolApi mir WorkerEcho
run_worker_case WorkerPoolApi ast WorkerEcho
run_worker_case WorkerFlakyApi mir WorkerFlaky
run_worker_case WorkerFlakyApi ast WorkerFlaky
run_worker_case WorkerReplyApi mir WorkerResultServer
run_worker_case WorkerReplyApi ast WorkerResultServer
run_worker_case WorkerReplyInvalidApi mir WorkerReplyBadServer
run_worker_case WorkerReplyInvalidApi ast WorkerReplyBadServer
run_worker_case WorkerPoolReplyApi mir WorkerReplyMixedServer
run_worker_case WorkerPoolReplyApi ast WorkerReplyMixedServer
run_platform_case mir
run_platform_case ast

run_udp_case() {
  local backend=$1
  local exe="$OUT/UdpApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/UdpApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$NET" -o "$exe" >"$OUT/udp-$backend.build.log" 2>&1; then
    echo "FAIL UdpApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/udp-$backend.build.log"
    return 1
  fi
  if ! timeout 30 "$exe" >"$OUT/UdpApi.actual" 2>"$OUT/udp-$backend.err"; then
    echo "FAIL UdpApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/udp-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/UdpApi.expected "$OUT/UdpApi.actual"; then
    echo "FAIL UdpApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   UdpApi（$backend）"
}

run_udp_case mir
run_udp_case ast

run_udp_loop_case() {
  local backend=$1
  local exe="$OUT/UdpLoopApi_$backend"
  if ! "$ROOT/build/mylc" tests/std_native/UdpLoopApi.myl \
      --backend "$backend" \
      -I "$ROOT/std/src" -I "$ROOT/std-posix/native" \
      --link "$LIB" --link "$NET" --link "$EVENT" -o "$exe" >"$OUT/udploop-$backend.build.log" 2>&1; then
    echo "FAIL UdpLoopApi（$backend 编译失败）"
    sed -n '1,20p' "$OUT/udploop-$backend.build.log"
    return 1
  fi
  if ! timeout 30 "$exe" >"$OUT/UdpLoopApi.actual" 2>"$OUT/udploop-$backend.err"; then
    echo "FAIL UdpLoopApi（$backend 运行失败）"
    sed -n '1,10p' "$OUT/udploop-$backend.err"
    return 1
  fi
  if ! diff -u tests/std_native/UdpLoopApi.expected "$OUT/UdpLoopApi.actual"; then
    echo "FAIL UdpLoopApi（$backend 输出不一致）"
    return 1
  fi
  echo "  ok   UdpLoopApi（$backend）"
}

run_udp_loop_case mir
run_udp_loop_case ast
run_tcp_case mir
run_tcp_case ast
run_event_case mir
run_event_case ast

# 没有 clang 时仍可验证 LLVM IR 的外部符号声明；真实链接由有 clang 的环境负责。
if "$ROOT/build/mylc" tests/std_native/FileApi.myl --emit-llvm "$OUT/FileApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$LIB" >"$OUT/llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_file_open' "$OUT/FileApi.ll"; then
  echo "  ok   FileApi（LLVM 外部声明）"
else
  echo "FAIL FileApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/FileOpsApi.myl --emit-llvm "$OUT/FileOpsApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$LIB" >"$OUT/fileops-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_file_exists' "$OUT/FileOpsApi.ll"; then
  echo "  ok   FileOpsApi（LLVM 外部声明）"
else
  echo "FAIL FileOpsApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/fileops-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/UdpApi.myl --emit-llvm "$OUT/UdpApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$NET" >"$OUT/udp-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_udp_bind' "$OUT/UdpApi.ll"; then
  echo "  ok   UdpApi（LLVM 外部声明）"
else
  echo "FAIL UdpApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/udp-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/UdpLoopApi.myl --emit-llvm "$OUT/UdpLoopApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$NET" --link "$EVENT" >"$OUT/udploop-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_udp_fd' "$OUT/UdpLoopApi.ll"; then
  echo "  ok   UdpLoopApi（LLVM 外部声明）"
else
  echo "FAIL UdpLoopApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/udploop-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/TcpApi.myl --emit-llvm "$OUT/TcpApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$LIB" --link "$NET" >"$OUT/tcp-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_tcp_bind' "$OUT/TcpApi.ll"; then
  echo "  ok   TcpApi（LLVM 外部声明）"
else
  echo "FAIL TcpApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/tcp-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/EventLoopApi.myl --emit-llvm "$OUT/EventLoopApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$LIB" --link "$NET" --link "$EVENT" >"$OUT/event-llvm.emit.log" 2>&1 \
    && grep -q 'declare i32 @mylis_event_wait' "$OUT/EventLoopApi.ll"; then
  echo "  ok   EventLoopApi（LLVM 外部声明）"
else
  echo "FAIL EventLoopApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/event-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/ProcessApi.myl --emit-llvm "$OUT/ProcessApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$PROCESS" >"$OUT/process-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_process_run' "$OUT/ProcessApi.ll"; then
  echo "  ok   ProcessApi（LLVM 外部声明）"
else
  echo "FAIL ProcessApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/process-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/ProcessSpawnApi.myl --emit-llvm "$OUT/ProcessSpawnApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$PROCESS" >"$OUT/process-spawn-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_process_spawn' "$OUT/ProcessSpawnApi.ll"; then
  echo "  ok   ProcessSpawnApi（LLVM 外部声明）"
else
  echo "FAIL ProcessSpawnApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/process-spawn-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/ProcessPopenApi.myl --emit-llvm "$OUT/ProcessPopenApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$PROCESS" >"$OUT/process-popen-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_process_popen' "$OUT/ProcessPopenApi.ll"; then
  echo "  ok   ProcessPopenApi（LLVM 外部声明）"
else
  echo "FAIL ProcessPopenApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/process-popen-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/WorkerApi.myl --emit-llvm "$OUT/WorkerApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$PROCESS" --link "$IO" >"$OUT/worker-llvm.emit.log" 2>&1 \
    && grep -q 'declare ptr @mylis_process_pipe_read_line' "$OUT/WorkerApi.ll" \
    && grep -q 'declare ptr @mylis_process_pipe_read_line_timeout' "$OUT/WorkerApi.ll"; then
  echo "  ok   WorkerApi（LLVM 外部声明）"
else
  echo "FAIL WorkerApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/worker-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/WorkerTerminateApi.myl --emit-llvm "$OUT/WorkerTerminateApi.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$PROCESS" >"$OUT/worker-terminate-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_process_pipe_terminate' "$OUT/WorkerTerminateApi.ll"; then
  echo "  ok   WorkerTerminateApi（LLVM 外部声明）"
else
  echo "FAIL WorkerTerminateApi（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/worker-terminate-llvm.emit.log"
  exit 1
fi

if "$ROOT/build/mylc" tests/std_native/ProcessPopen2Api.myl --emit-llvm "$OUT/ProcessPopen2Api.ll" \
    -I "$ROOT/std/src" -I "$ROOT/std-posix/native" --link "$PROCESS" >"$OUT/process-popen2-llvm.emit.log" 2>&1 \
    && grep -q 'declare i64 @mylis_process_popen2' "$OUT/ProcessPopen2Api.ll"; then
  echo "  ok   ProcessPopen2Api（LLVM 外部声明）"
else
  echo "FAIL ProcessPopen2Api（LLVM 外部声明）"
  sed -n '1,12p' "$OUT/process-popen2-llvm.emit.log"
  exit 1
fi
run_async_file_case mir
run_async_file_case ast
run_timer_case mir
run_timer_case ast
run_stdflag_named_case BufferedTcpApi mir
run_stdflag_named_case BufferedTcpApi ast
run_stdflag_named_case UnixSocketApi mir
run_stdflag_named_case UnixSocketApi ast
run_stdflag_named_case UnixEventLoopApi mir
run_stdflag_named_case UnixEventLoopApi ast
run_stdflag_named_case TcpReuseApi mir
run_stdflag_named_case TcpReuseApi ast
run_stdflag_named_case FilePermApi mir
run_stdflag_named_case FilePermApi ast
run_stdflag_named_case DateTimeApi mir
run_stdflag_named_case DateTimeApi ast
run_stdflag_named_case RegexApi mir
run_stdflag_named_case RegexApi ast
run_stdflag_named_case RegexRefApi mir
run_stdflag_named_case RegexRefApi ast
run_stdflag_named_case RegexFlagsApi mir
run_stdflag_named_case RegexFlagsApi ast
run_stdflag_named_case RegexUnicodeApi mir
run_stdflag_named_case RegexUnicodeApi ast
run_stdflag_named_case RegexUnicodeClassApi mir
run_stdflag_named_case RegexUnicodeClassApi ast
run_stdflag_named_case RegexUnicodeWordApi mir
run_stdflag_named_case RegexUnicodeWordApi ast
run_stdflag_named_case HashApi mir
run_stdflag_named_case HashApi ast
run_stdflag_named_case GzipApi mir
run_stdflag_named_case GzipApi ast

echo "std native 测试通过"
