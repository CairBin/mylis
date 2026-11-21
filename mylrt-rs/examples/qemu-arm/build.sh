#!/usr/bin/env bash
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
export PATH="${HOME}/.cargo/bin:${PATH}"
export MYLRT_IMPL_RS="$HERE/runtime.rs"
export MYLRT_PLATFORM_RS="$HERE/platform.rs"
cd "$HERE/kernel"
cargo build --release
OUT=$(timeout 15 qemu-system-arm -machine lm3s6965evb -nographic \
    -semihosting-config enable=on,target=native \
    -kernel target/thumbv7m-none-eabi/release/mylis-qemu-arm 2>&1 || true)
printf '%s\n' "$OUT"
if ! printf '%s' "$OUT" | grep -q 'qemu-arm freestanding ok'; then
    echo "QEMU 输出不符合预期" >&2
    exit 1
fi
