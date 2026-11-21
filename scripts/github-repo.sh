#!/usr/bin/env bash
# Prints OWNER/REPO for a GitHub remote URL, accepting the forms Callisto accepts for `origin`: https://github.com/o/r,
# ssh://[user@]github.com[:port]/o/r and scp-like user@github.com:o/r, each with an optional .git and trailing slash.
# Usage: scripts/github-repo.sh <remote-url>
# Exit codes: 0 printed OWNER/REPO, 2 usage, 3 the URL is not an accepted GitHub remote (the message names it).
set -uo pipefail

url="${1:-}"
[[ $# -eq 1 && -n "$url" ]] || { echo "usage: github-repo.sh <remote-url>" >&2; exit 2; }

n='[A-Za-z0-9_.-]+'
if [[ "$url" =~ ^https://github\.com/($n)/($n)/?$ ]] ||
  [[ "$url" =~ ^ssh://($n@)?github\.com(:[0-9]+)?/($n)/($n)/?$ ]] ||
  [[ "$url" =~ ^$n@github\.com:($n)/($n)/?$ ]]; then
  # The last two captures are always owner and repository.
  last=$((${#BASH_REMATCH[@]} - 1))
  owner="${BASH_REMATCH[last - 1]}"
  repo="${BASH_REMATCH[last]%.git}"
  if [[ -n "$owner" && -n "$repo" ]]; then
    echo "$owner/$repo"
    exit 0
  fi
fi
echo "FAIL: cannot read a GitHub OWNER/REPO from the remote URL '$url' (expected https://github.com/OWNER/REPO, ssh://git@github.com/OWNER/REPO or git@github.com:OWNER/REPO)" >&2
exit 3
