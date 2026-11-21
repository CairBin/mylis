#!/usr/bin/env bash
# 注解告警契约：拼错的内置注解必须告警；自定义注解保持静默；@deprecated 调用点告警。
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MYLC="$ROOT/build/mylc"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
fail=0

cat > "$TMP/A.myl" <<'ML'
@exprot("c")
pub fn f() -> i32 { return 1; }
fn main() -> i32 { return f(); }
ML
out="$("$MYLC" --check "$TMP/A.myl" 2>&1)"
echo "$out" | grep -q "MYLW01" || { echo "缺少 @exprot 拼写告警"; fail=1; }
echo "$out" | grep -q "@export" || { echo "缺少拼写建议 @export"; fail=1; }

cat > "$TMP/B.myl" <<'ML'
@route("/x")
fn h() -> i32 { return 0; }
fn main() -> i32 { return h(); }
ML
out="$("$MYLC" --check "$TMP/B.myl" 2>&1)"
if echo "$out" | grep -q "MYLW01"; then echo "自定义注解不应告警"; fail=1; fi

cat > "$TMP/C.myl" <<'ML'
@deprecated("改用 g")
pub fn old() -> i32 { return 1; }
pub fn g() -> i32 { return 2; }
fn main() -> i32 { return old(); }
ML
out="$("$MYLC" --check "$TMP/C.myl" 2>&1)"
echo "$out" | grep -q "MYLW02" || { echo "缺少 @deprecated 调用点告警"; fail=1; }

if [ "$fail" -eq 0 ]; then echo "注解告警契约通过"; fi
exit "$fail"
