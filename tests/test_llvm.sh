#!/usr/bin/env bash
# LLVM 原生后端验收（标量、字符串、类、数组、Option/Result）。
# 需要 LLVM 15+（opaque pointers）；没有则跳过，不阻塞其他后端。
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC=${MYLC:-$ROOT/build/mylc}
cd "$ROOT"

CC=""
for c in clang-20 clang-19 clang-18 clang-17 clang-16 clang-15 clang; do
    if command -v "$c" >/dev/null 2>&1; then CC=$c; break; fi
done
if [ -z "$CC" ]; then
    echo "LLVM 后端验收跳过：未找到 clang"
    exit 0
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0

for name in Arith Loops Basics Print GcAuto Objects Arrays Const Ctor2 Account Registry Results NestedOption Match Enums Enum AnyCast ComboIface IfaceAdapter ResultIface NestedIfaceOption OptionTry OptionArray MutualRec Generics GenericEnum GenericBound GenericTemplateBound Inference GenericReceiver VecApi; do
    src="tests/positive/$name.myl"
    out="$TMP/$name"
    if "$MYLC" --backend llvm --cc "$CC" "$src" -o "$out" >"$TMP/log" 2>&1 \
        && "$out" >"$TMP/out" 2>/dev/null \
        && diff -q "tests/positive/$name.expected" "$TMP/out" >/dev/null; then
        PASS=$((PASS+1))
    else
        echo "  FAIL $name"
        sed -n '1,5p' "$TMP/log"
        FAIL=$((FAIL+1))
    fi
done



# 库模式：静态库/动态库 + 头文件 + C 客户端链接运行
RT="$ROOT/build/libmylrt.a"
GCROOT=${MYLIS_GC_ROOT:-$HOME/mylis-toolchain}
GCINC="$GCROOT/include"
GCLIB="$GCROOT/lib/libgc.a"
BOEHM="$ROOT/build/libgc_boehm.a"
CINC="-I$ROOT/myrt -I$GCINC -I$ROOT/tests/lib"
for mode in static shared; do
    d="$TMP/$mode"; mkdir -p "$d"
    if [ "$mode" = static ]; then out="$d/libmathlib.a"; else out="$d/libmathlib.so"; fi
    ok=1
    "$MYLC" --backend llvm --cc "$CC" --$mode tests/lib/mathlib.myl -o "$out" >"$TMP/lib.log" 2>&1 || ok=0
    [ -f "$out" ] || ok=0
    [ -f "$d/libmathlib.h" ] || ok=0
    if [ $ok -eq 1 ] && [ -f "$RT" ] && [ -f "$GCLIB" ] && [ -f "$BOEHM" ]; then
        cp tests/lib/client.c "$d/"
        if [ "$mode" = static ]; then
            "$CC" -std=c11 -O2 $CINC -I"$d" "$d/client.c" "$out" "$RT" "$BOEHM" "$GCLIB" -lm -lpthread -ldl -o "$d/client" >>"$TMP/lib.log" 2>&1 || ok=0
        else
            "$CC" -std=c11 -O2 $CINC -I"$d" "$d/client.c" -L"$d" -lmathlib "$RT" "$BOEHM" "$GCLIB" -Wl,-rpath,"$d" -lm -lpthread -ldl -o "$d/client" >>"$TMP/lib.log" 2>&1 || ok=0
        fi
        [ $ok -eq 1 ] && { "$d/client" >"$d/out" 2>/dev/null || ok=0; }
        [ $ok -eq 1 ] && ! grep -q "add=5 scaled=12 greet=hello, library!" "$d/out" && ok=0
    fi
    if [ $ok -eq 1 ]; then PASS=$((PASS+1)); else
        echo "  FAIL lib-$mode"
        sed -n '1,8p' "$TMP/lib.log"
        FAIL=$((FAIL+1))
    fi
done

# Option/Result 小聚合库导出 ABI
d="$TMP/optlib"; mkdir -p "$d"
ok=1
"$MYLC" --backend llvm --cc "$CC" --static tests/lib/optlib.myl -o "$d/liboptlib.a" >"$TMP/optlib.log" 2>&1 || ok=0
[ -f "$d/liboptlib.h" ] || ok=0
if [ $ok -eq 1 ]; then
    "$CC" -std=c11 -O2 $CINC -I"$d" tests/lib/opt_client.c "$d/liboptlib.a" "$RT" "$ROOT/build/libmylgc.a" -lm -lpthread -ldl -o "$d/client" >>"$TMP/optlib.log" 2>&1 || ok=0
fi
[ $ok -eq 1 ] && { "$d/client" >"$d/out" 2>/dev/null || ok=0; }
[ $ok -eq 1 ] && ! grep -q "a=1,7" "$d/out" && ok=0
[ $ok -eq 1 ] && ! grep -q "b=0" "$d/out" && ok=0
[ $ok -eq 1 ] && ! grep -q "n=1,ok" "$d/out" && ok=0
[ $ok -eq 1 ] && ! grep -q "r=1,4" "$d/out" && ok=0
[ $ok -eq 1 ] && ! grep -q "e=0,bad" "$d/out" && ok=0
if [ $ok -eq 1 ]; then PASS=$((PASS+1)); else
    echo "  FAIL lib-opt-aggregate"
    sed -n '1,8p' "$TMP/optlib.log"
    FAIL=$((FAIL+1))
fi

# --emit-llvm 产物必须能被 LLVM 解析
"$MYLC" --emit-llvm "$TMP/emit.ll" tests/positive/Arith.myl >/dev/null 2>&1
LLC=""
for l in llc-20 llc-19 llc-18 llc-17 llc-16 llc-15 llc; do
    if command -v "$l" >/dev/null 2>&1; then LLC=$l; break; fi
done
if [ -n "$LLC" ] && "$LLC" "$TMP/emit.ll" -o "$TMP/emit.s" >/dev/null 2>&1; then
    PASS=$((PASS+1))
else
    echo "  FAIL emit-llvm parse"
    FAIL=$((FAIL+1))
fi

echo "LLVM 后端：通过 $PASS，失败 $FAIL"
[ "$FAIL" -eq 0 ]
