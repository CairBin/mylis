# 构建 std 的 Windows native 侧车（六个 Rust staticlib）。
# MSVC target 产出 mylis_*.lib；windows-gnu target 产出 libmylis_*.a。
param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$Out = "build\std-win32\native"
)
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Rustc = (Get-Command rustc.exe -ErrorAction SilentlyContinue).Source
if ([string]::IsNullOrWhiteSpace($Rustc)) {
    $Rustc = (Get-Command rustc -ErrorAction SilentlyContinue).Source
}
if ([string]::IsNullOrWhiteSpace($Rustc)) {
    throw "找不到 rustc；请安装 Rust 并确保它在 PATH 中"
}
$Src = Join-Path $Root "std-win32\native"
$OutPath = Join-Path $Root $Out
New-Item -ItemType Directory -Force -Path $OutPath | Out-Null
$Gnu = $Target -match "windows-gnu"
$Prefix = if ($Gnu) { "lib" } else { "" }
$Ext = if ($Gnu) { "a" } else { "lib" }

foreach ($Name in @("file", "net", "event", "io", "time", "process")) {
    $Input = Join-Path $Src ("$Name.rs")
    $Output = Join-Path $OutPath ("$Prefix" + "mylis_$Name.$Ext")
    Write-Host "  rustc $Name -> $Output" -ForegroundColor DarkGray
    & $Rustc --edition 2021 --crate-name "mylis_$Name" --crate-type staticlib `
        -C panic=abort -C opt-level=3 --target $Target `
        $Input -o $Output
    if ($LASTEXITCODE -ne 0) {
        throw "构建 $Output 失败（退出码 $LASTEXITCODE）"
    }
}
Write-Host "已生成 $OutPath\$Prefix`mylis_{file,io,time,net,event,process}.$Ext" -ForegroundColor Green
