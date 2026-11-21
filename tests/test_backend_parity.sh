#!/usr/bin/env bash
# 后端对拍：同一程序分别用 MIR-C（默认）与 AST-C 生成，比较 stdout 与退出码。
set -u
cd "$(dirname "$0")/.." || exit 1
: "${MYLIS_PATH:=$PWD:$PWD/std/src}"
export MYLIS_PATH
MYLC=./build/mylc
OUT=build/parity
mkdir -p "$OUT"
pass=0
fail=0
for f in tests/positive/*.myl; do
  name=$(basename "$f" .myl)
  case "$name" in *_bind) continue ;; esac
  if ! "$MYLC" --backend mir "$f" -o "$OUT/$name.mir" >"$OUT/$name.mir.log" 2>&1; then
    echo "  FAIL $name（MIR-C 编译失败）"; fail=$((fail+1)); continue
  fi
  if ! "$MYLC" --backend ast "$f" -o "$OUT/$name.ast" >"$OUT/$name.ast.log" 2>&1; then
    echo "  FAIL $name（AST-C 编译失败）"; fail=$((fail+1)); continue
  fi
  "$OUT/$name.mir" >"$OUT/$name.mir.out" 2>"$OUT/$name.mir.err"; mc=$?
  "$OUT/$name.ast" >"$OUT/$name.ast.out" 2>"$OUT/$name.ast.err"; ac=$?
  if [ "$mc" -ne "$ac" ]; then
    echo "  FAIL $name（退出码 $mc vs $ac）"; fail=$((fail+1)); continue
  fi
  if ! diff -u "$OUT/$name.ast.out" "$OUT/$name.mir.out" >"$OUT/$name.diff"; then
    echo "  FAIL $name（stdout 不一致）"; sed -n '1,12p' "$OUT/$name.diff"; fail=$((fail+1)); continue
  fi
  pass=$((pass+1))
done
echo "后端对拍：通过 $pass，失败 $fail"
[ "$fail" -eq 0 ]
