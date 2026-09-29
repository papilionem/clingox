#!/usr/bin/env bash
# Prepares the tree for a release: sets the version of the five workspace crates and
# of the requirements between them, refreshes Cargo.lock, and renames the
# changelog's Unreleased section (changelog.sh).
#
# Usage: scripts/release/prepare.sh <version>     e.g. 508.2.0-beta.2 or 508.2.1
#
# The maintainer picks the version. Our number encodes the clingo version, so
# nothing derives it from the API check: 508.2.x must stay API compatible, and a
# breaking change waits for a new clingo version or happens between pre-releases.
set -euo pipefail

version="${1:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-(beta|rc)\.[0-9]+)?$ ]] \
  || { echo "usage: $0 <version>, like 508.2.1 or 508.2.0-beta.2" >&2; exit 2; }

cd "$(dirname "$0")/../.."
manifests=(clingox/Cargo.toml clingox-sys/Cargo.toml clingox-derive/Cargo.toml systest/Cargo.toml xtask/Cargo.toml)

old=$(sed -n 's/^version = "\(.*\)"$/\1/p' clingox/Cargo.toml | head -n 1)
[ -n "$old" ] || { echo "cannot read the current version" >&2; exit 1; }
[ "$old" != "$version" ] || { echo "the version is already $version" >&2; exit 1; }

# The package version line, and the version of every requirement on a sibling
# crate (`=old` between the published crates, plain `old` in systest and xtask).
for m in "${manifests[@]}"; do
  sed -i -e "s/^version = \"$old\"$/version = \"$version\"/" \
    -e "/^clingox[a-z-]* = /s/version = \"\(=\?\)$old\"/version = \"\1$version\"/" "$m"
done
cargo update --workspace --offline

# Every crate at the new version, every sibling requirement pointing at it.
bad=$(cargo metadata --offline --format-version 1 | python3 -c '
import json, sys
new = sys.argv[1]
meta = json.load(sys.stdin)
members = set(meta["workspace_members"])
for p in meta["packages"]:
    if p["id"] not in members:
        continue
    if p["version"] != new:
        print(p["name"], "is at", p["version"])
    for d in p["dependencies"]:
        if d["name"].startswith("clingox") and d["req"].lstrip("=^") != new:
            print(p["name"], "requires", d["name"], d["req"])
' "$version")
[ -z "$bad" ] || { echo "$bad" >&2; exit 1; }

scripts/release/changelog.sh rename "$version"
echo "prepared $version (was $old)"
