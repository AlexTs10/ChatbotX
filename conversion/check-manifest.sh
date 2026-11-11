#!/bin/sh
# Run before every conversion commit. Fails if a frozen file changed, was removed, or was added.
set -e
base="$(cd "$(dirname "$0")" && pwd)"
check() {
  dir="$1"
  cd "$base/$dir"
  shasum -a 256 -c --quiet MANIFEST.sha256
  listed=$(wc -l < MANIFEST.sha256 | tr -d ' ')
  actual=$(find . -type f ! -name MANIFEST.sha256 | wc -l | tr -d ' ')
  [ "$listed" = "$actual" ] || { echo "$dir: file count mismatch: manifest=$listed disk=$actual"; exit 1; }
  echo "manifest OK: $dir $listed files"
}
check frozen-tests
check fixtures
