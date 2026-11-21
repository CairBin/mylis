#!/usr/bin/env bash
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"

make -s RT_NO_STD=1 RT_NO_BACKEND_GC=1 RT_IMPL="$HERE/runtime.rs" PLATFORM_IMPL="$HERE/platform.rs" myrt
gcc -nostdlib -static -fno-stack-protector -fno-pie -no-pie -Imyrt \
    "$HERE/client.c" build/libmylrt.a -lgcc -o /tmp/mylis_freestanding_client
/tmp/mylis_freestanding_client
make -s RT_IMPL=own myrt
