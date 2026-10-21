#!/usr/bin/env bash
# Fails if any tracked or untracked (non-ignored) file contains a forbidden word.
# Allowed: LICENSE, the word list itself, `authors` metadata lines, and the
# internal design/plan docs under docs/superpowers/.
set -euo pipefail
cd "$(dirname "$0")/.."

hits=$(git grep --untracked -n -i -I -F -f scripts/forbidden-words.txt -- \
    ':!LICENSE' ':!scripts/forbidden-words.txt' ':!docs/superpowers/' \
    | grep -v -E '^[^:]+:[0-9]+:[[:space:]]*authors[[:space:]]*=' || true)

if [ -n "$hits" ]; then
    echo "leak-scan: forbidden words found:"
    echo "$hits"
    exit 1
fi
echo "leak-scan: clean"
