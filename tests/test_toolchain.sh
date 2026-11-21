#!/usr/bin/env bash
# 工具链家族/插件规格验收：不依赖真实 MSVC，用假编译器检查命令拼装
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MYLC=${MYLC:-$ROOT/bin/mylc}
[ -x "$MYLC" ] || { echo "缺少 $MYLC"; exit 1; }
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
fail=0
ok() { printf '  ok   %s\n' "$1"; }
no() { printf '  FAIL %s\n' "$1"; fail=$((fail+1)); }

cat > "$TMP/fake-cl" <<'SH'
#!/bin/sh
out=""
for a in "$@"; do
  case "$a" in
    /Fe:*) out="${a#/Fe:}" ;;
    /Fo*)  touch "${a#/Fo}" ;;
    /OUT:*) touch "${a#/OUT:}" ;;
  esac
done
[ -n "$out" ] && touch "$out"
exit 0
SH
chmod +x "$TMP/fake-cl"
printf '#!/bin/sh\nfor a in "$@"; do case "$a" in /OUT:*) touch "${a#/OUT:}" ;; esac; done\n' > "$TMP/fake-lib"
chmod +x "$TMP/fake-lib"

# 1) 内置 msvc 家族：exe 走 /Fe: + 运行时源码
out=$(MYLIS_NO_MANIFEST=1 MYLIS_FAMILY=msvc MYLIS_CC="$TMP/fake-cl" MYLIS_AR="$TMP/fake-lib" MYLIS_GC=simple \
      "$MYLC" --backend mir --verbose "$ROOT/tests/positive/Alias.myl" -o "$TMP/a.exe" 2>&1)
if printf '%s' "$out" | grep -q '/Fe:' && printf '%s' "$out" | grep -q 'gc_simple.c'; then
  ok "msvc exe 命令拼装"
else
  no "msvc exe 命令拼装"; echo "$out" | tail -3
fi
[ -f "$TMP/a.exe" ] && ok "msvc exe 产物" || no "msvc exe 产物"

# 1b) msvc 默认（boehm）自动改用自研 GC：链 mylgc.lib 而不是 gc_simple.c
out=$(MYLIS_NO_MANIFEST=1 MYLIS_FAMILY=msvc MYLIS_CC="$TMP/fake-cl" MYLIS_AR="$TMP/fake-lib" \
  "$MYLC" --backend mir --verbose "$ROOT/tests/positive/Alias.myl" -o "$TMP/ag.exe" 2>&1)
if echo "$out" | grep -q "mylgc.lib" && ! echo "$out" | grep -q "gc_simple.c"; then
  ok "msvc 默认 GC → own（mylgc.lib）"
else
  no "msvc 默认 GC → own（mylgc.lib）"; echo "$out" | tail -3
fi

# 2) 内置 msvc 家族：static 走 /c + lib /OUT:
out=$(MYLIS_NO_MANIFEST=1 MYLIS_FAMILY=msvc MYLIS_CC="$TMP/fake-cl" MYLIS_AR="$TMP/fake-lib" MYLIS_GC=simple \
      "$MYLC" --backend mir --verbose "$ROOT/tests/positive/Alias.myl" --static -o "$TMP/liba.lib" 2>&1)
case "$out" in
  *"/Fo"*) ok "msvc static 编译参数" ;;
  *) no "msvc static 编译参数" ;;
esac
[ -f "$TMP/liba.lib" ] && ok "msvc static 产物" || no "msvc static 产物"

# 2b) msvc shared：/LD + /Fe:dll，生成 C 带 dllexport
out=$(MYLIS_NO_MANIFEST=1 MYLIS_FAMILY=msvc MYLIS_CC="$TMP/fake-cl" MYLIS_AR="$TMP/fake-lib" MYLIS_GC=simple \
      "$MYLC" --backend mir --verbose "$ROOT/tests/lib/mathlib.myl" --shared -o "$TMP/m.dll" 2>&1)
if printf '%s' "$out" | grep -q '/LD' && [ -f "$TMP/m.dll" ]; then
  ok "msvc shared 命令与产物"
else
  no "msvc shared 命令与产物"; echo "$out" | tail -3
fi
if grep -q '__declspec(dllexport)' "$TMP/m.dll.myl.c" && [ -f "$TMP/m.h" ]; then
  ok "msvc shared 导出属性与头文件"
else
  no "msvc shared 导出属性与头文件"
fi

# 3) 插件规格：自定义 toml 被选中并通过 MYLIS_FAMILY 生效
mkdir -p "$TMP/toolchains"
cat > "$TMP/toolchains/plug.toml" <<TOML
name = "plug"
style = "gnu"
cc = "$TMP/fake-gcc"
ar = "ar"
cflags = ["-DPLUGIN=1"]
runtime = "archive"
TOML
printf '#!/bin/sh\nout=""\nprev=""\nfor a in "$@"; do case "$prev" in -o) out="$a";; esac; prev="$a"; done\n[ -n "$out" ] && touch "$out"\nexit 0\n' > "$TMP/fake-gcc"
chmod +x "$TMP/fake-gcc"
out=$(MYLIS_FAMILY=plug MYLIS_TOOLCHAINS="$TMP/toolchains" MYLIS_GC=simple \
      "$MYLC" --backend mir --verbose "$ROOT/tests/positive/Alias.myl" -o "$TMP/p" 2>&1)
case "$out" in
  *"PLUGIN=1"*) ok "插件规格生效" ;;
  *) no "插件规格生效"; echo "$out" | tail -3 ;;
esac

# 4) 未知规格：明确告警并回退 gcc
out=$(MYLIS_FAMILY=no-such MYLIS_GC=simple "$MYLC" --backend mir "$ROOT/tests/positive/Alias.myl" -o "$TMP/n" 2>&1)
case "$out" in
  *"未找到工具链规格"*|*"toolchain spec"*) ok "未知规格告警" ;;
  *) no "未知规格告警" ;;
esac

# 5) gcc 家族回归：默认仍用 -o / -lname / libmylrt.a
out=$(MYLIS_GC=simple "$MYLC" --backend mir --verbose "$ROOT/tests/positive/Alias.myl" -o "$TMP/g" 2>&1)
case "$out" in
  *"-o"*"libmylrt.a"*) ok "gcc 家族不回归" ;;
  *) no "gcc 家族不回归" ;;
esac

if [ "$fail" -eq 0 ]; then echo "工具链验收通过"; else echo "工具链验收失败：$fail"; fi
exit "$fail"
