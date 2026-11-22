# Mylis Windows/MSVC 冒烟测试
#
# 用法（在 Windows 上，普通 PowerShell 即可；脚本会尝试自动加载 vcvars64）：
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\test_windows.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\test_windows.ps1 -Mode quick
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\test_windows.ps1 -Gc simple
#
# 行为：
#   1. 检查 Windows / MSVC / Cargo / Rust MSVC target / GNU make
#   2. make all（默认 own GC + LLVM 后端）
#   3. 验证 bin/ 发行布局
#   4. 编译并运行 hello、u64 模式、FmtSpec、三个后端
#   5. 生成并消费一个本地 .mylib
#   6. Windows 原生 std：File / Time / Process / TCP / UDP / EventLoop / Worker
#      协议、WorkerFlaky、写超时，含 LLVM/MIR/AST 后端
#   7. 可选：make dist，解开 zip 后再跑一遍包内 mylc.exe
#
# 不修改源码；所有临时文件放在系统临时目录，成功后删除。
[CmdletBinding()]
param(
    [ValidateSet("all", "quick", "package")]
    [string]$Mode = "all",
    [ValidateSet("own", "simple")]
    [string]$Gc = "own",
    [switch]$SkipBuild,
    [switch]$SkipLlvm,
    [switch]$SkipPackage,
    [switch]$Keep,
    [string]$VcVars = ""
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("mylis-win-smoke-" + $PID)
$OldMylisPath = $env:MYLIS_PATH
$OldMylisGc = $env:MYLIS_GC
$OldMylisRoot = $env:MYLIS_ROOT
$OldMylisCc = $env:MYLIS_CC
$OldMylisAr = $env:MYLIS_AR
$OldMylisFamily = $env:MYLIS_FAMILY
$OldCC = $env:CC
$OldAR = $env:AR
$Success = $false
$script:StdHelpers = @{}

function Write-Step([string]$Text) {
    Write-Host ""
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Write-Ok([string]$Text) {
    Write-Host "  ok $Text" -ForegroundColor Green
}

function Get-ToolPath([string]$Name) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -eq $cmd) {
        throw "找不到工具 $Name，请确认它在 PATH 中"
    }
    return $cmd.Source
}

function Invoke-Native([string]$Exe, [string[]]$Arguments, [string]$Label) {
    Write-Host ("  > " + $Exe + " " + ($Arguments -join " ")) -ForegroundColor DarkGray
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Label 失败（退出码 $LASTEXITCODE）"
    }
}

function Invoke-Program([string]$Exe, [string[]]$Arguments, [string]$Label) {
    $output = & $Exe @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "$Label 运行失败（退出码 $LASTEXITCODE）"
    }
    return (($output | Out-String).Replace("`r", "").Trim())
}

function Assert-Text([string]$Actual, [string]$Expected, [string]$Label) {
    $actualText = $Actual.Replace("`r", "").Trim()
    $expectedText = $Expected.Replace("`r", "").Trim()
    if ($actualText -ne $expectedText) {
        throw "$Label 输出不一致`n实际：`n$actualText`n期望：`n$expectedText"
    }
    Write-Ok $Label
}

function Assert-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "缺少 $Label：$Path"
    }
    Write-Ok "$Label 存在"
}

function Ensure-MsvcEnvironment {
    if ((Get-Command cl.exe -ErrorAction SilentlyContinue) -and
        (Get-Command lib.exe -ErrorAction SilentlyContinue)) {
        return
    }

    $vcvars = $VcVars
    if ([string]::IsNullOrWhiteSpace($vcvars)) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
        if (-not (Test-Path -LiteralPath $vswhere)) {
            throw "找不到 vswhere.exe，且 cl.exe/lib.exe 不在 PATH。请从 VS 开发者 PowerShell 运行，或用 -VcVars 指定 vcvars64.bat"
        }
        $vsPath = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
        if ([string]::IsNullOrWhiteSpace($vsPath)) {
            throw "vswhere 没有找到带 MSVC x64 工具的 Visual Studio"
        }
        $vcvars = Join-Path $vsPath "VC\Auxiliary\Build\vcvars64.bat"
    }
    if (-not (Test-Path -LiteralPath $vcvars)) {
        throw "vcvars64.bat 不存在：$vcvars"
    }

    Write-Host "加载 MSVC 环境：$vcvars" -ForegroundColor DarkGray
    $cmdLine = "`"$vcvars`" >nul 2>&1 && set"
    $envLines = & cmd.exe /c $cmdLine 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "vcvars64.bat 执行失败"
    }
    foreach ($line in $envLines) {
        if ($line -match '^([^=]+)=(.*)$') {
            Set-Item -Path ("env:" + $matches[1]) -Value $matches[2]
        }
    }
    if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
        throw "vcvars64 已执行，但 cl.exe 仍不在 PATH"
    }
    if (-not (Get-Command lib.exe -ErrorAction SilentlyContinue)) {
        throw "vcvars64 已执行，但 lib.exe 仍不在 PATH"
    }
}

function Assert-MylisEnvironment {
    Write-Step "检查 Windows / MSVC / Rust / GNU make"
    if ([Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
        throw "这个脚本只能在 Windows 上运行"
    }
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw "只支持 64 位 Windows（win64 发行包）"
    }
    Ensure-MsvcEnvironment
    # 显式锁定 MSVC，避免 Git Bash/终端继承的 MYLIS_CC=gcc 或 MYLIS_FAMILY=gcc
    # 让 mylc 误找 Unix 的 libmylrt.a。
    $env:MYLIS_FAMILY = "msvc"
    $env:MYLIS_CC = "cl.exe"
    $env:MYLIS_AR = "lib.exe"
    # Makefile 也使用 CC/AR；终端可能继承 CC=cc，必须一起锁定。
    $env:CC = "cl"
    $env:AR = "lib"
    Write-Ok "cl.exe / lib.exe 可用（MYLIS_FAMILY=msvc, CC=cl, AR=lib）"

    $cargo = Get-ToolPath "cargo"
    Write-Ok "cargo：$cargo"
    $rustup = Get-ToolPath "rustup"
    $targets = & $rustup target list --installed
    if ($LASTEXITCODE -ne 0) {
        throw "rustup target list --installed 失败"
    }
    if (-not ($targets -contains "x86_64-pc-windows-msvc")) {
        throw "缺少 Rust target x86_64-pc-windows-msvc，请执行：rustup target add x86_64-pc-windows-msvc"
    }
    Write-Ok "Rust target x86_64-pc-windows-msvc"
    $make = Get-ToolPath "make"
    Write-Ok "GNU make：$make"

    if (-not $SkipLlvm) {
        $clang = Get-Command clang.exe -ErrorAction SilentlyContinue
        if ($null -eq $clang) {
            throw "默认 LLVM 后端需要 clang.exe（LLVM 15+）。没有 LLVM 时可用 -SkipLlvm，仅测试 MIR-C/AST-C"
        }
        $version = (& $clang.Source --version | Select-Object -First 1)
        Write-Ok "clang：$version"
    }
}

function Build-All {
    Write-Step "构建 bin/（GC=$Gc，编译器 MSVC，mylc 默认后端 LLVM）"
    $make = Get-ToolPath "make"
    $makeArgs = @("all", "GC_BACKEND=$Gc", "CC=cl", "AR=lib")
    Invoke-Native $make $makeArgs "make all"
    Assert-File (Join-Path $Root "bin\mylc.exe") "bin/mylc.exe"
    Assert-File (Join-Path $Root "bin\myls.exe") "bin/myls.exe"
    Assert-File (Join-Path $Root "bin\build\mylrt.lib") "bin/build/mylrt.lib"
    Assert-File (Join-Path $Root "bin\build\mylgc.lib") "bin/build/mylgc.lib"
    Assert-File (Join-Path $Root "bin\build\gc_simple.lib") "bin/build/gc_simple.lib"
    Assert-File (Join-Path $Root "bin\core\Fmt.myl") "bin/core/Fmt.myl"
    Assert-File (Join-Path $Root "bin\std\src\Io.myl") "bin/std/src/Io.myl"
    Assert-File (Join-Path $Root "bin\std-win32\native\event.rs") "bin/std-win32/native/event.rs"
    Assert-File (Join-Path $Root "bin\build\std-win32\native\mylis_file.lib") "bin/build/std-win32/native/mylis_file.lib"
    Assert-File (Join-Path $Root "bin\build\std-win32\native\mylis_net.lib") "bin/build/std-win32/native/mylis_net.lib"
    Assert-File (Join-Path $Root "bin\build\std-win32\native\mylis_io.lib") "bin/build/std-win32/native/mylis_io.lib"
    Assert-File (Join-Path $Root "bin\build\std-win32\native\mylis_time.lib") "bin/build/std-win32/native/mylis_time.lib"
    Assert-File (Join-Path $Root "bin\build\std-win32\native\mylis_event.lib") "bin/build/std-win32/native/mylis_event.lib"
    Assert-File (Join-Path $Root "bin\build\std-win32\native\mylis_process.lib") "bin/build/std-win32/native/mylis_process.lib"
    Write-Ok "bin/ 目录布局完整"
}

function New-SmokeSources {
    Write-Step "生成冒烟测试源码"
    New-Item -ItemType Directory -Force -Path $Work | Out-Null
    $hello = @'
import { println } from "Io.myl";

fn main() -> i32 {
    println("win-smoke");
    return 0;
}
'@
    $features = @'
import { println } from "Io.myl";
import { Fmt, FmtSpec } from "Fmt.myl";

fn main() -> i32 {
    u64 big = 18446744073709551615;
    match big {
        18446744073709551615 => { println("u64-pattern"); }
        _ => { println("u64-bad"); }
    }
    println(Fmt::FormatI64(-42, new FmtSpec(6, -1, FmtSpec::ZERO)));
    return 0;
}
'@
    $lib = @'
pub fn add(i64 a, i64 b) -> i64 {
    return a + b;
}
'@
    $main = @'
import { println } from "Io.myl";
import { add } from "lib";

fn main() -> i32 {
    println("{add(2, 3)}");
    return 0;
}
'@
    Set-Content -Path (Join-Path $Work "hello.myl") -Value $hello -Encoding ASCII
    Set-Content -Path (Join-Path $Work "features.myl") -Value $features -Encoding ASCII
    New-Item -ItemType Directory -Force -Path (Join-Path $Work "libpkg") | Out-Null
    Set-Content -Path (Join-Path $Work "libpkg\lib.myl") -Value $lib -Encoding ASCII
    Set-Content -Path (Join-Path $Work "main.myl") -Value $main -Encoding ASCII
    Write-Ok "hello.myl / features.myl / libpkg"
}

function Test-CompilerAt([string]$CompilerRoot, [string]$Label, [string]$MyPath) {
    Write-Step "$Label：编译与运行"
    $mylc = Join-Path $CompilerRoot "mylc.exe"
    Assert-File $mylc "$Label mylc.exe"

    $env:MYLIS_ROOT = $CompilerRoot
    $env:MYLIS_PATH = $MyPath
    $env:MYLIS_GC = $Gc

    $helloExe = Join-Path $Work "hello-$Label.exe"
    $helloArgs = @((Join-Path $Work "hello.myl"), "-o", $helloExe)
    if ($SkipLlvm) {
        $helloArgs = @((Join-Path $Work "hello.myl"), "--backend", "mir", "-o", $helloExe)
    }
    Invoke-Native $mylc $helloArgs "$Label hello"
    Assert-Text (Invoke-Program $helloExe @() "$Label hello") "win-smoke" "$Label hello 输出"

    $backendList = @("mir", "ast")
    if (-not $SkipLlvm) {
        $backendList = @("llvm") + $backendList
    }
    $defaultExe = Join-Path $Work ("features-default-" + $Label + ".exe")
    $defaultArgs = @((Join-Path $Work "features.myl"), "-o", $defaultExe)
    if ($SkipLlvm) { $defaultArgs = @((Join-Path $Work "features.myl"), "--backend", "mir", "-o", $defaultExe) }
    Invoke-Native $mylc $defaultArgs "$Label default backend"
    Assert-Text (Invoke-Program $defaultExe @() "$Label default features") "u64-pattern`n-00042" "$Label default 输出"
    foreach ($backend in $backendList) {
        $exe = Join-Path $Work ("features-" + $Label + "-" + $backend + ".exe")
        Invoke-Native $mylc @((Join-Path $Work "features.myl"), "--backend", $backend, "-o", $exe) "$Label features ($backend)"
        Assert-Text (Invoke-Program $exe @() "$Label features") "u64-pattern`n-00042" "$Label features ($backend) 输出"
    }
    Write-Ok "$Label 编译器冒烟通过"
}

function Test-LocalMylib {
    Write-Step "本地 .mylib 生成与消费"
    $mylc = Join-Path $Root "bin\mylc.exe"
    $libDir = Join-Path $Work "libpkg"
    $mylib = Join-Path $Work "lib.mylib"
    $env:MYLIS_PATH = (Join-Path $Root "bin") + ";" + (Join-Path $Root "bin\std\src") + ";" + $libDir
    $env:MYLIS_GC = $Gc
    $env:MYLIS_ROOT = (Join-Path $Root "bin")
    Invoke-Native $mylc @((Join-Path $libDir "lib.myl"), "--emit-mylib", $mylib) "emit-mylib"
    Assert-File $mylib "lib.mylib"
    $exe = Join-Path $Work "main.exe"
    Invoke-Native $mylc @((Join-Path $Work "main.myl"), "--link-mylib", $mylib, "-o", $exe) "link-mylib"
    Assert-Text (Invoke-Program $exe @() "link-mylib main") "5" "link-mylib 输出"
    Write-Ok ".mylib 生成、链接、执行通过"
}

function Test-NativeInclude {
    Write-Step "mylis.toml [native] include/dirs/libs"
    $mylc = Join-Path $Root "bin\mylc.exe"
    $proj = Join-Path $Work "native-include"
    New-Item -ItemType Directory -Force -Path (Join-Path $proj "src"), (Join-Path $proj "native") | Out-Null

    $manifest = @'
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
out = "app.exe"
'@
    $bind = @'
@header("answer.h")
pub unsafe fn answer() -> i32;
'@
    $main = @'
import { println_i32 } from "Io.myl";
import { answer } from "native_bind.myl";

fn main() -> i32 {
    unsafe { println_i32(answer()); }
    return 0;
}
'@
    $header = @'
#ifndef ANSWER_H
#define ANSWER_H
int answer(void);
#endif
'@
    $source = @'
#include "answer.h"
int answer(void) { return 42; }
'@
    Set-Content -Path (Join-Path $proj "mylis.toml") -Value $manifest -Encoding ASCII
    Set-Content -Path (Join-Path $proj "src\native_bind.myl") -Value $bind -Encoding ASCII
    Set-Content -Path (Join-Path $proj "src\main.myl") -Value $main -Encoding ASCII
    Set-Content -Path (Join-Path $proj "native\answer.h") -Value $header -Encoding ASCII
    Set-Content -Path (Join-Path $proj "native\answer.c") -Value $source -Encoding ASCII

    Push-Location $proj
    try {
        Invoke-Native "cl.exe" @("/nologo", "/c", "native\answer.c", "/Fo:native\answer.obj") "native-include C 编译"
        Invoke-Native "lib.exe" @("/nologo", "/OUT:native\answer.lib", "native\answer.obj") "native-include lib"
        $env:MYLIS_PATH = (Join-Path $Root "bin") + ";" + (Join-Path $Root "bin\std\src")
        $env:MYLIS_GC = $Gc
        $env:MYLIS_ROOT = (Join-Path $Root "bin")
        Invoke-Native $mylc @("build", "app") "native-include 构建"
        $exe = Join-Path $proj "app.exe"
        Assert-Text (Invoke-Program $exe @() "native-include") "42" "native-include 输出"
        Write-Ok "[native] include/dirs/libs 生效"
    } finally {
        Pop-Location
    }
}

function Get-StdHelper([string]$Name) {
    if ($script:StdHelpers.ContainsKey($Name)) {
        return $script:StdHelpers[$Name]
    }
    $mylc = Join-Path $Root "bin\mylc.exe"
    $helperExe = Join-Path $Work ($Name + ".exe")
    $null = Invoke-Native $mylc @("--std", "--backend", "mir", (Join-Path $Root ("tests\std_native\" + $Name + ".myl")), "-o", $helperExe) $Name
    $script:StdHelpers[$Name] = $helperExe
    return $helperExe
}

function Invoke-StdCase {
    param(
        [string]$Name,
        [string[]]$Backends,
        [string]$Label,
        [string]$Helper = "",
        [int]$Repeat = 1
    )
    $mylc = Join-Path $Root "bin\mylc.exe"
    $source = Join-Path $Root ("tests\std_native\" + $Name + ".myl")
    $expected = Get-Content -LiteralPath (Join-Path $Root ("tests\std_native\" + $Name + ".expected")) -Raw
    if (-not [string]::IsNullOrWhiteSpace($Helper)) {
        $env:WORKER_CMD = Get-StdHelper $Helper
    }
    foreach ($backend in $Backends) {
        $exe = Join-Path $Work ("std-" + $Name + "-" + $backend + ".exe")
        Invoke-Native $mylc @($source, "--std", "--backend", $backend, "-o", $exe) "$Label ($backend)"
        for ($iteration = 0; $iteration -lt $Repeat; $iteration = $iteration + 1) {
            Assert-Text (Invoke-Program $exe @() "$Label ($backend)") $expected "$Label ($backend) 输出"
        }
    }
}

function Test-StdNative {
    Write-Step "Windows 标准库冒烟（File / Time / Process / TCP / UDP / EventLoop / Worker）"
    $oldWorkerCmd = $env:WORKER_CMD
    $oldSleeperCmd = $env:SLEEPER_CMD
    Push-Location $Root
    try {
        $buildDir = Join-Path $Root "build\tests"
        New-Item -ItemType Directory -Force -Path $buildDir | Out-Null
        $mylc = Join-Path $Root "bin\mylc.exe"
        $env:MYLIS_PATH = (Join-Path $Root "bin") + ";" + (Join-Path $Root "bin\std\src")
        $env:MYLIS_GC = $Gc
        $env:MYLIS_ROOT = (Join-Path $Root "bin")

        $sleeperExe = Join-Path $Work "win-sleeper.exe"
        $sleeperC = @'
#include <windows.h>
int main(void) {
    Sleep(10000);
    return 0;
}
'@
        $sleeperSource = Join-Path $Work "win-sleeper.c"
        Set-Content -Path $sleeperSource -Value $sleeperC -Encoding ASCII
        Invoke-Native "cl.exe" @("/nologo", ("/Fe:" + $sleeperExe), ("/Fo:" + (Join-Path $Work "win-sleeper.obj")), $sleeperSource, "/link", "kernel32.lib") "win-sleeper"
        $env:SLEEPER_CMD = $sleeperExe

        $backends = @("mir", "ast")
        if (-not $SkipLlvm) {
            $backends = @("llvm") + $backends
        }
        Invoke-StdCase "WindowsStdApi" @("mir") "WindowsStdApi"
        Invoke-StdCase "EventLoopApi" $backends "EventLoopApi"
        Invoke-StdCase "UdpLoopApi" $backends "UdpLoopApi"
        Invoke-StdCase "TimerApi" $backends "TimerApi"
        Invoke-StdCase "BufferedTcpApi" $backends "BufferedTcpApi"
        Invoke-StdCase "FilePermApi" $backends "FilePermApi"
        Invoke-StdCase "DateTimeApi" $backends "DateTimeApi"
        Invoke-StdCase "RegexApi" $backends "RegexApi"
        Invoke-StdCase "HashApi" $backends "HashApi"
        Invoke-StdCase "GzipApi" $backends "GzipApi"
        Invoke-StdCase "AsyncFileApi" @("mir") "AsyncFileApi"
        Invoke-StdCase "TcpApi" $backends "TcpApi"
        Invoke-StdCase "UdpApi" @("mir", "ast") "UdpApi"
        Invoke-StdCase "FileApi" @("mir") "FileApi"
        Invoke-StdCase "WorkerApi" @("mir") "WorkerApi" -Helper "WorkerEcho"
        Invoke-StdCase "WorkerPoolApi" @("mir") "WorkerPoolApi" -Helper "WorkerEcho"
        Invoke-StdCase "WorkerFlakyApi" @("mir") "WorkerFlakyApi" -Helper "WorkerFlaky" -Repeat 5
        Invoke-StdCase "WorkerReplyApi" @("mir") "WorkerReplyApi" -Helper "WorkerResultServer"
        Invoke-StdCase "WorkerReplyInvalidApi" @("mir") "WorkerReplyInvalidApi" -Helper "WorkerReplyBadServer"
        Invoke-StdCase "WorkerPoolReplyApi" @("mir") "WorkerPoolReplyApi" -Helper "WorkerReplyMixedServer"
        Invoke-StdCase "WindowsProcessApi" @("mir") "WindowsProcessApi"
        Invoke-StdCase "WindowsWriteTimeoutApi" @("mir") "WindowsWriteTimeoutApi"
        Invoke-StdCase "WindowsWorkerWriteTimeoutApi" @("mir") "WindowsWorkerWriteTimeoutApi"
        Write-Ok "Windows 标准库冒烟通过"
    } finally {
        $env:WORKER_CMD = $oldWorkerCmd
        $env:SLEEPER_CMD = $oldSleeperCmd
        Pop-Location
    }
}

function Test-Package {
    Write-Step "构建并验证 Windows 发行 zip"
    $make = Get-ToolPath "make"
    Invoke-Native $make @("dist") "make dist"
    $zip = (Get-ChildItem -Path (Join-Path $Root "dist") -Filter "*.zip" |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1)
    if ($null -eq $zip) {
        throw "make dist 没有生成 zip"
    }
    Write-Ok ("zip：" + $zip.FullName)

    $expand = Join-Path $Work "pkg"
    if (Test-Path -LiteralPath $expand) {
        Remove-Item -Recurse -Force -LiteralPath $expand
    }
    Expand-Archive -Path $zip.FullName -DestinationPath $expand -Force
    $pkgRoot = (Get-ChildItem -Path $expand -Directory | Select-Object -First 1).FullName
    Assert-File (Join-Path $pkgRoot "mylc.exe") "包内 mylc.exe"
    Assert-File (Join-Path $pkgRoot "myls.exe") "包内 myls.exe"
    Assert-File (Join-Path $pkgRoot "core\Fmt.myl") "包内 core/Fmt.myl"
    Assert-File (Join-Path $pkgRoot "std\src\Io.myl") "包内 std/src/Io.myl"
    Assert-File (Join-Path $pkgRoot "build\mylrt.lib") "包内 build/mylrt.lib"
    Assert-File (Join-Path $pkgRoot "build\mylgc.lib") "包内 build/mylgc.lib"
    Assert-File (Join-Path $pkgRoot "build\std-win32\native\mylis_file.lib") "包内 build/std-win32/native/mylis_file.lib"
    Assert-File (Join-Path $pkgRoot "build\std-win32\native\mylis_net.lib") "包内 build/std-win32/native/mylis_net.lib"
    Assert-File (Join-Path $pkgRoot "build\std-win32\native\mylis_io.lib") "包内 build/std-win32/native/mylis_io.lib"
    Assert-File (Join-Path $pkgRoot "build\std-win32\native\mylis_time.lib") "包内 build/std-win32/native/mylis_time.lib"
    Assert-File (Join-Path $pkgRoot "build\std-win32\native\mylis_event.lib") "包内 build/std-win32/native/mylis_event.lib"
    Assert-File (Join-Path $pkgRoot "build\std-win32\native\mylis_process.lib") "包内 build/std-win32/native/mylis_process.lib"
    Assert-File (Join-Path $pkgRoot "mylis-toolchain.json") "包内 mylis-toolchain.json"
    Assert-File (Join-Path $pkgRoot "mylis-build.json") "包内 mylis-build.json"

    $env:MYLIS_PATH = $pkgRoot + ";" + (Join-Path $pkgRoot "std\src")
    $env:MYLIS_ROOT = $pkgRoot
    $env:MYLIS_GC = $Gc
    $pkgMylc = Join-Path $pkgRoot "mylc.exe"
    $pkgExe = Join-Path $Work "packaged-hello.exe"
    $pkgArgs = @((Join-Path $Work "hello.myl"), "-o", $pkgExe)
    if ($SkipLlvm) { $pkgArgs = @((Join-Path $Work "hello.myl"), "--backend", "mir", "-o", $pkgExe) }
    Invoke-Native $pkgMylc $pkgArgs "包内 mylc hello"
    Assert-Text (Invoke-Program $pkgExe @() "包内 hello") "win-smoke" "包内 hello 输出"
    Write-Ok "发行 zip 自包含冒烟通过"
}

try {
    Assert-MylisEnvironment
    if (-not $SkipBuild) {
        Build-All
    } else {
        Write-Step "跳过构建，使用现有 bin/"
        Assert-File (Join-Path $Root "bin\mylc.exe") "bin/mylc.exe"
    }
    Write-Step "工具链与运行时一致性检查"
    $make = Get-ToolPath "make"
    Invoke-Native $make @("check-toolchain") "make check-toolchain"
    Invoke-Native $make @("check-rt") "make check-rt"
    New-SmokeSources
    Test-CompilerAt (Join-Path $Root "bin") "bin" ((Join-Path $Root "bin") + ";" + (Join-Path $Root "bin\std\src"))
    Test-LocalMylib
    Test-NativeInclude
    Test-StdNative
    if ($Mode -ne "quick" -and -not $SkipPackage) {
        Test-Package
    }
    $Success = $true
    Write-Host ""
    Write-Host "Windows/MSVC 测试全部通过" -ForegroundColor Green
} catch {
    Write-Host ""
    Write-Host ("测试失败：" + $_.Exception.Message) -ForegroundColor Red
    Write-Host ("临时目录：" + $Work) -ForegroundColor Yellow
    exit 1
} finally {
    $env:MYLIS_PATH = $OldMylisPath
    $env:MYLIS_GC = $OldMylisGc
    $env:MYLIS_ROOT = $OldMylisRoot
    $env:MYLIS_CC = $OldMylisCc
    $env:MYLIS_AR = $OldMylisAr
    $env:MYLIS_FAMILY = $OldMylisFamily
    $env:CC = $OldCC
    $env:AR = $OldAR
    if ($Success -and -not $Keep -and (Test-Path -LiteralPath $Work)) {
        Remove-Item -Recurse -Force -LiteralPath $Work
    }
}
