# Lines per Rust source file, totals and #[test] count (see report.sh).
$root = Split-Path -Parent $PSScriptRoot
$files = Get-ChildItem -Recurse -Filter *.rs (Join-Path $root 'src')
$files | ForEach-Object { [pscustomobject]@{ Linhas = (Get-Content $_.FullName).Count; Arquivo = $_.FullName.Substring($root.Length + 1) } } | Sort-Object Linhas | Format-Table -AutoSize
$src = ($files | ForEach-Object { (Get-Content $_.FullName).Count } | Measure-Object -Sum).Sum
$testFiles = Get-ChildItem -Recurse -Filter *.rs (Join-Path $root 'tests')
$tests = ($testFiles | ForEach-Object { (Get-Content $_.FullName).Count } | Measure-Object -Sum).Sum
$count = (($files + $testFiles) | Select-String -Pattern '#\[test\]' -AllMatches | ForEach-Object { $_.Matches.Count } | Measure-Object -Sum).Sum
"total src: $src linhas"
"total tests: $tests linhas"
"funções de teste: $count"
