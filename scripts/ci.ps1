# Local CI for Windows (see ci.sh). Exit code = number of failed steps.
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$failed = 0
function Step($name, [scriptblock]$body) {
    & $body
    if ($LASTEXITCODE -eq 0) { "ok    $name" } else { "FALHA $name ($LASTEXITCODE)"; $script:failed++ }
}
Step fmt { cargo fmt --all -- --check }
Step clippy { cargo clippy --workspace --all-targets --offline -- -D warnings }
Step test { cargo test --offline --quiet }
Step release { cargo build --release --offline --quiet }
$env:BIN = Join-Path $root 'target\release\mos6502.exe'
Step versões { & (Join-Path $PSScriptRoot 'check_versions.ps1') }
Step exemplos { & (Join-Path $PSScriptRoot 'examples.ps1') }
"falhas: $failed"
exit $failed
