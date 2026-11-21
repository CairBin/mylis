#!/usr/bin/env bash
# std 包自洽检查：std 必须能作为普通包被加载，且只通过包名引用 core。
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MYLC="$ROOT/build/mylc"
: "${MYLIS_PATH:=$ROOT}"
export MYLIS_PATH
"$MYLC" --check --update-lock "$ROOT/std/src/lib.myl" >/tmp/mylis-check-std.log 2>&1 || {
  cat /tmp/mylis-check-std.log
  exit 1
}
echo "std 包自洽检查通过"
