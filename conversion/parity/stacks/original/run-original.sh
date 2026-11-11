#!/usr/bin/env bash
# Boots the original ChatbotX stack in Docker, runs the parity suite against
# it, and stops the stack. Run on the Linux PC from the repo root:
#
#   bash conversion/parity/stacks/original/run-original.sh [options] [-- vitest args]
#
# Options:
#   --no-build          use the images that exist (skip docker compose build)
#   --keep              leave the stack running after the tests (default: down -v)
#   --update-snapshots  pass -u to vitest (record new/changed snapshots)
#   --build-only        build the images one at a time and exit
#   --up-only           boot the stack and exit (implies --keep)
#   --down              stop the stack, remove its volumes, and exit
#   --stats             save one `docker stats --no-stream` snapshot before the tests
# Exit code: the vitest exit code (or non-zero when the stack does not boot).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
parity_dir="$(cd "$here/../.." && pwd)"
state="$parity_dir/.state"
mkdir -p "$state/logs" "$state/reports"

build=1 build_only=0 keep=0 update=0 up_only=0 down_only=0 stats=0
vitest_args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) build=0 ;;
    --keep) keep=1 ;;
    --build-only) build_only=1 ;;
    --update-snapshots) update=1 ;;
    --up-only) up_only=1; keep=1 ;;
    --down) down_only=1 ;;
    --stats) stats=1 ;;
    --) shift; vitest_args=("$@"); break ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done
[[ $update -eq 1 ]] && vitest_args=("${vitest_args[@]}" --update)  # after the filters: `-u <path>` would read the path as its value

if ! docker info >/dev/null 2>&1; then
  echo "docker is not usable by $(id -un). Add the user to group docker:" >&2
  echo "  sudo usermod -aG docker $(id -un)   (then log in again)" >&2
  exit 3
fi

export PARITY_UID="$(id -u)" PARITY_GID="$(id -g)"
compose=(docker compose -p parity-original --project-directory "$parity_dir"
  -f "$parity_dir/stacks/harness.compose.yml" -f "$parity_dir/stacks/original/compose.yml")

log() { printf '[run-original %s] %s\n' "$(date +%H:%M:%S)" "$*"; }

if [[ $down_only -eq 1 ]]; then
  "${compose[@]}" --profile test down -v --remove-orphans
  exit 0
fi

collect_logs() {
  "${compose[@]}" logs --no-color >"$state/logs/stack.log" 2>&1 || true
}

finish() {
  local status=$?
  collect_logs
  if [[ $keep -eq 0 ]]; then
    log "stopping the stack (down -v)"
    "${compose[@]}" --profile test down -v --remove-orphans >/dev/null 2>&1 || true
  else
    log "stack kept running (project parity-original)"
  fi
  exit "$status"
}
if [[ $build_only -eq 0 ]]; then
  trap finish EXIT
fi

if [[ $build -eq 1 ]]; then
  # One image at a time: the PC has 15 GB RAM and `next build` is the peak.
  : >"$state/build-times.txt"
  for service in fake-upstream realtime worker builder; do
    log "building $service"
    started=$(date +%s)
    "${compose[@]}" build "$service"
    seconds=$(( $(date +%s) - started ))
    tag=original
    [[ $service == fake-upstream ]] && tag=local
    size=$(docker image inspect --format '{{.Size}}' "chatbotx-parity/$service:$tag" 2>/dev/null || echo 0)
    printf '%s\t%ss\t%s MB\n' "$service" "$seconds" "$(( size / 1000000 ))" | tee -a "$state/build-times.txt"
  done
fi
[[ $build_only -eq 1 ]] && exit 0

log "starting infra, mailhog and fake upstream"
"${compose[@]}" up -d --wait --wait-timeout 300 postgres redis filesystem mailhog fake-upstream
"${compose[@]}" up -d filesystem-init

log "starting builder (migrate + seed + serve, up to 5 min)"
"${compose[@]}" up -d --wait --wait-timeout 420 builder

log "starting worker, realtime and edge"
"${compose[@]}" up -d --wait --wait-timeout 300 worker realtime app

log "waiting for the worker"
for _ in $(seq 1 120); do
  if "${compose[@]}" logs --no-color worker 2>/dev/null | grep -q "Trigger worker started"; then
    break
  fi
  sleep 1
done
"${compose[@]}" logs --no-color worker 2>/dev/null | grep -q "Trigger worker started" \
  || { log "worker did not log 'Trigger worker started'"; exit 4; }

log "checking DNS + TLS interception inside the app containers"
probe='fetch("https://graph.facebook.com/parity-interception-probe").then(r=>{console.log("https status",r.status);process.exit(r.status===599?0:1)},e=>{console.error(String(e.cause??e));process.exit(1)})'
for service in builder worker realtime; do
  resolved=$("${compose[@]}" exec -T "$service" getent hosts graph.facebook.com | awk '{print $1}' | head -n1)
  if [[ "$resolved" != "1.1.1.1" ]]; then
    log "$service resolves graph.facebook.com to '$resolved', expected 1.1.1.1"
    exit 5
  fi
  "${compose[@]}" exec -T "$service" node -e "$probe" >/dev/null \
    || { log "$service: HTTPS to the fake upstream failed (CA not trusted?)"; exit 5; }
  log "$service: graph.facebook.com -> 1.1.1.1, TLS with test CA ok"
done

if [[ $up_only -eq 1 ]]; then
  log "stack is up: edge http://127.0.0.1:18480, mailhog :18425, fake admin :18410"
  exit 0
fi

if [[ $stats -eq 1 ]]; then
  docker stats --no-stream --format 'table {{.Name}}\t{{.MemUsage}}\t{{.CPUPerc}}' \
    $("${compose[@]}" ps -q) | tee "$state/stats.txt"
fi

log "running the parity suite"
set +e
"${compose[@]}" --profile test run --rm parity "${vitest_args[@]}"
status=$?
set -e
log "vitest exit code $status (reports in $state/reports)"
exit "$status"
