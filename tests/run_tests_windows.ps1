# Mylis Windows 回归（不依赖 Git Bash / MSYS fork）
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend ast
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend std
# 正向用例编译、运行并比对 .expected；反向用例检查错误码；std 套件用 --std 链接原生侧车。
param(
    [ValidateSet("mir", "ast", "llvm", "std")]
    [string]$Backend = "mir"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Mylc = Join-Path $Root "build\mylc.exe"
if (-not (Test-Path -LiteralPath $Mylc -PathType Leaf)) {
    $Mylc = Join-Path $Root "mylc.exe"   # 发行包里编译器在根目录
}
$Out = Join-Path $Root "build\tests-win-$Backend"
$env:MYLIS_PATH = "$Root;$Root\std\src"

if (-not (Test-Path -LiteralPath $Mylc -PathType Leaf)) {
    throw "找不到 mylc.exe（build\mylc.exe 或包根 mylc.exe），请先执行 make TOOLCHAIN_FAMILY=mingw all"
}
New-Item -ItemType Directory -Force -Path $Out | Out-Null

if ($Backend -eq "std") {
    New-Item -ItemType Directory -Force -Path (Join-Path $Root "build\tests") | Out-Null
    $pass = 0
    $fail = 0
    Write-Host "== Windows std native 侧车用例（--std --backend mir）"
    foreach ($name in @("WindowsStdApi", "FileApi", "TcpApi", "UdpApi", "EventLoopApi")) {
        $file = Join-Path $Root "tests\std_native\$name.myl"
        $exe = Join-Path $Out "$name.exe"
        $buildLog = Join-Path $Out "$name.build.log"
        & $Mylc --std --backend mir $file -o $exe *> $buildLog
        if ($LASTEXITCODE -ne 0) {
            Write-Host "  FAIL $name（编译失败）"
            Get-Content -LiteralPath $buildLog -Tail 8 | ForEach-Object { Write-Host "    $_" }
            $fail++
            continue
        }
        $actual = Join-Path $Out "$name.actual"
        $err = Join-Path $Out "$name.err"
        & $exe > $actual 2> $err
        if ($LASTEXITCODE -ne 0) {
            Write-Host "  FAIL $name（运行失败，exit=$LASTEXITCODE）"
            Get-Content -LiteralPath $err -Tail 4 | ForEach-Object { Write-Host "    $_" }
            $fail++
            continue
        }
        $diff = Compare-Object (Get-Content -LiteralPath (Join-Path $Root "tests\std_native\$name.expected")) (Get-Content -LiteralPath $actual)
        if ($null -ne $diff) {
            Write-Host "  FAIL $name（输出不符）"
            $diff | Select-Object -First 6 | ForEach-Object { Write-Host "    $($_.InputObject)" }
            $fail++
        } else {
            Write-Host "  ok   $name"
            $pass++
        }
    }
    Write-Host ""
    Write-Host "通过：$pass    失败：$fail"
    if ($fail -ne 0) { exit 1 }
    exit 0
}

$pass = 0
$fail = 0
$backendArgs = @("--backend", $Backend)
if ($Backend -eq "llvm") { $backendArgs += @("--cc", "clang") }

Write-Host "== 正向用例（$Backend）"
foreach ($file in Get-ChildItem -Path (Join-Path $Root "tests\positive") -Filter "*.myl") {
    $name = $file.BaseName
    if ($name -like "*_bind") { continue }
    $exe = Join-Path $Out "$name.exe"
    $buildLog = Join-Path $Out "$name.build.log"
    & $Mylc @backendArgs $file.FullName -o $exe *> $buildLog
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  FAIL $name（编译失败）"
        Get-Content -LiteralPath $buildLog -Tail 6 | ForEach-Object { Write-Host "    $_" }
        $fail++
        continue
    }
    $actual = Join-Path $Out "$name.actual"
    $err = Join-Path $Out "$name.err"
    & $exe > $actual 2> $err
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  FAIL $name（运行失败）"
        Get-Content -LiteralPath $err -Tail 4 | ForEach-Object { Write-Host "    $_" }
        $fail++
        continue
    }
    $diff = Compare-Object (Get-Content -LiteralPath (Join-Path $Root "tests\positive\$name.expected")) (Get-Content -LiteralPath $actual)
    if ($null -ne $diff) {
        Write-Host "  FAIL $name（输出不符）"
        $diff | Select-Object -First 6 | ForEach-Object { Write-Host "    $($_.InputObject)" }
        $fail++
    } else {
        Write-Host "  ok   $name"
        $pass++
    }
}

Write-Host "== 反向用例（$Backend）"
foreach ($file in Get-ChildItem -Path (Join-Path $Root "tests\negative") -Filter "*.myl") {
    $name = $file.BaseName
    if ($name -like "*_helper" -or $name -like "*_b" -or $name -like "*_bind") { continue }
    $expectFile = Join-Path $Root "tests\negative\$name.expect"
    $expect = if (Test-Path -LiteralPath $expectFile) { (Get-Content -LiteralPath $expectFile -Raw).Trim() } else { "MYL" }
    $checkLog = Join-Path $Out "$name.check.log"
    & $Mylc --check $file.FullName *> $checkLog
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  FAIL $name（本应报错，却通过了检查）"
        $fail++
        continue
    }
    if ((Get-Content -LiteralPath $checkLog -Raw) -match [regex]::Escape("error[$expect]")) {
        Write-Host "  ok   $name ( $expect )"
        $pass++
    } else {
        Write-Host "  FAIL $name（未出现 $expect）"
        Get-Content -LiteralPath $checkLog -Tail 4 | ForEach-Object { Write-Host "    $_" }
        $fail++
    }
}

Write-Host ""
Write-Host "通过：$pass    失败：$fail"
if ($fail -ne 0) { exit 1 }
