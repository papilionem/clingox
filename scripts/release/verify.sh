#!/usr/bin/env bash
# Checks that the tree is the release <tag> names, before anything is published.
# The release workflow runs it on the tagged commit; it runs the same way locally.
#
# Usage: scripts/release/verify.sh <tag> [--notes <file>]
#
#   - <tag> is `v` + a version in our scheme (DESIGN §10): MAJOR is 100 times
#     clingo's major version plus its minor version, MINOR is clingo's patch
#     version, and a pre-release is `-alpha.N`, `-beta.N` or `-rc.N`;
#   - MAJOR and MINOR name the clingo the bindings were generated from
#     (`CLINGO_VERSION_*` in clingox-sys/src/bindings.rs);
#   - the published crates are exactly clingox-sys, clingox-derive and clingox, every
#     workspace crate has the tag's version, and every requirement between workspace
#     crates asks for it, with `=` in the published ones;
#   - CHANGELOG.md has a non-empty section `## [<version>] - <date>`, which --notes
#     writes to <file> as the release notes.
#
# The version must also be above every other release tag of its line (same
# MAJOR.MINOR), whatever its kind.
#
# Prints `version`, `prerelease` (true or false) and `previous` (the newest earlier
# release tag in this repository, the baseline of the API check, or empty) as
# key=value lines, and appends them to $GITHUB_OUTPUT when that is set. Like
# `cargo xtask semver`, it counts releases, betas and release candidates as
# baselines, and no alpha.
set -euo pipefail

usage() { echo "usage: $0 <tag> [--notes <file>]" >&2; exit 2; }
tag="${1:-}"
[ -n "$tag" ] || usage
shift
notes=""
while [ $# -gt 0 ]; do
  case "$1" in
    --notes) [ $# -ge 2 ] || usage; notes=$2; shift 2 ;;
    *) usage ;;
  esac
done

cd "$(dirname "$0")/../.."
errors=0
fail() { printf 'verify: %s\n' "$*" >&2; errors=$((errors + 1)); }

# The published crates, in the order they are published.
PUBLISHED=(clingox-sys clingox-derive clingox)

# ---- the tag and the clingo it names ----------------------------------------------
num='(0|[1-9][0-9]*)'
if [[ ! "$tag" =~ ^v([1-9][0-9]*)([0-9]{2})\.$num\.$num(-(alpha|beta|rc)\.$num)?$ ]]; then
  echo "verify: '$tag' is not v<clingo major><clingo minor, 2 digits>.<clingo patch>.<release>[-alpha.N|-beta.N|-rc.N]" >&2
  exit 1
fi
version=${tag#v}
clingo_major=${BASH_REMATCH[1]}
clingo_minor=$((10#${BASH_REMATCH[2]}))
clingo_patch=${BASH_REMATCH[3]}
prerelease=false
[[ "$version" == *-* ]] && prerelease=true

macro() {
  sed -n "s/^pub const CLINGO_VERSION_$1: u32 = \([0-9]*\);$/\1/p" clingox-sys/src/bindings.rs
}
bound="$(macro MAJOR).$(macro MINOR).$(macro REVISION)"
if [ "$bound" != "$clingo_major.$clingo_minor.$clingo_patch" ]; then
  fail "$tag names clingo $clingo_major.$clingo_minor.$clingo_patch, but the bindings are for clingo $bound"
fi

# ---- the versions of the crates and of the requirements between them ------------
metadata=$(cargo metadata --no-deps --locked --format-version 1)
problems=$(jq -r --arg v "$version" --arg published "${PUBLISHED[*]}" '
  ($published | split(" ")) as $want
  | [.packages[] | .name] as $members
  | ([.packages[] | select(.publish != []) | .name] | sort) as $have
  | (if $have != ($want | sort) then
       "the published crates are \($have | join(", ")), expected \($want | sort | join(", "))"
     else empty end),
    (.packages[]
     | . as $p
     | (if .version != $v then "\(.name) has version \(.version)" else empty end),
       (.dependencies[]
        | select(.name as $n | $members | index($n))
        | if ($p.publish != []) and .req != "=\($v)" then
            "\($p.name) requires \(.name) \(.req), expected =\($v)"
          elif ($p.publish == []) and (.req | ltrimstr("=") | ltrimstr("^")) != $v then
            "\($p.name) requires \(.name) \(.req), expected \($v)"
          else empty end))
' <<<"$metadata")
if [ -n "$problems" ]; then
  while IFS= read -r line; do fail "$line"; done <<<"$problems"
fi

# ---- the changelog section -----------------------------------------------------------
if ! grep -Eq "^## \[${version//./\\.}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}[[:space:]]*$" CHANGELOG.md; then
  fail "CHANGELOG.md has no section '## [$version] - <date>'"
elif ! text=$(scripts/release/changelog.sh section "$version" 2>/dev/null) \
    || [ -z "$(printf '%s' "$text" | tr -d '[:space:]')" ]; then
  fail "the CHANGELOG.md section of $version is empty"
elif [ -n "$notes" ]; then
  printf '%s\n' "$text" > "$notes"
fi

[ "$errors" -eq 0 ] || { echo "verify: $errors problem(s); $tag is not releasable from this tree" >&2; exit 1; }

# ---- the earlier releases ----------------------------------------------------------
# `sort -V` orders `~` before anything, so `508.2.0~rc.1` sorts below `508.2.0`,
# as a pre-release sorts below its release.
key() { printf '%s\n' "${1#v}" | tr '-' '~'; }
# 0 when version tag $1 is lower than version tag $2.
lower() { [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$(key "$1")" "$(key "$2")" | sort -V | head -n 1)" = "$(key "$1")" ]; }
line="${tag%%-*}"
line="${line%.*}."
previous=""
newest_on_line=""
while IFS= read -r t; do
  [[ "$t" =~ ^v$num\.$num\.$num(-(alpha|beta|rc)\.$num)?$ ]] || continue
  [ "$t" != "$tag" ] || continue
  # The newest release on this line (same MAJOR.MINOR, any kind): the new version
  # must be above it.
  if [[ "$t" == "$line"* ]] && { [ -z "$newest_on_line" ] || lower "$newest_on_line" "$t"; }; then
    newest_on_line=$t
  fi
  # The baseline of the API check: like `cargo xtask semver`, a release, beta or
  # release candidate below this version, and no alpha.
  if [[ "$t" != *-alpha.* ]] && lower "$t" "$tag" && { [ -z "$previous" ] || lower "$previous" "$t"; }; then
    previous=$t
  fi
done < <(git tag --list 'v*')
if [ -n "$newest_on_line" ] && ! lower "$newest_on_line" "$tag"; then
  echo "verify: $tag is not above $newest_on_line, the newest release of ${line%.}.x" >&2
  exit 1
fi

out="version=$version
prerelease=$prerelease
previous=$previous"
printf '%s\n' "$out"
if [ -n "${GITHUB_OUTPUT:-}" ]; then printf '%s\n' "$out" >> "$GITHUB_OUTPUT"; fi
