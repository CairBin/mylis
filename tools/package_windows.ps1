# 生成 dist/mylis-<ver>-win64.zip（Windows + MSVC）
# 需要：Rust x86_64-pc-windows-msvc target、Visual Studio Build Tools（cl.exe/lib.exe）
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Version = if ($env:VERSION) { $env:VERSION } else { "0.1.0" }
$Name = "mylis-$Version-win64"
$Dist = Join-Path $Root "dist\$Name"
$Target = "x86_64-pc-windows-msvc"
$Out = "target\$Target\release"

Set-Location $Root
Write-Host "== 1/5 准备目录"
if (Test-Path (Join-Path $Root "dist")) { Remove-Item -Recurse -Force (Join-Path $Root "dist") }
foreach ($d in @("build","core","std","std-win32","myrt","gc\lib","gc\include","editors","docs","tests","tools","toolchains","mylc","mylgc","mylrt-rs","build\std-win32\native")) {
    New-Item -ItemType Directory -Force -Path (Join-Path $Dist $d) | Out-Null
}

Write-Host "== 2/5 交叉编译 mylc/myls（MSVC）"
Push-Location (Join-Path $Root "mylc")
& cargo build --release --target $Target
if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }
Pop-Location
Copy-Item (Join-Path $Root "mylc\$Out\mylc.exe") (Join-Path $Dist "mylc.exe")
Copy-Item (Join-Path $Root "mylc\$Out\myls.exe") (Join-Path $Dist "myls.exe")

Write-Host "== 3/5 编译运行时（Rust staticlib + cl 的 simple GC）"
Push-Location (Join-Path $Root "build")
& cl /nologo /std:c11 /O2 /I"$Root\myrt" /c "$Root\myrt\gc\gc_simple.c" /Fo"gc_simple.obj"
if ($LASTEXITCODE -ne 0) { throw "cl 编译 simple GC 失败" }
& lib /nologo /OUT:"$Root\build\gc_simple.lib" gc_simple.obj
if ($LASTEXITCODE -ne 0) { throw "lib 归档失败" }
Pop-Location

Push-Location (Join-Path $Root "mylrt-rs")
& cargo build --release --target $Target
if ($LASTEXITCODE -ne 0) { throw "mylrt-rs 构建失败" }
Pop-Location
Copy-Item (Join-Path $Root "mylrt-rs\$Out\mylrt.lib") (Join-Path $Root "build\mylrt.lib") -Force

Write-Host "== 3b/5 自研 GC（Rust staticlib，MSVC）"
Push-Location (Join-Path $Root "mylgc")
$env:RUSTFLAGS = "$env:RUSTFLAGS -C relocation-model=pic"
& cargo build --release --target $Target
if ($LASTEXITCODE -ne 0) { throw "mylgc 构建失败" }
Pop-Location
Copy-Item (Join-Path $Root "mylgc\$Out\mylgc.lib") (Join-Path $Root "build\mylgc.lib") -Force

Write-Host "== 3c/5 std native 侧车（Rust staticlib，MSVC）"
& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $Root "tools\build_std_native.ps1")
if ($LASTEXITCODE -ne 0) { throw "构建 std native 失败" }

Write-Host "== 4/5 收集文件"
Copy-Item -Recurse -Force (Join-Path $Root "core\*") (Join-Path $Dist "core")
Copy-Item -Recurse -Force (Join-Path $Root "std\*") (Join-Path $Dist "std")
Copy-Item -Recurse -Force (Join-Path $Root "std-win32\*") (Join-Path $Dist "std-win32")
Copy-Item -Recurse -Force (Join-Path $Root "myrt\*") (Join-Path $Dist "myrt")
Copy-Item -Recurse -Force (Join-Path $Root "docs\*") (Join-Path $Dist "docs")
Copy-Item -Recurse -Force (Join-Path $Root "tests\*") (Join-Path $Dist "tests")
Copy-Item -Recurse -Force (Join-Path $Root "editors\*") (Join-Path $Dist "editors")
Copy-Item -Recurse -Force (Join-Path $Root "toolchains\*") (Join-Path $Dist "toolchains")
Copy-Item -Recurse -Force (Join-Path $Root "mylc\src") (Join-Path $Dist "mylc")
Copy-Item (Join-Path $Root "mylc\Cargo.toml") (Join-Path $Dist "mylc")
Copy-Item -Recurse -Force (Join-Path $Root "mylgc\src") (Join-Path $Dist "mylgc")
Copy-Item (Join-Path $Root "mylgc\Cargo.toml") (Join-Path $Dist "mylgc")
Copy-Item -Recurse -Force (Join-Path $Root "mylrt-rs\src") (Join-Path $Dist "mylrt-rs")
Copy-Item (Join-Path $Root "mylrt-rs\Cargo.toml") (Join-Path $Dist "mylrt-rs")
Copy-Item (Join-Path $Root "mylrt-rs\build.rs") (Join-Path $Dist "mylrt-rs")
Copy-Item (Join-Path $Root "README.md"),(Join-Path $Root "Makefile"),(Join-Path $Root "LICENSE") $Dist
Copy-Item (Join-Path $Root "build\mylrt.lib") (Join-Path $Dist "build")
Copy-Item (Join-Path $Root "build\mylgc.lib") (Join-Path $Dist "build")
Copy-Item (Join-Path $Root "build\gc_simple.lib") (Join-Path $Dist "build")
foreach ($name in @("mylis_file","mylis_io","mylis_time","mylis_net","mylis_event","mylis_process")) {
    Copy-Item (Join-Path $Root "build\std-win32\native\$name.lib") (Join-Path $Dist "build\std-win32\native")
}

$json = '{"family":"msvc","cc":"cl.exe","ar":"lib.exe"}'
Set-Content -Path (Join-Path $Dist "mylis-toolchain.json") -Value $json -Encoding ascii

$manifest = [ordered]@{
    schema = 1
    os = "windows"
    family = "msvc"
    style = "msvc"
    abi = "msvc"
    target = $Target
    cc = "cl"
    ar = "lib"
    myrt = "build/mylrt.lib"
    mylgc = "build/mylgc.lib"
    simple = "build/gc_simple.lib"
    system_libs = @("ws2_32.lib","ntdll.lib","userenv.lib","advapi32.lib","bcrypt.lib","ole32.lib","oleaut32.lib","shell32.lib")
} | ConvertTo-Json -Compress
Set-Content -Path (Join-Path $Root "build\mylis-build.json") -Value $manifest -Encoding ascii
Set-Content -Path (Join-Path $Dist "mylis-build.json") -Value $manifest -Encoding ascii

Write-Host "== 5/5 打包"
$zip = Join-Path $Root "dist\$Name.zip"
Compress-Archive -Path $Dist -DestinationPath $zip -Force
Write-Host "包已生成：$zip"
