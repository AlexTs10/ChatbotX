#!/usr/bin/env python3
"""P0.5: classify each original test file into class B or C (class A tests are new, in conversion/parity/).

Rules (in order):
  1. feature == "Execute JavaScript flow step"            -> dropped
  2. apps/cli, apps/mcp-server                              -> B  (TypeScript HTTP clients; stay TS, run unchanged against the Rust API)
  3. packages/ui, packages/minigame-ui, packages/analytics-nextjs -> B (frontend libraries)
  4. apps/builder:
       - no DOM rendering and imports/mocks server modules -> C

       - jsdom (no `@vitest-environment node`)              -> B
       - node env and touches server code                   -> C  (oRPC / server actions / route handlers / business / database)
       - node env, pure client logic                        -> B
  5. everything else (worker, realtime, packages/*, integrations/*) -> C
  .snap files take the class of the test file they belong to.
Output: conversion/TEST_MAP.md rows. Run from the repo root.
"""
import re, subprocess, sys, pathlib

root = pathlib.Path(__file__).resolve().parents[2]
feature = dict(l.rstrip("\n").split("\t", 1) for l in open(root / "conversion/feature-of-test.tsv"))
files = sorted(feature)

SERVER_IMPORT = re.compile(  # any quoted module specifier (import, dynamic import, vi.mock) that is server-side
    r"""["'](@chatbotx\.io/(business|database|auth|redis|worker-config|event-bus|scheduler|filesystem|mail|payments|encryption|integration-[a-z-]+)\b"""
    r"""|@orpc/server|server-only|next/server|next/headers|next-intl/server|@/orpc\b|@/lib/safe-action|@/lib/auth\b|@/env\b"""
    r"""|[^"'\s]*/(api|actions?|server|router|orpc|procedures?|queries|mutations)(/[^"']*)?["']|[^"'\s]*\.(action|server|queries)["'])""")
SERVER_PATH = re.compile(r"/(api|actions|server|orpc|router)/|/app/api/|\.action\.|\.server\.|route\.test\.")
DOM = re.compile(r"react-dom|@testing-library|createRoot|renderHook|render\(")
SERVER_HARD = re.compile(
    r"""["'](@/orpc|@orpc/server|@/lib/safe-action|@/middlewares/[^"']*|[^"']*/api/(public|private)[^"']*|[^"']*\.action|[^"']*/actions/[^"']*|[^"']*-action)["']"""
    r"""|vi\.mock\(\s*["']@chatbotx\.io/(business|database/client|redis)""")
# Tests of the dropped feature that live in a kept package (found by the P0.5 verifier).
EXTRA_DROPPED = {"packages/variables/__tests__/interpolate-into-javascript.test.ts"}
CLIENT_APPS = ("apps/cli/", "apps/mcp-server/")
FRONT_PKGS = ("packages/ui/", "packages/minigame-ui/", "packages/analytics-nextjs/")

def classify(path):
    if feature[path] == "Execute JavaScript flow step" or path in EXTRA_DROPPED:
        return "-", "dropped", "feature dropped (DECISIONS 2026-11-11)"
    src = path
    if path.endswith(".snap"):
        src = re.sub(r"/__snapshots__/(.+)\.snap$", r"/\1", path)
    if src.startswith(CLIENT_APPS):
        return "B", "todo", "TS client app; runs unchanged against the Rust API"
    if src.startswith(FRONT_PKGS):
        return "B", "todo", "frontend package"
    if src.startswith("apps/builder/"):
        try:
            text = (root / src).read_text(errors="replace")
        except FileNotFoundError:
            text = ""
        if not DOM.search(text) and (SERVER_HARD.search(text) or SERVER_PATH.search(src)):
            return "C", "todo", "builder server code"
        if "@vitest-environment node" not in text:
            return "B", "todo", "builder jsdom"
        if SERVER_PATH.search(src) or SERVER_IMPORT.search(text):
            return "C", "todo", "builder server code"
        return "B", "todo", "builder node-env, client logic"
    return "C", "todo", ""

rows, counts = [], {}
for f in files:
    cls, status, note = classify(f)
    counts[(cls, status)] = counts.get((cls, status), 0) + 1
    rows.append(f"| `{f}` | {cls} | {'-' if cls != 'C' else 'tbd'} | {status} | {feature[f]}{'; ' + note if note else ''} |")

out = root / "conversion/TEST_MAP.md"
head = out.read_text().split("| Original test file |")[0]
out.write_text(head + "| Original test file | Class | New test location | Status | Feature; note |\n|--------------------|-------|-------------------|--------|---------------|\n" + "\n".join(rows) + "\n")
for k, v in sorted(counts.items()):
    print(k, v)
print("total", len(rows))
