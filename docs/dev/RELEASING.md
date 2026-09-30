# Releasing

A release is an annotated version tag, `v<version>`, on a commit of `main`. Pushing
the tag starts the release workflow (`.github/workflows/release.yml`), which checks
the tag, runs the full CI matrix on the tagged commit, publishes the crates to
crates.io and creates the GitHub Release. Nothing else publishes.

The three published crates, `clingox-sys`, `clingox-derive` and `clingox`, share one
version and depend on each other with exact `=` requirements. `systest` and `xtask`
carry the same number and are never published.

## The version is the maintainer's choice

The number encodes the clingo version (DESIGN §10): `MAJOR` is 100 times clingo's
major version plus its minor version (508 for clingo 5.8), `MINOR` is clingo's patch
version (2), `PATCH` is our own release counter for that clingo version. While the
API may still change the version carries a pre-release suffix (`-alpha.N`, `-beta.N`
or `-rc.N`); the first public release is `508.2.0-beta.1`.

Consequences:

- Within `508.2.x` no breaking change of the Rust API is allowed, because Cargo treats
  patch releases as compatible. Breaking changes wait for a new clingo version, or
  happen between pre-releases.
- No tool chooses the version: a bump derived from an API check would raise the major
  version and break the encoding. The API check fails a release instead
  (`cargo xtask semver`, cargo-semver-checks against the previous release tag). A
  finding that is intended is listed, with its reason, in `xtask/semver-allow`,
  under a `baseline <tag>` line naming the release it is against. Against that
  release the list must match the findings exactly; against any later one it is
  ignored, so its entries never outlive the release that follows them.
- The release workflow refuses a tag whose `MAJOR.MINOR` does not name the clingo
  the bindings were generated from.

## Making a release

1. Keep the "Unreleased" section of `CHANGELOG.md` current in the changes that alter
   behaviour. Nothing generates it from commit messages.
2. Make a release commit on `main` that sets the version and names the changelog
   section:

   ```sh
   cargo set-version --workspace 508.2.0-beta.2        # cargo-edit
   scripts/release/changelog.sh rename 508.2.0-beta.2  # "Unreleased" becomes "[508.2.0-beta.2] - <date>"
   scripts/release/verify.sh v508.2.0-beta.2           # the workflow's own checks of the tree
   cargo xtask check
   ```

   `verify.sh` checks the version scheme against the bindings, the version of every
   crate and of every requirement between them, and the changelog section.
3. Optionally rehearse the release (below) on that commit.
4. Tag the commit and push the tag:

   ```sh
   git tag -a v508.2.0-beta.2 -m "clingox 508.2.0-beta.2"
   git push origin v508.2.0-beta.2
   ```

### What the tag starts

| Job | What it does |
|---|---|
| `verify` | The tag matches `v<version>` in the scheme above, and its clingo is the vendored one. Every crate has that version and the `=` requirements agree. The tagged commit is an ancestor of `main`. `CHANGELOG.md` has a non-empty section for the version, saved as the artifact `release-notes`. The API check passes against the previous release tag (it is skipped while none exists). A pre-release is reported as such. |
| `ci`, `docs.rs` | The whole CI workflow (`ci.yml`, called as a reusable workflow) and the docs.rs build (`docs-rs.yml`) on the tagged commit. For a release tag CI runs every job, the ones that otherwise run only on the nightly schedule included; its experimental jobs report without failing, as they do everywhere. |
| `package` | `cargo package --locked` of the three crates, which builds each from its packaged sources, before anything is published. The `.crate` files and their `SHA256SUMS` are kept as the artifact `packages`. |
| `publish` | Only when every job above passed. Runs in the `release` environment, so a required reviewer approves it first. Publishes `clingox-sys`, `clingox-derive` and `clingox` in that order with `--locked`, and after each waits until the crates.io index lists it. A crate already on crates.io at this version is skipped, so a run that failed halfway can be re-run. |
| `GitHub Release` | Downloads the three `.crate` files from crates.io and checks each against the checksum the index records, writes `SHA256SUMS`, creates build provenance attestations for the `.crate` files, and creates the GitHub Release for the tag: the changelog section as notes, marked as a pre-release for an `-alpha`, `-beta` or `-rc` version, with the `.crate` files and `SHA256SUMS` attached. |

If a job fails before `publish`, nothing is published: fix `main`, and release the fix
under the next version (a pushed tag is never moved). A flaky CI job can be re-run
with "Re-run failed jobs"; the run then continues to `publish`. If `publish` fails
after some crates went out, re-run it: the published ones are skipped.

### Rehearsal

Actions, Release, Run workflow, with `tag` set to the release to rehearse (for
example `v508.2.0-beta.2`) and `dry_run` left on. The run uses the commit of the
branch or tag it is started from (usually `main` with the release commit), which
must be on `main`; if the tag exists already, it must point at that commit. It runs
`verify`, the full CI, the docs.rs build and `package`, and stops there: nothing is
published, attested or released, and the `release` environment is not used.

A run with `dry_run` off publishes like a tag push, and only when it is started from
the tag itself (Use workflow from: the tag).

## Security

- The workflow has no permissions by default; each job asks for the least it needs.
  Only `publish` may request an OIDC token for crates.io, and it runs in the
  `release` environment; only `GitHub Release` may write to the repository (the
  release and its attestations).
- Every action is pinned to a full commit SHA. Checkouts do not keep the token
  (`persist-credentials: false`).
- The jobs that package and publish use no cache, so nothing a pull request could
  have written to a cache reaches a published crate.
- Publishing uses crates.io trusted publishing: the job's OIDC identity becomes a
  short-lived token, revoked when the job ends. No crates.io token is stored after
  the first release.
- Tags `v*` are protected by a ruleset (below), so only maintainers can start a
  release, and a tag can be neither moved nor deleted.
- CI runs `zizmor` on the workflows in every pull request.

## One-time setup

In the repository settings on GitHub:

1. **Environment.** Settings, Environments, New environment `release`:
   - Deployment branches and tags: Selected branches and tags, add the tag rule `v*`
     (a publish runs only from a tag; a rehearsal does not use the environment).
   - Required reviewers (optional): the maintainers. A release then waits for one of
     them to approve the `publish` job.
2. **Tag ruleset.** Settings, Rules, Rulesets, New tag ruleset `release tags`:
   enforcement Active; target tags matching `v*`; bypass list: Repository admin (or
   the maintainers' team); rules: Restrict creations, Restrict updates, Restrict
   deletions. Only the bypass list can then create a release tag, and nobody can move
   or delete one.
3. **Pages.** Settings, Pages, Source: GitHub Actions, for the guide (`pages.yml`
   deploys it from every push to `main`).

On crates.io, the first release needs a token, because trusted publishing can only be
configured for a crate that exists:

4. On crates.io, Account Settings, API Tokens, create a token with the scopes
   `publish-new` and `publish-update`, limited to the crates `clingox*`, with a short
   expiry. Store it on GitHub as the secret `CARGO_REGISTRY_TOKEN` of the `release`
   environment (Settings, Environments, `release`, Environment secrets). The workflow
   uses it when it exists and trusted publishing otherwise.
5. Make the first release (`v508.2.0-beta.1`) as above.
6. For each of `clingox-sys`, `clingox-derive` and `clingox` on crates.io: Settings,
   Trusted Publishing, Add, GitHub, with owner `papilionem`, repository `clingox`,
   workflow `release.yml`, environment `release`.
7. Delete the `CARGO_REGISTRY_TOKEN` environment secret and revoke the token on
   crates.io. Every later release uses trusted publishing.
8. Update the installation text of the README and the guide from the git dependency
   to `clingox = "=508.2.0-beta.1"` (Cargo selects no pre-release otherwise), and the
   README status line ("not published yet").

The API check in `cargo xtask check` skips itself until a release tag exists
(`xtask/semver-baseline` names `latest`). From `v508.2.0-beta.1` on it checks the API
against the newest release tag, betas and release candidates included. A baseline
built for another clingo needs that clingo's checkout, see `xtask/src/semver.rs`.

## Verifying a download

Each GitHub Release carries the three `.crate` files exactly as crates.io serves
them, their `SHA256SUMS`, and a build provenance attestation for each file that ties
it to the release workflow run of this repository. To check a `.crate` file, from the
release or from Cargo's download cache (`~/.cargo/registry/cache/*/`), with the
GitHub CLI:

```sh
gh attestation verify clingox-508.2.0-beta.2.crate --repo papilionem/clingox \
  --signer-workflow papilionem/clingox/.github/workflows/release.yml
```

and against the checksums:

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

## Yanking

A published version cannot be deleted, only yanked: `cargo yank --version X.Y.Z
clingox`, and the same for `clingox-sys` and `clingox-derive` (the exact requirements
tie the three together, so yank all three). `cargo yank --undo` reverses it. Yanking
stops new dependency resolution onto the version and leaves existing lockfiles
working. Then release a fixed version (a new `PATCH`, compatible by the rule above),
say so in the changelog, and mark the GitHub Release as superseded in its notes.

## Checks that do not need a release

- `docs.rs`: `DOCS_RS=1 cargo doc -p clingox-sys` must not build clingo; the
  `docs.rs` workflow checks it on every change to the crates.
- Package contents: each published crate names what it ships with an `include`
  list in its manifest (nothing else is packed), and `cargo xtask check` compares
  `cargo package --list` of each with `xtask/package-lists/<crate>.txt`. When a
  change to the packed files is intended, `cargo xtask package-lists --bless`
  rewrites the lists, and the diff shows in review. `clingox-sys` is about 1 MB
  compressed.
- `cargo package --locked -p clingox-sys -p clingox-derive -p clingox` packs and
  builds all three the way the `package` job does.
