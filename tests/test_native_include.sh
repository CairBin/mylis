#!/usr/bin/env bash
# mylis.toml [native] include/dirs/libs：C 头文件与库由清单自动加入编译和链接。
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC="$ROOT/build/mylc"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

PROJ="$WORK/native_include"
mkdir -p "$PROJ/src" "$PROJ/native"
cat > "$PROJ/mylis.toml" <<'TOML'
[package]
name = "native_include"
version = "0.1.0"

[build]
backend = "mir"

[native]
include = ["native"]
dirs = ["native"]
libs = ["answer"]

[targets.app]
kind = "exe"
entry = "src/main.myl"
out = "app"
TOML
cat > "$PROJ/src/native_bind.myl" <<'MYL'
@header("answer.h")
pub unsafe fn answer() -> i32;
MYL
cat > "$PROJ/src/main.myl" <<'MYL'
import { println_i32 } from "Io.myl";
import { answer } from "native_bind.myl";

fn main() -> i32 {
    unsafe { println_i32(answer()); }
    return 0;
}
MYL
cat > "$PROJ/native/answer.h" <<'C'
#ifndef ANSWER_H
#define ANSWER_H
int answer(void);
#endif
C
cat > "$PROJ/native/answer.c" <<'C'
#include "answer.h"
int answer(void) { return 42; }
C

cc -c "$PROJ/native/answer.c" -o "$PROJ/native/answer.o"
ar rcs "$PROJ/native/libanswer.a" "$PROJ/native/answer.o"

cd "$PROJ"
MYLIS_PATH="$ROOT:$ROOT/std/src" "$MYLC" build app >"$WORK/build.log" 2>&1
actual=$("./app")
if [ "$actual" != "42" ]; then
  echo "FAIL native_include（输出：$actual）"
  sed -n '1,8p' "$WORK/build.log"
  exit 1
fi

echo "  ok   native_include（[native] include/dirs/libs）"
