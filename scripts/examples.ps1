# Runs every entry of scripts/examples.tsv (see examples.sh). BIN and MANIFEST may be set.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$bin = if ($env:BIN) { $env:BIN } else { Join-Path $root 'target\release\mos6502.exe' }
$manifest = if ($env:MANIFEST) { $env:MANIFEST } else { Join-Path $root 'scripts\examples.tsv' }
if (-not (Test-Path $bin)) { Write-Error "binário ausente: $bin" -ErrorAction Continue; exit 2 }
$pass = 0; $fail = 0
foreach ($line in Get-Content $manifest) {
    if ($line -eq '' -or $line.StartsWith('#')) { continue }
    $file, $kind, $variant, $count, $dump, $expected = $line -split "`t"
    $got = (& $bin "--$kind" $file --variant $variant -n $count --dump-mem $dump 2>&1 | Where-Object { "$_" -like 'MEM`[*' } | Select-Object -Last 1)
    if ("$got" -eq $expected) { $pass++ } else { $fail++; Write-Host "FALHA $file ($variant): esperado '$expected', obtido '$got'" }
}
"exemplos: $pass ok, $fail falhas"
if ($fail -eq 0 -and $pass -gt 0) { exit 0 } else { exit 1 }
