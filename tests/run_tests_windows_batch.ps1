# 分批运行 Windows 回归：每个批次起一个独立 PowerShell 进程。
# 目的：避免低内存宿主在 100+ 用例的 AST 全量输出时把单个 PowerShell 进程打崩。
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows_batch.ps1 -Backend mir -BatchSize 8
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows_batch.ps1 -Backend ast -BatchSize 8
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows_batch.ps1 -Backend std
param(
    [ValidateSet("mir", "ast", "llvm", "std")]
    [string]$Backend = "mir",
    [int]$BatchSize = 8
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Child = Join-Path $Root "tests\run_tests_windows.ps1"
if ($BatchSize -lt 1) { throw "-BatchSize 必须 >= 1" }
if (-not (Test-Path -LiteralPath $Child -PathType Leaf)) { throw "找不到 $Child" }

# 用当前 PowerShell 宿主自己的 exe 起子进程；Windows PowerShell 5.1 下是 powershell.exe。
$PowerShellExe = Join-Path $PSHOME "pwsh.exe"
if (-not (Test-Path -LiteralPath $PowerShellExe -PathType Leaf)) {
    $PowerShellExe = Join-Path $PSHOME "powershell.exe"
}
if (-not (Test-Path -LiteralPath $PowerShellExe -PathType Leaf)) {
    throw "找不到 PowerShell 可执行文件（PSHOME=$PSHOME）"
}

$script:TotalPass = 0
$script:TotalFail = 0
$script:HadFailure = $false

# 用 Start-Process 重定向到临时文件，避免 Windows PowerShell 5.1 把子进程
# stderr 提升为 NativeCommandError 并在 $ErrorActionPreference=Stop 下终止批处理。
function Invoke-ChildProcess([string[]]$ChildArgs) {
    $outFile = [System.IO.Path]::GetTempFileName()
    $errFile = [System.IO.Path]::GetTempFileName()
    try {
        $proc = Start-Process -FilePath $PowerShellExe -ArgumentList $ChildArgs -NoNewWindow -Wait -PassThru `
            -RedirectStandardOutput $outFile -RedirectStandardError $errFile
        $stdout = if (Test-Path -LiteralPath $outFile) { [System.IO.File]::ReadAllText($outFile) } else { "" }
        $stderr = if (Test-Path -LiteralPath $errFile) { [System.IO.File]::ReadAllText($errFile) } else { "" }
        return [pscustomobject]@{
            ExitCode = $proc.ExitCode
            Text = ($stdout + "`n" + $stderr)
        }
    } finally {
        Remove-Item -LiteralPath $outFile, $errFile -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-Batch([int]$BatchStart, [int]$BatchCount) {
    $result = Invoke-ChildProcess @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$Child`"",
        "-Backend", $Backend, "-Start", "$BatchStart", "-Count", "$BatchCount"
    )
    $text = $result.Text
    $match = [regex]::Match($text, '通过：(\d+)\s+失败：(\d+)')
    $pass = 0
    $fail = 0
    if ($match.Success) {
        $pass = [int]$match.Groups[1].Value
        $fail = [int]$match.Groups[2].Value
    }
    return [pscustomobject]@{
        ExitCode = $result.ExitCode
        HasSummary = $match.Success
        Pass = $pass
        Fail = $fail
        Output = @($text -split "`r?`n")
        Text = $text
    }
}

function Run-BatchRange([int]$BatchStart, [int]$BatchCount) {
    $result = Invoke-Batch $BatchStart $BatchCount
    $end = $BatchStart + $BatchCount - 1
    if ($result.HasSummary) {
        $script:TotalPass += $result.Pass
        $script:TotalFail += $result.Fail
        if ($result.ExitCode -eq 0 -and $result.Fail -eq 0) {
            Write-Host ("  ok   [{0}..{1}] 通过 {2} 失败 0" -f $BatchStart, $end, $result.Pass)
            return
        }
        $script:HadFailure = $true
        Write-Host ("  FAIL [{0}..{1}] 通过 {2} 失败 {3}（exit={4}）" -f $BatchStart, $end, $result.Pass, $result.Fail, $result.ExitCode)
        $result.Output | ForEach-Object { Write-Host "    $_" }
        return
    }

    # 没有汇总行说明子进程在输出最终计数前就崩了；减半重试，定位到单个用例。
    if ($BatchCount -le 1) {
        $script:TotalFail++
        $script:HadFailure = $true
        Write-Host ("  FAIL [{0}] 子进程未输出汇总（exit={1}）" -f $BatchStart, $result.ExitCode)
        $result.Output | ForEach-Object { Write-Host "    $_" }
        return
    }
    $first = [math]::Floor($BatchCount / 2)
    $second = $BatchCount - $first
    Write-Host ("  ..   [{0}..{1}] 未拿到汇总（exit={2}），拆成 {3}+{4} 重试" -f $BatchStart, $end, $result.ExitCode, $first, $second)
    Run-BatchRange $BatchStart $first
    Run-BatchRange ($BatchStart + $first) $second
}

$listResult = Invoke-ChildProcess @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$Child`"",
    "-Backend", $Backend, "-List"
)
$listText = $listResult.Text
$listMatch = [regex]::Match($listText, 'COUNT=(\d+)')
if (-not $listMatch.Success) {
    $listText -split "`r?`n" | ForEach-Object { Write-Host $_ }
    throw "无法获取 $Backend 用例总数"
}
$total = [int]$listMatch.Groups[1].Value
if ($total -le 0) { throw "$Backend 用例总数为 0" }

Write-Host "== Windows 批处理回归（$Backend，共 $total 项，批大小 $BatchSize）"
for ($start = 0; $start -lt $total; $start += $BatchSize) {
    $count = [math]::Min($BatchSize, $total - $start)
    Run-BatchRange $start $count
}

Write-Host ""
Write-Host "总计：通过 $script:TotalPass    失败 $script:TotalFail"
if ($script:HadFailure -or $script:TotalFail -ne 0) { exit 1 }
