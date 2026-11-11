# Memory (RSS)

Budget: the full system (backend + frontend server) ≤ 200 MB RSS when idle, with one workspace.

| Date | Phase | Stack | Service | RSS | Command | Note |
|------|-------|-------|---------|-----|---------|------|
| 2026-11-11 | 0 | original (Next.js + worker + realtime + Postgres + Redis + RustFS + JS executor) | all | ~2 GB | none (user-set baseline) | User decision: no benchmark of the old stack. Target ≤ 200 MB for the new stack. |
