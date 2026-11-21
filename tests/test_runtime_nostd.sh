#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

make -s RT_NO_STD=1 myrt
test -s build/libmylrt.a

cat > "$TMP/client.c" <<'C'
#include "mylrt.h"
int main(void) {
    myrt_init();
    MyStr *s = mylrt_str_lit("nostd ok", 8);
    myrt_println(s);
    return 0;
}
C
gcc -Imyrt "$TMP/client.c" build/libmylrt.a build/libmylgc.a -lpthread -ldl -lm -o "$TMP/client"
"$TMP/client" > "$TMP/out"
grep -q 'nostd ok' "$TMP/out"

make -s RT_IMPL=own myrt
test -s build/libmylrt.a
echo "no_std 运行时验收通过"
