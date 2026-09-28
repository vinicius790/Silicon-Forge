#!/usr/bin/env bash
# Assemble/run every entry of scripts/examples.tsv and compare the dumped memory.
# BIN (default target/release/mos6502) and MANIFEST may be overridden.
# The self-test stops at test $14 on NMOS because the CLI halts on BRK (test 14);
# $13 on CMOS ($A7 is a NOP there) and $05 on the 2A03 (no decimal mode).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${BIN:-target/release/mos6502}
MANIFEST=${MANIFEST:-scripts/examples.tsv}
[ -x "$BIN" ] || { echo "binário ausente: $BIN" >&2; exit 2; }
pass=0 fail=0
while IFS=$'\t' read -r file kind variant count dump expected; do
  [[ -z "$file" || "$file" == \#* ]] && continue
  got=$("$BIN" "--$kind" "$file" --variant "$variant" -n "$count" --dump-mem "$dump" 2>&1 | grep '^MEM\[' | tail -1 || true)
  if [[ "$got" == "$expected" ]]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FALHA $file ($variant): esperado '$expected', obtido '$got'" >&2
  fi
done < "$MANIFEST"
# VIA/ACIA examples: checked via tests/pe_examples.rs (same map as the CLI `board` command).
if [[ "${DEVICE_EXAMPLES:-0}" == 1 ]]; then
  if cargo test --offline -q --test pe_examples >/dev/null 2>&1; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FALHA tests/pe_examples.rs (via_clock/acia_irq_echo)" >&2
  fi
fi
echo "exemplos: $pass ok, $fail falhas"
[ "$fail" -eq 0 ] && [ "$pass" -gt 0 ]
