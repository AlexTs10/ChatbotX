#!/bin/sh
# Prints the generated public spec (gzip + base64) on stdout. Run on the Dell
# from the repo root, through chatbotx-dell:
#   chatbotx-dell sh conversion/tools/public-spec-gen/gen.sh \
#     | sed -n '/^BEGIN-SPEC$/,/^END-SPEC$/p' | sed '1d;$d' | base64 -d | gunzip \
#     > crates/api/assets/public-spec.json
set -e
cp conversion/tools/public-spec-gen/print-public-spec.test.ts apps/builder/__tests__/zz-print-public-spec.test.ts
trap 'rm -f apps/builder/__tests__/zz-print-public-spec.test.ts' EXIT
out=$(mktemp)
(cd apps/builder && PUBLIC_SPEC_OUT="$out" pnpm exec vitest run __tests__/zz-print-public-spec.test.ts >&2)
echo BEGIN-SPEC
gzip -c "$out" | base64
echo END-SPEC
rm -f "$out"
