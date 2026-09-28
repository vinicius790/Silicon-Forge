#!/usr/bin/env bash
# Source archive for the Drive/release: <out>/mos6502-<version>.tgz, top folder
# 01-02-mos6502, without target/ and .git/. Refuses when versions are inconsistent
# and verifies the archive afterwards.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
out=${1:-$here/target/package}
"$here/scripts/check_versions.sh" >/dev/null
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/Cargo.toml" | head -1)
name=$(basename "$here")
mkdir -p "$out"
archive="$out/mos6502-$version.tgz"
tar -C "$(dirname "$here")" --exclude="$name/target" --exclude="$name/.git" -czf "$archive" "$name"
listing=$(tar -tzf "$archive")
if grep -qE "^$name/(target|\.git)/" <<<"$listing"; then
  echo "arquivo contém target/ ou .git/" >&2; exit 1
fi
for need in Cargo.toml LICENSE CHANGELOG.md src/lib.rs; do
  grep -qx "$name/$need" <<<"$listing" || { echo "falta $need" >&2; exit 1; }
done
echo "$archive"
