#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/ext"
cat > "$TMP/ext/runtime.rs" <<'RS'
use crate::gc::BackendGc;
use crate::gc::Gc;
use crate::runtime::Runtime;
struct ExtRuntime;
static GC: BackendGc = BackendGc;
static RT: ExtRuntime = ExtRuntime;
impl Runtime for ExtRuntime { fn gc(&self) -> &dyn Gc { &GC } }
pub fn runtime() -> &'static dyn Runtime { &RT }
RS

make -s RT_IMPL="$TMP/ext" myrt
test -s build/libmylrt.a

cp "$TMP/ext/runtime.rs" "$TMP/ext/external.rs"
make -s RT_IMPL=external RT_IMPL_DIR="$TMP/ext" myrt
test -s build/libmylrt.a
nm -g build/libmylrt.a 2>/dev/null > "$TMP/syms1"
grep -q 'myrt_alloc' "$TMP/syms1"

make -s RT_IMPL=own myrt
test -s build/libmylrt.a
nm -g build/libmylrt.a 2>/dev/null > "$TMP/syms2"
grep -q 'myrt_alloc' "$TMP/syms2"

echo "运行时实现切换验收通过"
