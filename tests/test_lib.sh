#!/usr/bin/env bash
# 库模式验收：编译静态库 → 用 C 客户端链接 → 运行比对输出
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC="$ROOT/build/mylc"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
export PATH="$HOME/.cargo/bin:$PATH"
GC=${MYLIS_GC_ROOT:-$HOME/mylis-toolchain}

cd "$WORK"
cp "$ROOT"/tests/lib/* .

echo "== 1/3 编译库"
"$MYLC" --backend mir --lib mathlib.myl -o libmathlib.a

echo "== 2/3 链接 C 客户端"
if [ -n "${MYLIS_WIN_CC:-}" ]; then
    "$MYLIS_WIN_CC" -std=c11 -O2 -I"$ROOT/myrt" -I"$GC/include" -I. \
        client.c libmathlib.a "$ROOT/build/libmylrt.a" "$ROOT/build/libgc_boehm.a" "$GC/lib/libgc.a" \
        -static -static-libgcc -o client.exe
else
    gcc -std=c11 -O2 -I"$ROOT/myrt" -I"$GC/include" -I. \
        client.c libmathlib.a "$ROOT/build/libmylrt.a" "$ROOT/build/libgc_boehm.a" "$GC/lib/libgc.a" \
        -lm -lpthread -ldl -o client
fi

echo "== 3/3 运行"
if [ -x client ]; then ./client; else WINEDEBUG=-all /usr/lib/wine/wine64 client.exe | tr -d '\r'; fi
echo "库模式验收通过"
