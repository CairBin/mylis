#!/usr/bin/env bash
# 不依赖 clang 的 LLVM IR 生成回归：只检查所有正向用例能成功 emit，
# 并抽查 enum→interface vtable 和源码 @header 外部声明。
set -u
cd "$(dirname "$0")/.." || exit 1
ROOT=$PWD
: "${MYLIS_PATH:=$ROOT:$ROOT/std/src}"
export MYLIS_PATH
MYLC=./build/mylc
OUT=build/llvm-emit
mkdir -p "$OUT"
pass=0
fail=0

echo "== LLVM IR 生成 =="
for f in tests/positive/*.myl; do
  name=$(basename "$f" .myl)
  case "$name" in *_bind) continue ;; esac
  if "$MYLC" --backend llvm --emit-llvm "$OUT/$name.ll" "$f" >"$OUT/$name.log" 2>&1; then
    pass=$((pass + 1))
  else
    echo "  FAIL $name（emit-llvm 失败）"
    sed -n '1,5p' "$OUT/$name.log"
    fail=$((fail + 1))
  fi
done

# Annots：enum 插值必须生成 enum→ToString 的 vtable 包装。
if grep -q 'define ptr @VTW_' "$OUT/Annots.ll" \
    && grep -q ' = global %Vt_' "$OUT/Annots.ll"; then
  echo "  ok   Annots（enum→interface vtable）"
  pass=$((pass + 1))
else
  echo "  FAIL Annots（缺少 enum→interface vtable）"
  fail=$((fail + 1))
fi

# Ffi：源码里的 @header unsafe 声明必须成为 LLVM 外部声明。
if grep -q 'declare i64 @strlen(ptr)' "$OUT/Ffi.ll"; then
  echo "  ok   Ffi（源码 @header 外部声明）"
  pass=$((pass + 1))
else
  echo "  FAIL Ffi（缺少 strlen 外部声明）"
  fail=$((fail + 1))
fi

echo
echo "LLVM IR 生成：通过 $pass    失败 $fail"
[ "$fail" -eq 0 ]
