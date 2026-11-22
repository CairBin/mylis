# 生成 dist/mylis-<ver>-win64-mingw.zip（Windows + MinGW GCC / Rust windows-gnu）
# 前置：make TOOLCHAIN_FAMILY=mingw all 已生成 bin/。
# 默认会检查 bin/；缺失或不是 MinGW 布局时自动构建。
param(
    [string]$Version = $(if ($env:VERSION) { $env:VERSION } else { "0.1.0" }),
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Name = "mylis-$Version-win64-mingw"
$Dist = Join-Path $Root "dist\$Name"
$Bin = Join-Path $Root "bin"
$Manifest = Join-Path $Bin "mylis-build.json"

Set-Location $Root
$manifestData = $null
if ((Test-Path -LiteralPath $Manifest) -and (Test-Path -LiteralPath (Join-Path $Bin "mylc.exe"))) {
    $manifestData = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
}
if (-not $SkipBuild -and ($null -eq $manifestData -or $manifestData.family -ne "mingw")) {
    Write-Host "== 构建 bin/（TOOLCHAIN_FAMILY=mingw）"
    $make = Get-Command make -ErrorAction SilentlyContinue
    if ($null -eq $make) { throw "找不到 make，请先把 GNU Make 加入 PATH" }
    & $make.Source "TOOLCHAIN_FAMILY=mingw" "all"
    if ($LASTEXITCODE -ne 0) { throw "make TOOLCHAIN_FAMILY=mingw all 失败" }
    $manifestData = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
}
if ($null -eq $manifestData -or $manifestData.family -ne "mingw") {
    throw "bin/ 不是 MinGW 布局；请执行 make TOOLCHAIN_FAMILY=mingw all"
}

Write-Host "== 1/3 准备发行目录"
if (Test-Path -LiteralPath $Dist) {
    Remove-Item -LiteralPath $Dist -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

Write-Host "== 2/3 收集 bin/ 与源码"
Copy-Item -Path (Join-Path $Bin "*") -Destination $Dist -Recurse -Force
foreach ($d in @("docs", "tests", "editors", "toolchains")) {
    $src = Join-Path $Root $d
    if (Test-Path -LiteralPath $src) {
        Copy-Item -Path $src -Destination (Join-Path $Dist $d) -Recurse -Force
    }
}
foreach ($f in @("README.md", "Makefile", "LICENSE")) {
    Copy-Item -LiteralPath (Join-Path $Root $f) -Destination $Dist -Force
}
foreach ($pkg in @("mylc", "mylgc", "mylrt-rs")) {
    Copy-Item -LiteralPath (Join-Path $Root "$pkg\src") -Destination (Join-Path $Dist $pkg) -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $Root "$pkg\Cargo.toml") -Destination (Join-Path $Dist $pkg) -Force
}
Copy-Item -LiteralPath (Join-Path $Root "mylrt-rs\build.rs") -Destination (Join-Path $Dist "mylrt-rs") -Force

$toolchain = '{"family":"mingw"}'
Set-Content -Path (Join-Path $Dist "mylis-toolchain.json") -Value $toolchain -Encoding ascii
Set-Content -Path (Join-Path $Dist "mylis-build.json") -Value (Get-Content -LiteralPath $Manifest -Raw) -Encoding ascii

Write-Host "== 3/3 打包"
$zip = Join-Path $Root "dist\$Name.zip"
$tar = Join-Path $env:SystemRoot "System32\tar.exe"
if (Test-Path -LiteralPath $tar) {
    # Windows 自带 bsdtar 流式压缩，避免 Compress-Archive 在大包上一次性占用大量内存。
    & $tar -a -c -f $zip -C (Split-Path -Parent $Dist) (Split-Path -Leaf $Dist)
    if ($LASTEXITCODE -ne 0) { throw "tar 打包失败" }
} else {
    Compress-Archive -Path $Dist -DestinationPath $zip -Force
}
Write-Host "包已生成：$zip"
