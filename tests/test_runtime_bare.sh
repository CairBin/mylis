#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
if [ "$(uname -m)" != "x86_64" ] || [ "$(uname -s)" != "Linux" ]; then
  echo "跳过 freestanding 验收：仅支持 Linux x86_64 示例"
  exit 0
fi
"$ROOT/mylrt-rs/examples/freestanding/build.sh" 2>&1 | tee /tmp/mylis_freestanding.out
grep -q 'freestanding ok' /tmp/mylis_freestanding.out
echo "freestanding/no-libc 运行时验收通过"
