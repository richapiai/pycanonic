#!/usr/bin/env bash
# Fails if any tracked or untracked (non-ignored) file contains a forbidden word
# (scripts/forbidden-words.txt, case-insensitive) outside an allowed fixed string
# (scripts/allowed-patterns.txt). Skips LICENSE and the two lists. Fails closed on any git grep
# error.
set -euo pipefail
cd "$(dirname "$0")/.."

words=scripts/forbidden-words.txt
allowed=scripts/allowed-patterns.txt

set +e
raw=$(git grep --untracked -n -i -I -F -f "$words" -- \
    ':!LICENSE' ":!$words" ":!$allowed")
rc=$?
set -e
if [ "$rc" -gt 1 ]; then
    echo "leak-scan: git grep failed (exit $rc)" >&2
    exit "$rc"
fi

[ -r "$allowed" ] || { echo "leak-scan: cannot read $allowed" >&2; exit 2; }
patterns=()
while IFS= read -r pat; do
    [ -n "$pat" ] && patterns+=("$pat")
done < "$allowed"
hits=""
while IFS= read -r line; do
    [ -z "$line" ] && continue
    stripped=$line
    for pat in ${patterns[@]+"${patterns[@]}"}; do
        stripped=${stripped//"$pat"/}
    done
    if grep -q -i -F -f "$words" <<<"$stripped"; then
        hits+="$line"$'\n'
    fi
done <<<"$raw"

if [ -n "$hits" ]; then
    echo "leak-scan: forbidden words found:"
    printf '%s' "$hits"
    exit 1
fi
echo "leak-scan: clean"
