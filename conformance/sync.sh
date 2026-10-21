#!/bin/sh
# Vendor the TypeScript corpus from the sibling checkout ../canonic and record its commit, the
# commit's date and the date of this sync (CI gates freshness on the last).
set -eu
cd "$(dirname "$0")/.."
src=../canonic/conformance/cases
if [ ! -d "$src" ]; then
  echo "sync.sh: $src not found; check out canonic next to this repo" >&2
  exit 1
fi
if ! git -C ../canonic diff --quiet HEAD -- conformance/cases; then
  echo "sync.sh: ../canonic has uncommitted changes in conformance/cases" >&2
  exit 1
fi
# Everything that can fail runs before any file is touched.
source=$(git -C ../canonic log -1 --format='%H%n%cI')
synced=$(date -u +%Y-%m-%dT%H:%M:%SZ)
rm -rf conformance/cases
cp -R "$src" conformance/cases
printf '%s\n%s\n' "$source" "$synced" > conformance/SOURCE_COMMIT
