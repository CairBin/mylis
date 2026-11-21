#!/usr/bin/env bash
# core 层边界检查：core/ 必须与操作系统无关。
#   1) core/ 不允许任何 @header（不绑定 C 标准库，也不显式绑定运行时头）
#   2) import 只能指向 core/ 内的文件（不允许反向依赖 std/ 层）
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CORE="$ROOT/core"
fail=0

ALLOW_HEADERS=" "

# ---- 1) C 头文件白名单
while IFS= read -r h; do
  [ -z "$h" ] && continue
  if [ "${ALLOW_HEADERS/ $h /}" = "$ALLOW_HEADERS" ]; then
    echo "core 层违规：@header(\"$h\")；core/ 不允许 C 头文件"
    fail=1
  fi
done < <(grep -ho '@header("[^"]*")' "$CORE"/*.myl 2>/dev/null | sed 's/@header("//; s/")//')

# ---- 2) import 只能在 core 内部
while IFS=$'\t' read -r f raw; do
  [ -z "${raw:-}" ] && continue
  base="$(basename "$raw")"
  if [ ! -f "$CORE/$base" ]; then
    echo "core 层违规：$(basename "$f") 依赖 core 之外的文件 \"$raw\"（core 层不能反向依赖 std 层）"
    fail=1
  fi
done < <(grep -Ho 'import "[^"]*"' "$CORE"/*.myl 2>/dev/null | sed 's/:import "/\t/; s/"$/\t/')

if [ "$fail" -eq 0 ]; then
  echo "core 层边界检查通过：core/ 无 C 头文件，只调用 mylrt 运行时符号"
fi
exit "$fail"
