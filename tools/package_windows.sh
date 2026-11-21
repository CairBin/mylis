#!/usr/bin/env bash
# Windows 发行包现在走 MSVC 脚本（tools/package_windows.ps1）。
# 保留此入口只做转发；旧 MinGW 流程已废弃。
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
if command -v powershell >/dev/null 2>&1; then
  exec powershell -NoProfile -ExecutionPolicy Bypass -File "$ROOT/tools/package_windows.ps1"
fi
if command -v pwsh >/dev/null 2>&1; then
  exec pwsh -NoProfile -File "$ROOT/tools/package_windows.ps1"
fi
echo "Windows 发行包需要 PowerShell + MSVC（cl.exe/lib.exe）；请在 Windows 上运行："
echo "  powershell -ExecutionPolicy Bypass -File tools/package_windows.ps1"
exit 1
