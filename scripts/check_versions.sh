#!/usr/bin/env bash
# Version consistency: Cargo.toml, CHANGELOG ([Unreleased] + a section for the version),
# and the binary's --version when BIN is set. Exit 1 on any mismatch.
set -euo pipefail
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || { echo "Cargo.toml sem versão" >&2; exit 1; }
status=0
grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "CHANGELOG sem [Unreleased]" >&2; status=1; }
grep -q "^## \[$version\]" CHANGELOG.md || { echo "CHANGELOG sem seção $version" >&2; status=1; }
first=$(grep -m1 -o '^## \[[0-9][^]]*\]' CHANGELOG.md | tr -d '#[] ')
[ "$first" = "$version" ] || { echo "última seção do CHANGELOG é $first, Cargo.toml diz $version" >&2; status=1; }
if [ -n "${BIN:-}" ]; then
  "$BIN" --version | grep -q "v$version" || { echo "--version não mostra $version" >&2; status=1; }
fi
[ "$status" -eq 0 ] && echo "versão $version consistente"
exit "$status"
