#!/usr/bin/env bash
# Mylis 测试套件：正向用例（编译 + 运行 + 输出比对）与反向用例（应当报出指定错误码）。
# 默认 MIR；可用 MYLIS_BACKEND=ast 对同一套用例跑 AST 回退后端。
set -u

cd "$(dirname "$0")/.." || exit 1
ROOT=$PWD
: "${MYLIS_PATH:=$ROOT:$ROOT/std/src}"
export MYLIS_PATH
MYLC=./build/mylc
BACKEND=${MYLIS_BACKEND:-mir}
OUT=build/tests
mkdir -p "$OUT"

pass=0
fail=0

echo "== 正向用例 =="
for f in tests/positive/*.myl; do
  name=$(basename "$f" .myl)
  case "$name" in
    *_bind) continue ;;
  esac
  bin="$OUT/$name"
  if ! "$MYLC" --backend "$BACKEND" "$f" -o "$bin" >"$OUT/$name.build.log" 2>&1; then
    echo "  FAIL $name（编译失败）"
    sed -n '1,6p' "$OUT/$name.build.log"
    fail=$((fail + 1))
    continue
  fi
  if ! "$bin" >"$OUT/$name.actual" 2>"$OUT/$name.err"; then
    echo "  FAIL $name（运行失败）"
    sed -n '1,4p' "$OUT/$name.err"
    fail=$((fail + 1))
    continue
  fi
  if diff -u "tests/positive/$name.expected" "$OUT/$name.actual" >"$OUT/$name.diff"; then
    echo "  ok   $name"
    pass=$((pass + 1))
  else
    echo "  FAIL $name（输出不符）"
    sed -n '1,12p' "$OUT/$name.diff"
    fail=$((fail + 1))
  fi
done

echo "== 反向用例 =="
for f in tests/negative/*.myl; do
  name=$(basename "$f" .myl)
  case "$name" in
    *_helper|*_b|*_bind) continue ;;
  esac
  expect=$(cat "tests/negative/$name.expect" 2>/dev/null || echo "MYL")
  if "$MYLC" --check "$f" >"$OUT/$name.check.log" 2>&1; then
    echo "  FAIL $name（本应报错，却通过了检查）"
    fail=$((fail + 1))
    continue
  fi
  if grep -q "error\[$expect\]" "$OUT/$name.check.log"; then
    echo "  ok   $name ( $expect )"
    pass=$((pass + 1))
  else
    echo "  FAIL $name（未出现 $expect）"
    sed -n '1,6p' "$OUT/$name.check.log"
    fail=$((fail + 1))
  fi
done

echo
echo "通过：$pass    失败：$fail"
[ "$fail" -eq 0 ]
