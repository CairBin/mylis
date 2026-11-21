#!/usr/bin/env bash
# 下载并以静态库方式编译 Boehm GC 到 $GC_ROOT（默认 ~/mylis-toolchain）。
# Mylis 的内存模型固定使用 Boehm GC（规范第 2.5 节）。
set -euo pipefail

GC_ROOT="${GC_ROOT:-$HOME/mylis-toolchain}"
VERSION="8.2.8"
TARBALL="gc-${VERSION}.tar.gz"
URL="https://github.com/ivmai/bdwgc/releases/download/v${VERSION}/${TARBALL}"
WORK="$(mktemp -d)"

mkdir -p "$GC_ROOT"
if [ -f "$GC_ROOT/lib/libgc.a" ]; then
  echo "Boehm GC 已存在于 $GC_ROOT，跳过。"
  exit 0
fi

echo "==> 下载 bdwgc ${VERSION}"
curl -sSL -o "$WORK/$TARBALL" "$URL"
tar -xzf "$WORK/$TARBALL" -C "$WORK"

echo "==> 配置（静态库、无线程疑虑、关闭 libatomic_ops 依赖）"
cd "$WORK/gc-${VERSION}"
./configure --prefix="$GC_ROOT" \
  --enable-static --disable-shared \
  --enable-threads=posix --with-libatomic-ops=none >/dev/null

echo "==> 编译"
make -j"$(nproc)" >/dev/null
make install >/dev/null

echo "Boehm GC 已安装到 $GC_ROOT"
