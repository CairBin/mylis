param(
    [Parameter(Mandatory=$true)][string]$Out,
    [Parameter(Mandatory=$true)][string]$Os,
    [Parameter(Mandatory=$true)][string]$Family,
    [Parameter(Mandatory=$true)][string]$Style,
    [Parameter(Mandatory=$true)][string]$Abi,
    [Parameter(Mandatory=$true)][string]$Target,
    [Parameter(Mandatory=$true)][string]$Cc,
    [Parameter(Mandatory=$true)][string]$Ar,
    [Parameter(Mandatory=$true)][string]$Myrt,
    [Parameter(Mandatory=$true)][string]$Mylgc,
    [Parameter(Mandatory=$true)][string]$Simple,
    [string]$SystemLibs = ""
)
$obj = [ordered]@{
    schema = 1
    os = $Os
    family = $Family
    style = $Style
    abi = $Abi
    target = $Target
    cc = $Cc
    ar = $Ar
    myrt = $Myrt
    mylgc = $Mylgc
    simple = $Simple
    system_libs = @($SystemLibs -split '\s+' | Where-Object { $_ -ne "" })
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null
($obj | ConvertTo-Json -Compress) | Set-Content -Path $Out -Encoding ascii
