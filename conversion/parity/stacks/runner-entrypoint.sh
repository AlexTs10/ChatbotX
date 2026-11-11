#!/bin/sh
# Entrypoint of the `parity` runner container: install the harness packages
# when package-lock.json changed, run vitest with the given arguments, then
# give the files vitest wrote (.state/reports, new snapshots) to the host user.
set -u
cd /parity

lock_hash=$(sha256sum package-lock.json | cut -c1-64)
if [ "$(cat node_modules/.parity-lock 2>/dev/null)" != "$lock_hash" ]; then
  npm ci --no-audit --no-fund || exit 1
  echo "$lock_hash" >node_modules/.parity-lock
fi

mkdir -p .state/reports
npx vitest run "$@"
status=$?

chown -R "${PARITY_UID:-1000}:${PARITY_GID:-1000}" .state tests 2>/dev/null || true
exit "$status"
