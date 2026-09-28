#!/usr/bin/env bash
# Quick check of the release binary on the two flat demo images (build first: ci.sh
# does). Fails when the dumped bytes differ from the known results.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${BIN:-target/release/mos6502}
[ -x "$BIN" ] || cargo build --release --offline --quiet
expect() {
  local want=$1; shift
  local got
  got=$("$BIN" "$@" | grep '^MEM\[' | tail -1)
  [ "$got" = "$want" ] || { echo "smoke: esperado '$want', obtido '$got'" >&2; exit 1; }
}
expect 'MEM[0010]: 42' --bin examples/hello_lda.bin -n 20 --mode cycles --dump-mem 0x10:0x01
expect 'MEM[0000]: 0F 01' --bin examples/sum_loop.bin -n 200 --mode cycles --dump-mem 0x00:0x02
echo "smoke OK"
