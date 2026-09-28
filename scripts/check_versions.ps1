# Version consistency (same rules as check_versions.sh). Exit 1 on any mismatch.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$log = Get-Content (Join-Path $root 'CHANGELOG.md')
$status = 0
if (-not ($log -match '^## \[Unreleased\]')) { Write-Error 'CHANGELOG sem [Unreleased]' -ErrorAction Continue; $status = 1 }
$first = ($log | Select-String -Pattern '^## \[(\d[^\]]*)\]' | Select-Object -First 1).Matches[0].Groups[1].Value
if ($first -ne $version) { Write-Error "última seção do CHANGELOG é $first, Cargo.toml diz $version" -ErrorAction Continue; $status = 1 }
if ($env:BIN) {
    if (-not ((& $env:BIN --version) -match "v$version")) { Write-Error "--version não mostra $version" -ErrorAction Continue; $status = 1 }
}
if ($status -eq 0) { "versão $version consistente" }
exit $status
