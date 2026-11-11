# Memory (RSS)

Budget: the full system (backend + frontend server) ≤ 200 MB RSS when idle, with one workspace.

| Date | Phase | Stack | Service | RSS | Command | Note |
|------|-------|-------|---------|-----|---------|------|
| 2026-11-11 | 0 | original (Next.js + worker + realtime + Postgres + Redis + RustFS + JS executor) | all | ~2 GB | none (user-set baseline) | User decision: no benchmark of the old stack. Target ≤ 200 MB for the new stack. |
| 2026-11-11 | 1 | original (parity stack on the Dell, idle after smoke run) | builder 709 MiB, worker 462, postgres 154, realtime 93, fake-upstream 43, rustfs 31, edge 13, redis 7, mailhog 6 | ≈ 1.52 GB | `docker stats --no-stream` (one snapshot by P1.1) | Single snapshot, not a benchmark. User baseline stays 2 GB. |
| 2026-11-11 | 2 | new (crates/jobs example, Dell, release) | jobs scheduler, 17 workers + 44 crons, idle 30 s | 15.7–16.4 MB VmHWM (db layer alone 12.5 MB) | `/proc/self/status` VmHWM | 0.17 wake-ups/s idle without crons |
