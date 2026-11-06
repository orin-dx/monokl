#!/usr/bin/env bash
# Checks that a published release is complete and usable: tag, GitHub release, crates.io, attestation, and the binary.
# Usage: scripts/verify-release.sh <version>   (for example 0.4.0)
set -uo pipefail

version="${1:?usage: verify-release.sh <version>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
config="$root/callisto.toml"

repo="$(sed -n 's/^forge-repository = "\(.*\)"/\1/p' "$config")"
template="$(sed -n 's/^tag-template = "\(.*\)"/\1/p' "$config")"
expected_assets="$(grep -c '^\[\[release.artifact\]\]' "$config")"
for value in "$repo" "$template"; do
  [[ -n "$value" ]] || { echo "FAIL: could not read forge-repository or tag-template from callisto.toml"; exit 2; }
done
tag="${template//\{version\}/$version}"
crate="${template%%@*}"

failed=0
check() { # description, then a command that succeeds when the check holds
  local what="$1"; shift
  if "$@" > /dev/null 2>&1; then echo "ok    $what"; else echo "FAIL  $what"; failed=1; fi
}

check "tag $tag exists on origin" git -C "$root" ls-remote --exit-code --tags origin "refs/tags/$tag"

release_json="$(gh release view "$tag" --repo "$repo" --json isDraft,isPrerelease,assets 2> /dev/null || true)"
check "GitHub release is published, not a draft or prerelease" \
  jq -e '.isDraft == false and .isPrerelease == false' <<< "$release_json"
check "GitHub release has $expected_assets assets" \
  jq -e --argjson n "$expected_assets" '(.assets | length) == $n' <<< "$release_json"

published="$(curl -s -A "verify-release" "https://crates.io/api/v1/crates/$crate" | jq -r '.crate.max_version' 2> /dev/null || true)"
check "crates.io max version is $version (found: ${published:-none})" test "$published" = "$version"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Linux-aarch64) target="aarch64-unknown-linux-gnu" ;;
  Linux-x86_64) target="x86_64-unknown-linux-gnu" ;;
  *) target="" ;;
esac
asset="$crate-$target.tar.gz"
if [[ -z "$target" ]] || ! jq -e --arg a "$asset" '.assets[] | select(.name == $a)' <<< "$release_json" > /dev/null 2>&1; then
  echo "SKIP  no release asset for this host ($(uname -s)-$(uname -m)): attestation and binary not checked"
else
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  if gh release download "$tag" --repo "$repo" --pattern "$asset" --dir "$work" > /dev/null 2>&1; then
    check "attestation verifies for $asset" gh attestation verify "$work/$asset" --repo "$repo"
    tar -xzf "$work/$asset" -C "$work"
    check "binary reports version $version" test "$("$work/$crate" --version 2> /dev/null)" = "$crate $version"
    mkdir -p "$work/fixture"
    printf 'export function verifyReleaseNeedle() {\n  return 1;\n}\n' > "$work/fixture/a.ts"
    check "binary finds a function in a fresh fixture" \
      bash -c "'$work/$crate' search verifyReleaseNeedle '$work/fixture' | grep -q verifyReleaseNeedle"
  else
    echo "FAIL  could not download $asset"; failed=1
  fi
fi

exit "$failed"
