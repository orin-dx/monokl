#!/usr/bin/env bash
# Fails when a copy of a fact disagrees with its owner. Owners:
#   Callisto version and plugin commit: .prototools
set -euo pipefail

cd "$(dirname "$0")/.."

status=0
fail() {
  echo "drift: $1" >&2
  status=1
}

version=$(sed -n 's/^callisto = "\([0-9][0-9.]*\)"$/\1/p' .prototools)
commit=$(sed -n 's|.*orin-dx/callisto/\([0-9a-f]\{40\}\)/.*|\1|p' .prototools)
[[ -n "$version" ]] || fail ".prototools has no callisto version"
[[ -n "$commit" ]] || fail ".prototools has no callisto plugin commit"

for file in .github/workflows/*.yml; do
  while read -r found; do
    [[ "$found" == "$version" ]] || fail "$file spells callisto@$found; .prototools pins $version"
  done < <(grep -o 'callisto@[0-9][0-9.]*' "$file" | sed 's/^callisto@//')
  while read -r found; do
    [[ "$found" == "$commit" ]] || fail "$file pins orin-dx/callisto at $found; .prototools pins $commit"
  done < <(grep -o 'orin-dx/callisto/[^@ ]*@[0-9a-f]\{40\}' "$file" | sed 's/.*@//')
done

exit "$status"
