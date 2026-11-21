#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=${TARGET:-}
RUSTC=${RUSTC:-rustc}
if ! command -v "$RUSTC" >/dev/null 2>&1; then
  if [ -x "$HOME/.cargo/bin/rustc" ]; then
    RUSTC="$HOME/.cargo/bin/rustc"
  else
    echo "缺少 rustc；无法构建 std native 后端" >&2
    exit 1
  fi
fi

if [ -n "$TARGET" ] && [[ "$TARGET" == *windows* ]]; then
  OUT="$ROOT/build/std-win32/native"
  SRC="$ROOT/std-win32/native"
  EXT="lib"
  CRATE_PREFIX=""
else
  OUT="$ROOT/build/std-posix/native"
  SRC="$ROOT/std-posix/native"
  EXT="a"
  CRATE_PREFIX="lib"
fi
mkdir -p "$OUT"

TARGET_ARGS=()
if [ -n "$TARGET" ]; then
  TARGET_ARGS=(--target "$TARGET")
fi

for name in file net event io time process; do
  "$RUSTC" --edition 2021 --crate-name "mylis_$name" --crate-type staticlib \
    -C panic=abort -C opt-level=3 \
    "${TARGET_ARGS[@]}" \
    "$SRC/$name.rs" -o "$OUT/${CRATE_PREFIX}mylis_$name.$EXT"
done
echo "已生成 $OUT/${CRATE_PREFIX}mylis_{file,io,time,net,event,process}.$EXT"
