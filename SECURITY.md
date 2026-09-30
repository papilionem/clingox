# Security policy

## Reporting a vulnerability

Please report security problems privately, through GitHub's private vulnerability
reporting: open the repository's **Security** tab and choose **Report a
vulnerability**. Do not open a public issue or pull request for a suspected
vulnerability.

A useful report has the clingox version, the platform, and the smallest program that
shows the problem. If you can, say which safe API the program calls and what goes
wrong (a crash, an abort, a hang, a wrong result, memory that is read or written
outside its bounds).

We are a small project. Expect an acknowledgement within about a week and a fix or a
reasoned answer within about a month for a confirmed problem. We will tell you when a
fix is released and credit you unless you prefer otherwise.

A fixed vulnerability is announced in a GitHub security advisory on this repository
and in the changelog, and reported to the [RustSec advisory
database](https://github.com/RustSec/advisory-db), so that `cargo audit` and
`cargo deny` warn users of affected versions.

## Supported versions

Only the latest release receives security fixes. While clingox is in pre-release, that
means the latest pre-release. `clingox`, `clingox-sys` and `clingox-derive` are
released together with the same version, and a fix is released for all three.

## What counts as a security issue

clingox promises that code using its safe API cannot cause undefined behaviour. The
following are therefore security issues:

- **Memory-safety bugs reachable from safe clingox APIs.** Any use-after-free, buffer
  overflow, data race, double free or similar defect that a program without `unsafe`
  can trigger, whether the defect is in clingox or in clingo, clasp or gringo and clingox
  could have prevented it.
- **Process-ending behaviour reachable from safe clingox APIs.** An abort, a signal or
  an `_exit` that a safe program can cause and that cannot be caught, for example a
  division trap in the grounder or a callback error that makes clingo end the process.
  clingox either prevents these or documents them; an undocumented case is a bug.
- **Bugs in clingox's checks that let an invalid value reach clingo**, when clingo
  mishandles that value in a way that corrupts memory or ends the process.

The following are usually not security issues, and a normal issue is the right place:

- A wrong answer set or a clean error for a program clingo itself rejects.
- Problems in `unsafe` code paths that are documented as the caller's responsibility
  (the `raw` module is not public API).
- Behaviour of a system clingo that clingox's patches would fix in the vendored build;
  these are listed in [`docs/dev/UPSTREAM-ISSUES.md`](docs/dev/UPSTREAM-ISSUES.md).
- Resource use that grows with the size of the input, such as memory for a large
  program, unless a small input causes an outsized allocation.

If you are not sure whether something qualifies, report it privately; we would rather
receive a report that turns out to be an ordinary bug.

## The vendored clingo

`clingox-sys` ships the source of clingo 5.8.2 (with clasp, gringo, libpotassco and
the header-only libraries listed in `clingox-sys/THIRD-PARTY-LICENSES`) and applies
the patches in `clingox-sys/patches` to it at build time. This code is in scope: a
memory-safety defect in it that safe clingox code can reach is a clingox security
issue, and so is a defect that one of the patches introduces.

Defects that originate in clingo, clasp, gringo or libpotassco are recorded in
[`docs/dev/UPSTREAM-ISSUES.md`](docs/dev/UPSTREAM-ISSUES.md) and reported to the
Potassco project where that is appropriate, so that other users of clingo benefit.
When Potassco releases a fix, clingox moves to that clingo release; until then, a
patch in `clingox-sys/patches` carries the fix for the vendored build. A clingo
installed on the system is outside what clingox can fix: report its defects to
[Potassco](https://github.com/potassco/clingo/issues), and use the vendored build
if a patch exists.

## Verifying a release

Every GitHub release carries the `.crate` files of the three crates exactly as
crates.io serves them, a `SHA256SUMS` file, and a build provenance attestation for
each `.crate` file, signed through Sigstore by the release workflow. To check a
release, with the [GitHub CLI](https://cli.github.com):

```sh
gh release download v508.2.0-beta.2 --repo papilionem/clingox
sha256sum --check SHA256SUMS
gh attestation verify clingox-508.2.0-beta.2.crate --repo papilionem/clingox
```

To compare with what Cargo downloads, fetch the same file from crates.io and check
its hash against `SHA256SUMS`:

```sh
curl -sSfL https://static.crates.io/crates/clingox/clingox-508.2.0-beta.2.crate | sha256sum
```

Cargo itself checks every downloaded `.crate` file against the checksum in the
crates.io index, and records it in `Cargo.lock`.
