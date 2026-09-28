#!/usr/bin/env bash
# Lines per Rust source file, totals, and the number of #[test] functions.
set -euo pipefail
cd "$(dirname "$0")/.."
find src -name '*.rs' -print0 | xargs -0 wc -l | sort -n | sed '$d'
src=$(find src -name '*.rs' -print0 | xargs -0 cat | wc -l)
tests=$(find tests -name '*.rs' -print0 | xargs -0 cat | wc -l)
count=$(grep -rhoE '#\[test\]' src tests | wc -l)
echo "total src: $src linhas"
echo "total tests: $tests linhas"
echo "funções de teste: $count"
