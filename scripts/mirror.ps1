# Mirror a staged tree into a destination: copy everything, then move files that exist
# only in the destination (outside target/ and .git/) into a dated side folder, keeping
# their relative paths. Nothing is deleted. Prints one line per moved file and a summary.
param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$Destination,
    [Parameter(Mandatory)][string]$Aside
)
$ErrorActionPreference = 'Stop'
Copy-Item -Recurse -Force (Join-Path $Source '*') $Destination
function Rel($root) {
    Get-ChildItem -Recurse -File -Force $root |
        Where-Object { $_.FullName -notmatch '\\(target|\.git)(\\|$)' } |
        ForEach-Object { $_.FullName.Substring($root.TrimEnd('\').Length + 1) }
}
$keep = @{}
Rel $Source | ForEach-Object { $keep[$_] = $true }
$moved = 0
foreach ($rel in Rel $Destination) {
    if ($keep.ContainsKey($rel)) { continue }
    $target = Join-Path $Aside $rel
    New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
    Move-Item -Force (Join-Path $Destination $rel) $target
    "movido: $rel"
    $moved++
}
"sobras movidas: $moved"
