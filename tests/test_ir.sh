#!/usr/bin/env bash
# MIR 验收：
#   1) --emit-ir 不需要 C 工具链/GC 就能产出文本
#   2) 基本结构（函数、CFG、分支、调用）存在
#   3) MIR 结构校验 0 错误
#   4) --emit-ir 之后 C 后端仍可正常编译（互不干扰由 run_tests 覆盖）
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC="$ROOT/build/mylc"
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

echo "== MIR dump"
"$MYLC" --emit-ir "$OUT/loops.myl.ir" "$ROOT/tests/positive/Loops.myl" --verbose \
    > "$OUT/stdout" 2> "$OUT/stderr"
grep -q "MIR functions=" "$OUT/stderr"
grep -q "verify_errors=0" "$OUT/stderr"

IR="$OUT/loops.myl.ir"
grep -q "; Mylis MIR v0" "$IR"
grep -q "fn @.*main" "$IR"
grep -q "bb0:" "$IR"
grep -q "br %" "$IR"
grep -q "ret " "$IR"
echo "  ok MIR 结构"

echo "== match lowering"
"$MYLC" --emit-ir "$OUT/match.myl.ir" "$ROOT/tests/positive/Match.myl" --verbose \
    2> "$OUT/match.err"
grep -q "verify_errors=0" "$OUT/match.err"
grep -q "match.test" "$OUT/match.myl.ir"
grep -q "bind " "$OUT/match.myl.ir"
grep -q "match.arm" "$OUT/match.myl.ir"
echo "  ok match CFG"

echo "== MIR verifier 拦截坏引用"
# 单元测试在 mylc 内部；这里只确认 --verbose 的校验通道被调用
"$MYLC" --emit-ir "$OUT/simple.myl.ir" "$ROOT/tests/positive/Arith.myl" --verbose \
    2> "$OUT/simple.err"
grep -q "verify_errors=0" "$OUT/simple.err"
echo "  ok verifier"

echo "MIR 验收通过"
