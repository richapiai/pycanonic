#!/bin/sh
# Vendor the TypeScript corpus from the sibling checkout ../canonic and record its commit and date.
set -eu
cd "$(dirname "$0")/.."
src=../canonic/conformance/cases
if [ ! -d "$src" ]; then
  echo "sync.sh: $src not found; check out canonic next to this repo" >&2
  exit 1
fi
rm -rf conformance/cases
cp -R "$src" conformance/cases
git -C ../canonic log -1 --format='%H%n%cI' > conformance/SOURCE_COMMIT
