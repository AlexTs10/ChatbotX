#!/bin/sh
# Run before every conversion commit. Fails if a frozen file changed, was removed, or was added.
set -e
cd "$(dirname "$0")/frozen-tests"
shasum -a 256 -c --quiet MANIFEST.sha256
listed=$(wc -l < MANIFEST.sha256 | tr -d ' ')
actual=$(find . -type f ! -name MANIFEST.sha256 | wc -l | tr -d ' ')
[ "$listed" = "$actual" ] || { echo "file count mismatch: manifest=$listed disk=$actual"; exit 1; }
echo "manifest OK: $listed files"
