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

## Supported versions

Only the latest release receives security fixes. While clingox is in pre-release, that
means the latest pre-release tag.

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

## Upstream

Defects that originate in clingo, clasp, gringo or libpotassco are recorded in
[`docs/dev/UPSTREAM-ISSUES.md`](docs/dev/UPSTREAM-ISSUES.md) and reported to the
Potassco project where that is appropriate, so that other users of clingo benefit.
