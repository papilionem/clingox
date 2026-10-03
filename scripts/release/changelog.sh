#!/usr/bin/env bash
# Keeps CHANGELOG.md (Keep a Changelog) in step with a release.
#
#   changelog.sh rename <version> [date]   turns the "Unreleased" section into
#                                          "[<version>] - <date>" and puts a new,
#                                          empty "Unreleased" section above it;
#                                          does nothing when <version> already has one
#   changelog.sh section <version>         prints the notes of <version>: the body
#                                          of its section; fails when it has none
set -euo pipefail

file="${CHANGELOG:-CHANGELOG.md}"
cmd="${1:-}"
version="${2:-}"
if [ -z "$cmd" ] || [ -z "$version" ]; then
  echo "usage: $0 rename|section <version> [date]" >&2
  exit 2
fi

case "$cmd" in
  rename)
    date="${3:-$(date -u +%F)}"
    if grep -Eq "^## \[$version\]" "$file"; then
      echo "$file already has a section for $version"
      exit 0
    fi
    grep -Eq '^## \[?Unreleased\]?' "$file" || { echo "$file has no Unreleased section" >&2; exit 1; }
    tmp=$(mktemp)
    awk -v v="$version" -v d="$date" '
      !done && /^## \[?Unreleased\]?[[:space:]]*$/ {
        print "## Unreleased\n\n## [" v "] - " d
        done = 1
        next
      }
      { print }
    ' "$file" > "$tmp"
    cat "$tmp" > "$file"
    rm -f "$tmp"
    ;;
  section)
    text=$(awk -v v="$version" '
      function heading(l) { return l ~ /^## / }
      heading($0) { on = ($0 ~ "^## \\[" v "\\]") ? 1 : 0; if (on) next }
      on { print }
    ' "$file")
    [ -n "$(printf '%s' "$text" | tr -d '[:space:]')" ] || { echo "no notes for $version in $file" >&2; exit 1; }
    printf '%s\n' "$text"
    ;;
  *) echo "unknown command $cmd" >&2; exit 2 ;;
esac
