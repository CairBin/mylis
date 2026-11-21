#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/build/std-posix/native"
mkdir -p "$OUT"
RUSTC=${RUSTC:-rustc}
if ! command -v "$RUSTC" >/dev/null 2>&1; then
  if [ -x "$HOME/.cargo/bin/rustc" ]; then
    RUSTC="$HOME/.cargo/bin/rustc"
  else
    echo "缺少 rustc；无法构建 std native 后端" >&2
    exit 1
  fi
fi
"$RUSTC" --edition 2021 --crate-name mylis_file --crate-type staticlib \
  -C panic=abort -C opt-level=3 \
  "$ROOT/std-posix/native/file.rs" -o "$OUT/libmylis_file.a"
"$RUSTC" --edition 2021 --crate-name mylis_net --crate-type staticlib \
  -C panic=abort -C opt-level=3 \
  "$ROOT/std-posix/native/net.rs" -o "$OUT/libmylis_net.a"
"$RUSTC" --edition 2021 --crate-name mylis_event --crate-type staticlib \
  -C panic=abort -C opt-level=3 \
  "$ROOT/std-posix/native/event.rs" -o "$OUT/libmylis_event.a"
"$RUSTC" --edition 2021 --crate-name mylis_io --crate-type staticlib \
  -C panic=abort -C opt-level=3 \
  "$ROOT/std-posix/native/io.rs" -o "$OUT/libmylis_io.a"
"$RUSTC" --edition 2021 --crate-name mylis_time --crate-type staticlib \
  -C panic=abort -C opt-level=3 \
  "$ROOT/std-posix/native/time.rs" -o "$OUT/libmylis_time.a"
"$RUSTC" --edition 2021 --crate-name mylis_process --crate-type staticlib \
  -C panic=abort -C opt-level=3 \
  "$ROOT/std-posix/native/process.rs" -o "$OUT/libmylis_process.a"
echo "已生成 $OUT/libmylis_file.a $OUT/libmylis_io.a $OUT/libmylis_time.a $OUT/libmylis_net.a $OUT/libmylis_event.a $OUT/libmylis_process.a"
