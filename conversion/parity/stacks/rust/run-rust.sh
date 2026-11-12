#!/usr/bin/env bash
# Run the parity suite against the Rust stack (spec conversion/specs/P3.5.md, P3.5c).
# Modelled on stacks/original/run-original.sh. On the Linux PC, from the repo root:
#
#   bash conversion/parity/stacks/rust/run-rust.sh                 # build, boot, test, down -v
#   bash conversion/parity/stacks/rust/run-rust.sh --no-build -- tests/p1-06-contacts/00-auth.test.ts
#
# Options: --build-only, --no-build (use the existing binary and images),
# --keep (leave the stack up), --up-only (boot and exit), --down (stop and
# remove volumes), --stats (one `docker stats` snapshot). Arguments after `--`
# go to vitest. Snapshots are recorded on the original stack only, so there is
# no --update-snapshots here. The exit code is the vitest exit code.
#
# Build: `cargo build --release -p chatbotx-server` on the host (the binary is
# mounted into a slim glibc image), plus the fake-upstream image when missing.
# The stack uses the compose project `parity-rust` and the same debug ports as
# the original (edge 127.0.0.1:18480, MailHog :18425, fake admin :18410), so
# only one of the two stacks can run at a time.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
parity_dir="$(cd "$here/../.." && pwd)"
repo_dir="$(cd "$parity_dir/../.." && pwd)"
state="$parity_dir/.state"
mkdir -p "$state/logs" "$state/reports"

build=1 build_only=0 keep=0 up_only=0 down_only=0 stats=0
vitest_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) build=0 ;;
    --keep) keep=1 ;;
    --build-only) build_only=1 ;;
    --up-only) up_only=1; keep=1 ;;
    --down) down_only=1 ;;
    --stats) stats=1 ;;
    --) shift; vitest_args=("$@"); break ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

if ! docker info >/dev/null 2>&1; then
  echo "docker is not usable by $(id -un). Add the user to group docker:" >&2
  echo "  sudo usermod -aG docker $(id -un)   (then log in again)" >&2
  exit 3
fi

export PARITY_UID="$(id -u)" PARITY_GID="$(id -g)"
compose=(docker compose -p parity-rust --project-directory "$parity_dir"
  -f "$parity_dir/stacks/harness.compose.yml" -f "$parity_dir/stacks/rust/compose.yml")
binary="$repo_dir/target/release/chatbotx-server"

log() { printf '[run-rust %s] %s\n' "$(date +%H:%M:%S)" "$*"; }

if [[ $down_only -eq 1 ]]; then
  "${compose[@]}" --profile test down -v --remove-orphans
  exit 0
fi

collect_logs() {
  "${compose[@]}" logs --no-color >"$state/logs/stack-rust.log" 2>&1 || true
}

finish() {
  local status=$?
  collect_logs
  if [[ $keep -eq 0 ]]; then
    log "stopping the stack (down -v)"
    "${compose[@]}" --profile test down -v --remove-orphans >/dev/null 2>&1 || true
  else
    log "stack kept running (project parity-rust)"
  fi
  exit "$status"
}

if [[ $build -eq 1 ]]; then
  log "building chatbotx-server (release)"
  started=$(date +%s)
  (cd "$repo_dir" && cargo build --release -p chatbotx-server)
  printf 'chatbotx-server\t%ss\t%s MB\n' "$(( $(date +%s) - started ))" \
    "$(( $(stat -c %s "$binary") / 1000000 ))" | tee "$state/build-times-rust.txt"
  if ! docker image inspect chatbotx-parity/fake-upstream:local >/dev/null 2>&1; then
    log "building fake-upstream"
    "${compose[@]}" build fake-upstream
  fi
fi
[[ $build_only -eq 1 ]] && exit 0

if [[ ! -x "$binary" ]]; then
  log "no binary at $binary (run without --no-build)"
  exit 4
fi
if docker compose -p parity-original ps -q 2>/dev/null | grep -q .; then
  log "the original stack (project parity-original) is running; stop it first (run-original.sh --down)"
  exit 4
fi

trap finish EXIT

log "starting mailhog and fake upstream"
"${compose[@]}" up -d --wait --wait-timeout 300 mailhog fake-upstream
log "starting the server (migrate + serve) and the edge"
"${compose[@]}" up -d --wait --wait-timeout 300 server app

if [[ $up_only -eq 1 ]]; then
  log "stack is up: edge http://127.0.0.1:18480, mailhog :18425, fake admin :18410"
  exit 0
fi

if [[ $stats -eq 1 ]]; then
  docker stats --no-stream --format 'table {{.Name}}\t{{.MemUsage}}\t{{.CPUPerc}}' \
    $("${compose[@]}" ps -q) | tee "$state/stats-rust.txt"
fi

log "running the parity suite"
set +e
"${compose[@]}" --profile test run --rm parity "${vitest_args[@]}"
status=$?
set -e
log "vitest exit code $status (reports in $state/reports)"
exit "$status"
