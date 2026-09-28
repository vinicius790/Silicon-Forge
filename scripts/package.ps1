# Source archive without target/ and .git/ (see package.sh). Argument: output folder.
param([string]$Out)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot 'check_versions.ps1') | Out-Null
if ($LASTEXITCODE -ne 0) { exit 1 }
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $Out) { $Out = Join-Path $root 'target\package' }
New-Item -ItemType Directory -Force $Out | Out-Null
$name = Split-Path -Leaf $root
$archive = Join-Path $Out "mos6502-$version.tgz"
tar -C (Split-Path -Parent $root) --exclude="$name/target" --exclude="$name/.git" -czf $archive $name
if ($LASTEXITCODE -ne 0) { exit 1 }
$listing = tar -tzf $archive
if ($listing -match "^$name/(target|\.git)/") { Write-Error 'arquivo contém target/ ou .git/' }
foreach ($need in 'Cargo.toml', 'LICENSE', 'CHANGELOG.md', 'src/lib.rs') {
    if (-not ($listing -contains "$name/$need")) { Write-Error "falta $need" }
}
$archive
