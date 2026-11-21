#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
if ! command -v qemu-system-arm >/dev/null 2>&1; then
    echo "跳过 QEMU ARM 验收：未安装 qemu-system-arm"
    exit 0
fi
"$ROOT/mylrt-rs/examples/qemu-arm/build.sh" | tee /tmp/mylis_qemu_arm.out
grep -q 'qemu-arm freestanding ok' /tmp/mylis_qemu_arm.out
echo "QEMU ARM 裸机运行时验收通过"
