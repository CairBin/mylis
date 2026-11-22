# Windows MinGW 发行包发布检查。
# 前置：已执行 make TOOLCHAIN_FAMILY=mingw dist，生成 dist/mylis-<ver>-win64-mingw.zip。
# 检查：bin 产物齐全、bin/mylc.exe 与 build/mylc.exe 一致、zip 结构完整、
# 包内 mylc.exe 与 bin 一致，并用包内编译器跑 Arith（MIR）与 WindowsStdApi（--std）冒烟。
param(
    [string]$Version = $(if ($env:VERSION) { $env:VERSION } else { "0.1.0" }),
    [switch]$SkipSmoke
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Name = "mylis-$Version-win64-mingw"
$Bin = Join-Path $Root "bin"
$Dist = Join-Path $Root "dist\$Name"
$Zip = Join-Path $Root "dist\$Name.zip"
$ManifestPath = Join-Path $Bin "mylis-build.json"

function Assert-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "缺少 $Label：$Path" }
}

function Get-Sha256([string]$Path) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.IO.File]::ReadAllBytes($Path)
        $hash = $sha.ComputeHash($bytes)
        return ([System.BitConverter]::ToString($hash) -replace "-", "")
    } finally {
        $sha.Dispose()
    }
}

# Windows PowerShell 5.1 会把原生程序 stderr 当成终止性错误；临时降为 Continue，
# 把 stdout/stderr 一起写进日志，再由退出码判断。
function Invoke-NativeLogged([string]$Exe, [string[]]$Arguments, [string]$LogPath) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $Exe @Arguments *> $LogPath
        $code = $LASTEXITCODE
        return $code
    } finally {
        $ErrorActionPreference = $old
    }
}

function Read-TextFile([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) {
        $text = [System.Text.Encoding]::Unicode.GetString($bytes)
    } else {
        $text = [System.Text.Encoding]::UTF8.GetString($bytes)
    }
    return $text.TrimStart([char]0xFEFF)
}

function Get-TextLines([string]$Path) {
    $text = (Read-TextFile $Path) -replace "`r`n", "`n" -replace "`r", "`n"
    $text = $text.TrimEnd("`n")
    if ($text.Length -eq 0) { return @() }
    return @($text -split "`n")
}

if (-not (Test-Path -LiteralPath $Zip -PathType Leaf)) {
    throw "缺少发行包：$Zip；请先执行 make TOOLCHAIN_FAMILY=mingw dist"
}
Assert-File (Join-Path $Bin "mylc.exe") "bin/mylc.exe"
Assert-File (Join-Path $Bin "myls.exe") "bin/myls.exe"
Assert-File $ManifestPath "bin/mylis-build.json"
foreach ($lib in @("libmylrt.a", "libmylgc.a", "libgc_simple.a")) {
    Assert-File (Join-Path $Bin "build\$lib") "bin/build/$lib"
}
foreach ($lib in @("file", "io", "time", "net", "event", "process")) {
    Assert-File (Join-Path $Bin "build\std-win32\native\libmylis_$lib.a") "bin/build/std-win32/native/libmylis_$lib.a"
}

$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
if ($manifest.family -ne "mingw" -or $manifest.abi -ne "gnu") {
    throw "bin/mylis-build.json 不是 MinGW/GNU 清单：family=$($manifest.family) abi=$($manifest.abi)"
}

$buildMylc = Join-Path $Root "build\mylc.exe"
if (Test-Path -LiteralPath $buildMylc -PathType Leaf) {
    $buildHash = Get-Sha256 $buildMylc
    $binHash = Get-Sha256 (Join-Path $Bin "mylc.exe")
    if ($buildHash -ne $binHash) { throw "bin/mylc.exe 落后于 build/mylc.exe；请重新 make TOOLCHAIN_FAMILY=mingw all" }
    Write-Host "  ok   bin/mylc.exe 与 build/mylc.exe 一致（$binHash）"
} else {
    Write-Host "  skip 未找到 build/mylc.exe，跳过 bin 与 build 的哈希比对"
}

$tar = Join-Path $env:SystemRoot "System32\tar.exe"
if (-not (Test-Path -LiteralPath $tar -PathType Leaf)) { throw "缺少 bsdtar：$tar" }
$entries = @(& $tar -tf $Zip)
if ($LASTEXITCODE -ne 0) { throw "无法读取 zip：$Zip" }
$prefix = "$Name/"
$required = @(
    "mylc.exe", "myls.exe", "mylis-build.json",
    "build/libmylrt.a", "build/libmylgc.a", "build/libgc_simple.a",
    "build/std-win32/native/libmylis_file.a",
    "build/std-win32/native/libmylis_io.a",
    "build/std-win32/native/libmylis_time.a",
    "build/std-win32/native/libmylis_net.a",
    "build/std-win32/native/libmylis_event.a",
    "build/std-win32/native/libmylis_process.a",
    "std/", "docs/", "tests/"
)
foreach ($rel in $required) {
    $entry = $prefix + $rel
    if (-not ($entries | Where-Object { $_ -eq $entry })) { throw "zip 缺少条目：$entry" }
}
Write-Host "  ok   zip 结构完整（$($entries.Count) 个条目）"

$Temp = Join-Path $env:TEMP ("mylis-release-check-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Temp | Out-Null
& $tar -xf $Zip -C $Temp
if ($LASTEXITCODE -ne 0) { throw "解压 zip 失败" }
$PkgRoot = Join-Path $Temp $Name
$pkgHash = Get-Sha256 (Join-Path $PkgRoot "mylc.exe")
$binHash = Get-Sha256 (Join-Path $Bin "mylc.exe")
if ($pkgHash -ne $binHash) { throw "包内 mylc.exe 与 bin/mylc.exe 不一致" }
Write-Host "  ok   包内 mylc.exe 与 bin 一致"
Write-Host "  temp $Temp"

if ($SkipSmoke) {
    Write-Host "发布检查通过（已跳过冒烟）"
    exit 0
}

$gcc = Get-Command gcc -ErrorAction SilentlyContinue
if ($null -eq $gcc) { throw "PATH 中找不到 gcc，无法做包内 MIR 冒烟" }
$env:MYLIS_ROOT = $PkgRoot
$env:MYLIS_PATH = "$PkgRoot;$PkgRoot\std\src"
New-Item -ItemType Directory -Force -Path (Join-Path $PkgRoot "build\tests") | Out-Null
Push-Location $PkgRoot
try {
    $pkgMylc = Join-Path $PkgRoot "mylc.exe"
    $arithSrc = Join-Path $PkgRoot "tests\positive\Arith.myl"
    $arithExe = Join-Path $PkgRoot "release-arith.exe"
    $arithBuildLog = Join-Path $PkgRoot "release-arith.build.log"
    $arithOut = Join-Path $PkgRoot "release-arith.out"
    $arithExpected = Join-Path $PkgRoot "tests\positive\Arith.expected"
    $code = Invoke-NativeLogged $pkgMylc @("--backend", "mir", $arithSrc, "-o", $arithExe) $arithBuildLog
    if ($code -ne 0) { throw "包内 mylc 编译 Arith 失败（MIR）" }
    $code = Invoke-NativeLogged $arithExe @() $arithOut
    if ($code -ne 0) { throw "包内 Arith 运行失败" }
    $diff = Compare-Object (Get-TextLines $arithExpected) (Get-TextLines $arithOut)
    if ($null -ne $diff) { throw "包内 Arith 输出不符" }
    Write-Host "  ok   包内 Arith（MIR）编译、运行、输出一致"

    $stdSrc = Join-Path $PkgRoot "tests\std_native\WindowsStdApi.myl"
    $stdExe = Join-Path $PkgRoot "release-std.exe"
    $stdBuildLog = Join-Path $PkgRoot "release-std.build.log"
    $stdOut = Join-Path $PkgRoot "release-std.out"
    $stdExpected = Join-Path $PkgRoot "tests\std_native\WindowsStdApi.expected"
    $code = Invoke-NativeLogged $pkgMylc @("--std", "--backend", "mir", $stdSrc, "-o", $stdExe) $stdBuildLog
    if ($code -ne 0) { throw "包内 mylc 编译 WindowsStdApi 失败（--std）" }
    $code = Invoke-NativeLogged $stdExe @() $stdOut
    if ($code -ne 0) { throw "包内 WindowsStdApi 运行失败" }
    $diff = Compare-Object (Get-TextLines $stdExpected) (Get-TextLines $stdOut)
    if ($null -ne $diff) { throw "包内 WindowsStdApi 输出不符" }
    Write-Host "  ok   包内 WindowsStdApi（--std）编译、运行、输出一致"
} finally {
    Pop-Location
}

Write-Host "发布检查通过：$Zip"
