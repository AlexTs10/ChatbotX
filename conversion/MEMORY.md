# Memory (RSS)

Budget: the full system (backend + frontend server) ≤ 200 MB RSS when idle, with one workspace.

| Date | Phase | Stack | Service | RSS | Command | Note |
|------|-------|-------|---------|-----|---------|------|
| 2026-11-11 | 0 | original (Next.js + worker + realtime + Postgres + Redis + RustFS + JS executor) | all | ~2 GB | none (user-set baseline) | User decision: no benchmark of the old stack. Target ≤ 200 MB for the new stack. |
| 2026-11-11 | 1 | original (parity stack on the Dell, idle after smoke run) | builder 709 MiB, worker 462, postgres 154, realtime 93, fake-upstream 43, rustfs 31, edge 13, redis 7, mailhog 6 | ≈ 1.52 GB | `docker stats --no-stream` (one snapshot by P1.1) | Single snapshot, not a benchmark. User baseline stays 2 GB. |
