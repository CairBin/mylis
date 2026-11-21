#!/usr/bin/env bash
# 产物形态与外部库验收：
#   1) --static 产出 .a + .h，C 客户端可链接运行
#   2) --shared 产出 .so/.dll + .h，导出符号可被 C 客户端链接
#   3) 消费外部静态库（-L + -l）
#   4) 消费外部动态库（-L + -l，运行时 rpath）
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC="$ROOT/build/mylc"
GC=${MYLIS_GC_ROOT:-$HOME/mylis-toolchain}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
cp "$ROOT"/tests/lib/mathlib.myl .

CC=${MYLIS_CC:-gcc}
INC="-I$ROOT/myrt -I$GC/include"

echo "== 1/4 静态库 + C 客户端"
"$MYLC" --backend mir --static mathlib.myl -o libmathlib.a
cp "$ROOT"/tests/lib/client.c .
"$CC" -std=c11 -O2 $INC -I. client.c libmathlib.a \
    "$ROOT/build/libmylrt.a" "$ROOT/build/libgc_boehm.a" "$GC/lib/libgc.a" -lm -lpthread -ldl -o client_static
./client_static > static.out
grep -q "add=5 scaled=12" static.out && echo "  ok 静态库链接"

echo "== 2/4 动态库 + C 客户端"
"$MYLC" --backend mir --shared mathlib.myl -o libmathlib.so
"$CC" -std=c11 -O2 $INC -I. client.c -L. -lmathlib \
    "$ROOT/build/libmylrt.a" "$ROOT/build/libgc_boehm.a" "$GC/lib/libgc.a" -Wl,-rpath,"$WORK" -o client_shared
./client_shared > shared.out
grep -q "add=5 scaled=12" shared.out && echo "  ok 动态库链接"

echo "== 3/4 消费外部静态库"
cat > cadd.c <<'C'
int c_add(int a, int b) { return a + b; }
int c_mul(int a, int b) { return a * b; }
C
"$CC" -c cadd.c -o cadd.o && ar rcs libcadd.a cadd.o
cat > cadd_bind.myl <<'B'
pub unsafe fn c_add(i32 a, i32 b) -> i32;
pub unsafe fn c_mul(i32 a, i32 b) -> i32;
B
cat > use_static.myl <<'M'
import { println_i32 } from "Io.myl";
import { c_add, c_mul } from "cadd_bind.myl";

pub fn main() -> i32 {
    unsafe {
        println_i32(c_add(7, 5));
        println_i32(c_mul(7, 5));
    }
    return 0;
}
M
"$MYLC" --backend mir use_static.myl -L . -l cadd -o use_static
./use_static > use_static.out
printf '12\n35\n' | diff -u - use_static.out && echo "  ok 外部静态库"

echo "== 4/4 消费外部动态库"
"$CC" -fPIC -shared cadd.c -o libcadddyn.so
"$MYLC" --backend mir use_static.myl -L . -l cadddyn -o use_dyn
LD_LIBRARY_PATH="$WORK" ./use_dyn > use_dyn.out
printf '12\n35\n' | diff -u - use_dyn.out && echo "  ok 外部动态库"
echo "产物形态验收通过"
