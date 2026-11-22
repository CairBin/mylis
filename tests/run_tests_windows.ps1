# Mylis Windows 回归（不依赖 Git Bash / MSYS fork）
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend ast
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend std
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend mir -Start 0 -Count 8
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend mir -List
# 正向用例编译、运行并比对 .expected；反向用例检查错误码；std 套件用 --std 链接原生侧车。
# -Start/-Count 用于批处理；-List 只输出 COUNT=<总数>，供外层批处理脚本切分。
param(
    [ValidateSet("mir", "ast", "llvm", "std")]
    [string]$Backend = "mir",
    [int]$Start = 0,
    [int]$Count = 0,
    [switch]$List
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Mylc = Join-Path $Root "build\mylc.exe"
if (-not (Test-Path -LiteralPath $Mylc -PathType Leaf)) {
    $Mylc = Join-Path $Root "mylc.exe"   # 发行包里编译器在根目录
}
$Out = Join-Path $Root "build\tests-win-$Backend"
$env:MYLIS_PATH = "$Root;$Root\std\src"

# Windows PowerShell 5.1 会把原生程序的 stderr 当成错误记录；在
# $ErrorActionPreference=Stop 下会直接终止脚本。这里临时降为 Continue，
# 把 stderr 一并写进日志，再把退出码返回给调用方判断。
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

function Invoke-NativeOutput([string]$Exe, [string]$OutPath, [string]$ErrPath) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $Exe > $OutPath 2> $ErrPath
        $code = $LASTEXITCODE
        return $code
    } finally {
        $ErrorActionPreference = $old
    }
}

# Windows PowerShell 5.1 的 Get-Content 默认按 ANSI 读无 BOM 文本，会把 UTF-8
# 的 .expected 读成乱码；原生 stdout 重定向又会写成 UTF-16LE。这里按 BOM 判断
# 编码并统一成按行数组，保证 5.1 / 7.x 下比对结果一致。
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

if ($Backend -eq "std") {
    $stdNames = @("WindowsStdApi", "FileApi", "AsyncFileApi", "TcpApi", "UdpApi", "EventLoopApi", "UdpLoopApi", "TimerApi", "BufferedTcpApi", "FilePermApi", "DateTimeApi", "RegexApi", "HashApi", "GzipApi")
    $cases = @()
    foreach ($name in $stdNames) {
        $cases += [pscustomobject]@{
            Name = $name
            FilePath = Join-Path $Root "tests\std_native\$name.myl"
            Expected = Join-Path $Root "tests\std_native\$name.expected"
        }
    }
} else {
    $cases = @()
    foreach ($file in Get-ChildItem -Path (Join-Path $Root "tests\positive") -Filter "*.myl" | Sort-Object Name) {
        if ($file.BaseName -like "*_bind") { continue }
        $cases += [pscustomobject]@{
            Kind = "positive"
            Name = $file.BaseName
            FilePath = $file.FullName
            Expected = Join-Path $Root ("tests\positive\" + $file.BaseName + ".expected")
        }
    }
    foreach ($file in Get-ChildItem -Path (Join-Path $Root "tests\negative") -Filter "*.myl" | Sort-Object Name) {
        $name = $file.BaseName
        if ($name -like "*_helper" -or $name -like "*_b" -or $name -like "*_bind") { continue }
        $expectFile = Join-Path $Root ("tests\negative\" + $name + ".expect")
        $expected = if (Test-Path -LiteralPath $expectFile) { (Get-Content -LiteralPath $expectFile -Raw).Trim() } else { "MYL" }
        $cases += [pscustomobject]@{
            Kind = "negative"
            Name = $name
            FilePath = $file.FullName
            Expected = $expected
        }
    }
}

if ($List) {
    Write-Output ("COUNT=" + $cases.Count)
    exit 0
}

if (-not (Test-Path -LiteralPath $Mylc -PathType Leaf)) {
    throw "找不到 mylc.exe（build\mylc.exe 或包根 mylc.exe），请先执行 make TOOLCHAIN_FAMILY=mingw all"
}
if ($Start -lt 0 -or $Count -lt 0) { throw "-Start/-Count 不能为负数" }
if ($Start -gt $cases.Count) { throw "-Start $Start 超过用例总数 $($cases.Count)" }
if ($Start -gt 0 -or $Count -gt 0) {
    if ($Count -eq 0) { $Count = $cases.Count - $Start }
    $cases = @($cases | Select-Object -Skip $Start -First $Count)
}
if ($cases.Count -eq 0) { throw "本批没有用例（Start=$Start，Count=$Count）" }
New-Item -ItemType Directory -Force -Path $Out | Out-Null

if ($Backend -eq "std") {
    New-Item -ItemType Directory -Force -Path (Join-Path $Root "build\tests") | Out-Null
    $pass = 0
    $fail = 0
    Write-Host "== Windows std native 侧车用例（--std --backend mir，本批 $($cases.Count) 项）"
    foreach ($case in $cases) {
        $name = $case.Name
        $exe = Join-Path $Out "$name.exe"
        $buildLog = Join-Path $Out "$name.build.log"
        $code = Invoke-NativeLogged $Mylc @("--std", "--backend", "mir", $case.FilePath, "-o", $exe) $buildLog
        if ($code -ne 0) {
            Write-Host "  FAIL $name（编译失败）"
            Get-Content -LiteralPath $buildLog -Tail 8 | ForEach-Object { Write-Host "    $_" }
            $fail++
            continue
        }
        $actual = Join-Path $Out "$name.actual"
        $err = Join-Path $Out "$name.err"
        $code = Invoke-NativeOutput $exe $actual $err
        if ($code -ne 0) {
            Write-Host "  FAIL $name（运行失败，exit=$code）"
            Get-Content -LiteralPath $err -Tail 4 | ForEach-Object { Write-Host "    $_" }
            $fail++
            continue
        }
        $diff = Compare-Object (Get-TextLines $case.Expected) (Get-TextLines $actual)
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

Write-Host "== 用例（$Backend，本批 $($cases.Count) 项）"
foreach ($case in $cases) {
    $name = $case.Name
    if ($case.Kind -eq "positive") {
        $exe = Join-Path $Out "$name.exe"
        $buildLog = Join-Path $Out "$name.build.log"
        $compileArgs = @($backendArgs) + @($case.FilePath, "-o", $exe)
        $code = Invoke-NativeLogged $Mylc $compileArgs $buildLog
        if ($code -ne 0) {
            Write-Host "  FAIL $name（编译失败）"
            Get-Content -LiteralPath $buildLog -Tail 6 | ForEach-Object { Write-Host "    $_" }
            $fail++
            continue
        }
        $actual = Join-Path $Out "$name.actual"
        $err = Join-Path $Out "$name.err"
        $code = Invoke-NativeOutput $exe $actual $err
        if ($code -ne 0) {
            Write-Host "  FAIL $name（运行失败）"
            Get-Content -LiteralPath $err -Tail 4 | ForEach-Object { Write-Host "    $_" }
            $fail++
            continue
        }
        $diff = Compare-Object (Get-TextLines $case.Expected) (Get-TextLines $actual)
        if ($null -ne $diff) {
            Write-Host "  FAIL $name（输出不符）"
            $diff | Select-Object -First 6 | ForEach-Object { Write-Host "    $($_.InputObject)" }
            $fail++
        } else {
            Write-Host "  ok   $name"
            $pass++
        }
    } else {
        $expect = $case.Expected
        $checkLog = Join-Path $Out "$name.check.log"
        $code = Invoke-NativeLogged $Mylc @("--check", $case.FilePath) $checkLog
        if ($code -eq 0) {
            Write-Host "  FAIL $name（本应报错，却通过了检查）"
            $fail++
            continue
        }
        $checkText = (Get-Content -LiteralPath $checkLog -Raw) -replace "`r?`n", ""
        if ($checkText -match [regex]::Escape("error[$expect]")) {
            Write-Host "  ok   $name ( $expect )"
            $pass++
        } else {
            Write-Host "  FAIL $name（未出现 $expect）"
            Get-Content -LiteralPath $checkLog -Tail 4 | ForEach-Object { Write-Host "    $_" }
            $fail++
        }
    }
}

Write-Host ""
Write-Host "通过：$pass    失败：$fail"
if ($fail -ne 0) { exit 1 }
