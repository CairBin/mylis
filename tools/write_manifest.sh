#!/bin/sh
# 生成 build/mylis-build.json；参数顺序固定，供 Makefile 调用。
set -eu
out="$1"; os="$2"; family="$3"; style="$4"; abi="$5"; target="$6"
cc="$7"; ar="$8"; myrt="$9"; mylgc="${10}"; simple="${11}"; syslibs="${12:-}"
mkdir -p "$(dirname "$out")"
if [ -z "$syslibs" ]; then
  sysjson="[]"
else
  sysjson=""
  for lib in $syslibs; do
    if [ -n "$sysjson" ]; then sysjson="$sysjson,"; fi
    sysjson="$sysjson\"$lib\""
  done
  sysjson="[$sysjson]"
fi
cat > "$out" <<JSON
{"schema":1,"os":"$os","family":"$family","style":"$style","abi":"$abi","target":"$target","cc":"$cc","ar":"$ar","myrt":"$myrt","mylgc":"$mylgc","simple":"$simple","system_libs":$sysjson}
JSON
