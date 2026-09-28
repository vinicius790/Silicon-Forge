#!/usr/bin/env bash
# Local CI, mirroring .github/workflows/ci.yml plus the release build and the project
# scripts. Every step runs; the exit code is the number of failed steps (0 = green).
set -uo pipefail
cd "$(dirname "$0")/.."
failed=0
step() {
  local name=$1; shift
  if "$@"; then echo "ok    $name"; else echo "FALHA $name ($?)"; failed=$((failed + 1)); fi
}
step fmt cargo fmt --all -- --check
step clippy cargo clippy --workspace --all-targets --offline -- -D warnings
step test cargo test --offline --quiet
step release cargo build --release --offline --quiet
step versões env BIN=target/release/mos6502 scripts/check_versions.sh
step exemplos scripts/examples.sh
step smoke scripts/smoke.sh
echo "falhas: $failed"
exit "$failed"
