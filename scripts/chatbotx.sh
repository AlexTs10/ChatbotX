#!/usr/bin/env bash
# One command to build and run ChatbotX: the SPA (apps/web/dist) and the single
# chatbotx-server binary (SQLite under DATA_DIR, no Postgres, Redis or worker).
#
#   scripts/chatbotx.sh              # install, build, run on http://localhost:3000
#   scripts/chatbotx.sh --no-build   # run the existing build
#
# The first run writes random secrets to $DATA_DIR/chatbotx.env (mode 600).
# Any variable already set in the environment wins over that file.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" != "--no-build" ]]; then
  pnpm install --frozen-lockfile
  pnpm --filter web build
  cargo build --release --locked -p chatbotx-server
fi

export DATA_DIR="${DATA_DIR:-./data}"
mkdir -p "$DATA_DIR"
secrets="$DATA_DIR/chatbotx.env"
if [[ ! -f "$secrets" ]]; then
  (umask 077 && {
    echo "BETTER_AUTH_SECRET=$(openssl rand -hex 32)"
    echo "ENCRYPTION_KEY=$(openssl rand -hex 32)"
    echo "REALTIME_BROADCAST_SECRET=$(openssl rand -hex 32)"
  } >"$secrets")
fi
while IFS='=' read -r key value; do
  [[ -n "$key" && -z "${!key:-}" ]] && export "$key=$value"
done <"$secrets"

export PORT="${PORT:-3000}"
export BETTER_AUTH_URL="${BETTER_AUTH_URL:-http://localhost:$PORT}"
export NEXT_PUBLIC_BUILDER_URL="${NEXT_PUBLIC_BUILDER_URL:-$BETTER_AUTH_URL}"
export WEB_DIR="${WEB_DIR:-apps/web/dist}"
exec target/release/chatbotx-server
