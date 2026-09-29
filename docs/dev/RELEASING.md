# Releasing

Releases go through a release pull request, which is the human gate, and through
release-plz (`release-plz.toml`, `.github/workflows/release.yml`). Nothing is
published until someone merges that pull request and CI is green for the merge commit.

The three published crates, `clingox-sys`, `clingox-derive` and `clingox`, share one
version and depend on each other with exact `=` requirements. `systest` and `xtask`
carry the same number and are never published.

## The version is the maintainer's choice

The number encodes the clingo version (DESIGN §10): `MAJOR` is 100 times clingo's
major version plus its minor version (508 for clingo 5.8), `MINOR` is clingo's patch
version (2), `PATCH` is our own release counter for that clingo version. While the
API may still change the version carries a pre-release suffix, `-beta.N`; the first
public release is `508.2.0-beta.1`.

Consequences, which the release pull request has to respect:

- Within `508.2.x` no breaking change of the Rust API is allowed, because Cargo treats
  patch releases as compatible. Breaking changes wait for a new clingo version, or
  happen between pre-releases.
- No tool may choose the version. release-plz would derive it from an API check and
  bump the major version, which breaks the encoding. So release-plz is used only to
  publish (`release-plz release`), never to propose versions (`release-pr` is not
  used), and its `release_always = false` makes it publish only the merge of a pull
  request from a branch named `release-plz-*`. It publishes exactly the version
  committed in the manifests; on any other commit it does nothing.
- The API check fails a release instead of raising a number: `cargo xtask semver`
  (cargo-semver-checks against the newest release tag) runs when the pull request is
  prepared and again in CI. A pre-release may break the API when the maintainer
  says so (`allow_breaking`).

## Making a release

1. Keep the "Unreleased" section of `CHANGELOG.md` current in the pull requests that
   change behaviour. Nothing generates it from commit messages.
2. Actions, Release, Run workflow on main, with the version (`508.2.0-beta.2`,
   `508.2.1`, ...). The `prepare` job runs `scripts/release/prepare.sh <version>`
   (the three manifests, the `=` requirements, `systest`, `xtask`, `Cargo.lock`, and
   the changelog: "Unreleased" becomes `[version] - date` above a new empty
   "Unreleased"), runs the API check, and opens the pull request `Release vX.Y.Z` from
   `release-plz-X.Y.Z`. The same script works by hand for a local branch, as long as
   the branch is named `release-plz-<version>`.
3. Read the pull request: versions, the changelog section. The pull request opened
   with the default `GITHUB_TOKEN` does not start CI; store a GitHub App token or a
   personal access token (contents and pull requests write) as the secret
   `RELEASE_PLZ_TOKEN` and it does. Without it, close and reopen the pull request.
4. Merge it (squash or merge commit; the branch name marks it as a release).
5. CI runs on the merge commit. When it has finished, the `release` job checks that
   the CI `conclusion` job of that commit succeeded, then release-plz publishes
   `clingox-sys`, `clingox-derive` and `clingox` in that order (each publish builds
   the package, and `clingox-sys` builds clingo, so this takes a while), tags
   `vX.Y.Z`, and the job creates the GitHub Release (a pre-release for a `-beta.N`
   version) with the changelog section as its notes.

`workflow_run` is used, and not a reusable call of the CI workflow, because CI already
runs on the same push and a call would run the matrix twice.

If CI fails on the merge commit nothing is published. A flaky job: re-run the failed
jobs of that CI run (`gh run rerun --failed <id>`); when the run finishes green the
`release` job starts again for the same commit. A real failure: fix main and prepare a
new pull request with the next version.

## Publishing credentials

crates.io trusted publishing (OIDC) covers every release after the first: the
`release` job has `id-token: write` and exchanges its identity for a short-lived
token with `rust-lang/crates-io-auth-action`.

For each crate, once it exists on crates.io: Settings, Trusted Publishing, add
repository `papilionem/clingox`, workflow `release.yml`, environment `release`.

In the repository settings, create the environment `release` and add the maintainer as a
required reviewer: a second gate on top of the pull request. Also set Settings,
Pages, Source: GitHub Actions, for the guide (`pages.yml`).

## The first release

crates.io offers trusted publishing only for crates that exist, so the first publish
of each of the three crates needs a normal token:

1. On crates.io create an API token limited to publishing new crates (scope
   `publish-new`) and store it as the repository secret `CARGO_REGISTRY_TOKEN`.
   The workflow uses the secret only when it exists, and trusted publishing
   otherwise.
2. Make the release as above, with version `508.2.0-beta.1`: the manifests already
   have it, so `prepare.sh` refuses the same version. Do the first pull request by
   hand instead: branch `release-plz-508.2.0-beta.1`, rename the changelog section with
   `scripts/release/changelog.sh rename 508.2.0-beta.1`, open the pull request, merge
   it when CI is green. The `release` job publishes the three crates and creates tag
   `v508.2.0-beta.1`.
3. Add the trusted publisher to each of the three crates (above), then delete the
   `CARGO_REGISTRY_TOKEN` secret and revoke the token.
4. Update the installation text of the README and the guide from the git dependency
   to `clingox = "=508.2.0-beta.1"` (Cargo selects no pre-release otherwise), and the
   README status line ("not published yet").

The semver check in `cargo xtask check` skips itself until a release tag exists
(`xtask/semver-baseline` names `latest`). From `v508.2.0-beta.1` on it checks the API
against the newest release tag, betas and release candidates included. A baseline
built for another clingo needs that clingo's checkout, see `xtask/src/semver.rs`.

## Yanking

A published version cannot be deleted, only yanked: `cargo yank --version X.Y.Z
clingox`, and the same for `clingox-sys` and `clingox-derive` (the exact requirements
tie the three together, so yank all three). `cargo yank --undo` reverses it. Yanking
stops new dependency resolution onto the version and leaves existing lockfiles
working. Then release a fixed version through the normal pull request (a new `PATCH`,
compatible by the rule above), say so in the changelog, and mark the GitHub Release
as superseded in its notes.

## Checks that do not need a release

- `docs.rs`: `DOCS_RS=1 cargo doc -p clingox-sys` must not build clingo; the
  `docs.rs` workflow checks it on every change to the crates.
- Package contents: `cargo package --list -p clingox-sys` (about 1 MB; the `include`
  list of its manifest names what ships) and `cargo publish --dry-run -p clingox-sys`.
  A dry run of `clingox` itself needs its two dependencies on the registry.
