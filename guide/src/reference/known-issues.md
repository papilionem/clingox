# Known issues in clingo

These are problems in clingo 5.8.2 itself, not in clingox, that you can run into
through clingox. The full record, with sources, evidence, and whether the clingo
project already tracks each one, is in `docs/dev/UPSTREAM-ISSUES.md` in the
repository.

## Fixed in the vendored build, open with a system clingo

clingox builds clingo from source with small patches for these bugs (the feature
`vendored`, on by default). A clingo installed on the system does not get the
patches, so with one of them these remain:

- **Integer overflow in division.** Program or term text that divides the smallest
  32-bit integer by -1, or takes a remainder by zero in a term (for example the term
  `1\0`), makes clingo raise a hardware exception on x86. The process ends; this
  cannot be caught. The same crash also happens grounding an ordinary rule such as
  `t(X) :- p(X), p(-X+0).` once `p`'s domain holds the smallest 32-bit integer. The
  vendored build treats every one of these divisions as undefined, like a division
  by zero: a term fails to parse, a rule instance is dropped with an "operation
  undefined" message, or, for the rule case, the match simply fails. On AArch64,
  where these divisions previously returned a value instead of trapping, the
  vendored build now also treats them as undefined, so the answer set can differ
  from an unpatched AArch64 build. With a system clingo, run clingox in a separate
  process if you accept program text from untrusted sources.
- **Symbols after `main` returns.** clingo frees its symbol table when the process
  exits, while other threads may still use symbols. The vendored build never frees
  it. With a system clingo, join every thread that uses clingox before `main`
  returns.
- **Statistics on several threads.** clasp registers each kind of statistic the
  first time it is used, without a lock, so controls that first solve on several
  threads at once race inside clasp, which can in rare cases read freed memory.
  This covers your own statistics too, not only clasp's built-in ones: writing a
  new entry with `MutableStatistics::set_value`/`push_array`/`add_map_key` from
  `SolveEventHandler::on_statistics` registers its kind through the exact same,
  unlocked path. The
  vendored build makes the registry thread-safe, but only with the `threads`
  feature on: with it off, clasp compiles under a C++ standard that has no
  `<mutex>`, so the fix cannot take a lock there and the race remains. With
  `threads` off, or with a system clingo, do not use controls from several
  threads at once; solve once on one thread, with the options the other threads
  will use, before starting controls on them.

- **Arithmetic type of `#external`.** Grounding `#external e(X) : f(X). [X\2]`, or
  any type term that is an operator other than a plain sum, difference or
  product with a constant, such as `[X**2]`, `[|X|]` or `[X+X]`, dereferences a null
  pointer in clingo and ends the process; this cannot be caught. The vendored build
  grounds these statements without declaring anything (clingo ignores every type
  other than `true`, `false`, `free` and `release`), and reports an unbound
  variable as unsafe. With a system clingo, run clingox in a separate process if
  you accept program text from untrusted sources.

- **Reference count of a syntax tree node.** Every `Ast` clone adds one to a 32-bit
  count inside clingo, and clingo does not check it. Forgetting four billion clones
  of one node (`std::mem::forget`) wraps the count, after which the next release
  frees a node that handles still use. The vendored build aborts the process at
  that point instead. With a system clingo, do not leak handles in bulk.
- **A range that ends at the largest integer.** A fact such as
  `p(2147483646..2147483647).` never finishes grounding in an unpatched clingo and
  allocates until the process is killed. The vendored build ends the range. With a
  system clingo, keep the upper end of a range below 2147483647.
- **Statistics events without threads, in an application.** In a build without
  threads (WebAssembly without atomics), a solve-event handler on the control that
  `Application::main` receives gets model events but never the statistics or finish
  events. The vendored build delivers them. With a system clingo, read the result
  from the handle instead.
- **A leak when a parallel search is interrupted while splitting.** With
  `--parallel-mode=N,split`, a search that is interrupted leaks the guiding paths
  still queued, 32 bytes each. The vendored build frees them. With a system clingo
  it is a few bytes per interrupted search, not a growing leak.
- **Reified output from a backend writer mixes up its two options.** With
  `BackendWriterKind::REIFY`, an unpatched clingo ignores `reify_steps` on its own
  and adds step numbers for `reify_sccs` too. The vendored build passes each option
  where it belongs, so the output matches the `clingo` command line's
  `--reify-steps` and `--reify-sccs`. With a system clingo, set both options or
  neither.

## Platforms

- **ARM64 Android phones.** clingo 5.8 stores flags in the high bits of pointers,
  which Android 11 and later tag on ARM64. Grounding and solving then give wrong,
  empty results (clingo issues #475 and #540). clingox's Android tests run on x86_64
  and x86 emulators, which cannot show this; the ARM64 build is compiled in CI but
  not run. The clingo maintainers say clingo 6 fixes this.

- **A linker warning on Windows (MSVC).** Every binary that links the vendored clingo
  prints `warning: linker stdout: clingo.lib(control.obj) : MSIL .netmodule or module
  compiled with /GL found; restarting link with /LTCG`. clasp and libpotassco build
  with `/GL` (whole-program optimisation) in Release, so the linker restarts with
  `/LTCG` on its own. It is harmless: the result is the same and only the link
  restarts. It comes from clingo's own Release settings; clingox keeps them
  because building without `/GL` made clingo about 3% slower on larger programs.
  Silence it with `#![allow(linker_messages)]` in the root of the crate that is
  linked (a binary, or a test), or with `RUSTFLAGS="-A linker-messages"`; both were
  tried on Windows CI and removed the message.

## Results to read carefully

- **Interrupted searches are never conclusive.** clingo can report an interrupted
  search as unsatisfiable even when the program has answer sets. clingox reports
  every interrupted search as unknown instead.
- **Integers in program text wrap.** clingo reads `2147483648` in program text as a
  wrapped 32-bit value without an error. Values you pass from Rust are range-checked.
- **Multi-threaded optimisation with `--opt-strategy=usc`.** A few readers of
  clasp's shared best-bound-so-far do not check whether the value they read is
  still current, so in principle a read could mix bounds from different
  generations while another solver thread is updating them. No wrong optimum
  or cost has been observed: a differential run against a single-threaded
  reference found none over thousands of runs. This applies to a system clingo
  the same as the vendored build; it is a race in clasp itself, not something a
  patch changes. If you ever see a wrong optimum with several solver threads
  and `usc`, this is the first thing to suspect; the workaround is
  `--opt-strategy=bb` or one solver thread.

## Behaviour to know about

- **Very deep syntax trees overflow the stack.** clingo releases, prints, copies,
  compares and hashes an abstract syntax tree recursively, one call frame per
  level. On a 2 MiB stack, merely dropping an `Ast` aborts the process at roughly
  9 000 levels of `f(f(...))` and 20 000 of `-(-(...))`. `ast::parse_string`
  itself does not recurse: a million-level term parses. Program text added to a
  control does: in a release build, `Control::add_base` of a fact `p(f(f(...)))`
  aborted the process at 22 500 levels on a 2 MiB stack and at 90 000 on an 8 MiB
  stack, because clingo checks whether the fact is ground recursively, and grounding
  such a fact aborted at 20 000 and 70 000 levels, when clingo prints it for the
  output. Ordinary programs are nowhere near these depths; for generated or
  untrusted input that nests deeply, parse, add, ground and handle the nodes on a
  thread with a large stack (`std::thread::Builder::stack_size`), or in a separate
  process.
  On Windows (MSVC) the frames are larger: building and dropping a chain of 100 000
  nested functions needs more than 128 MiB of stack on x86_64 and more than 32 MiB
  on i686.
- **A backend writer does not report a failed write.** If the file given to
  `Control::register_backend_writer` cannot take the output, for example on a
  full disk, grounding and solving succeed and the file is short or empty.
  Check the file afterwards when it matters.

- **A huge `#project` arity allocates until memory runs out.** `#project p/4294967295.`
  makes clingo build one variable per argument, in text and through a
  `project_signature` node alike. clingox refuses a negative arity in the node
  constructors, since clingo reads it as a huge one, but a large positive arity is
  clingo's own limit and is accepted.
- **A syntax error ends a `Control`.** clingo cannot ground after a parse error, so
  clingox poisons the control. Create a new one.
- **A malformed aspif file ends a `Control` too.** `Control::load_aspif` on a file
  that opens but fails to parse as aspif poisons the control, exactly as a syntax
  error in ordinary program text does: clingo cannot parse or ground anything else
  on it afterward.
- **Rejected option values.** A rejected value leaves the option as it was, except
  for a tester option that was never set: clingo gives it its default.
- **Options that need threads.** `solve.parallel_mode` and three related options do
  not exist in builds without threads, such as the default WebAssembly build.
- **No CPU time in WebAssembly.** `summary.times.cpu` is always 0 there: Emscripten
  has no real `getrusage`. The vendored build does not call it there; an unpatched
  clingo does, and a debug build then prints
  `warning: unsupported syscall: __syscall_getrusage` each time.
- **`is_consequence` only agrees with `is_true` for a shown or projected literal.**
  Outside brave and cautious enumeration, `Model::is_consequence` normally agrees
  with `Model::is_true` on the same literal, but clingo forces `false` for one that
  is not itself shown or projected, even when it is true in the model, in two
  distinct ways: explicit projection (clingo's `--project`, or
  `Control::update_project`; with `a. b. #project a.` solved under `--project`, `b`
  is true in the model but is not a consequence), and a negative literal, even
  without projection at all (clingo only ever treats the positive literal of a
  shown atom as shown, never its negation, so `is_consequence` on a true negative
  literal is `false` for an ordinary program with no projection involved).
- **`PropagateControl::add_watch`/`has_watch`/`remove_watch`, called from several
  solver threads of a non-sequential propagator, race clasp's own internal
  bookkeeping.** clasp reads the master solver's own assignment word with no
  lock to check whether a variable has been eliminated, while that same
  thread may be writing it through its own, ordinary decision-making --
  ThreadSanitizer reports this as a data race. The bit actually read never
  changes once the search starts, so the value observed is always correct
  in practice; this is a formal C++ race, not an observed wrong answer.
  `PropagateInit::add_watch` (only ever called from `init`, which never runs
  concurrently with a solver thread) is not affected, and
  `Control::register_propagator_sequential` avoids it entirely, at the cost
  of serialising every call into the propagator.
- **A solve-event handler that fails must never return `false` to clingo, for any
  of its four events.** clingo's internal solve-event dispatcher treats a `false`
  return from `on_unsat`, `on_statistics` or `on_finish` as fatal: it calls
  `std::_Exit(1)` immediately, with no unwinding and no error reported, ending the
  process as if it had crashed. `on_model`'s `false` has a second, documented-as-safe
  path instead (clingo's ordinary error state), but taking it still corrupts
  clasp's own internal state for an async, multi-threaded search (clingox's own
  blocking `solve_with_events` is one whenever it runs in async mode, with a
  timeout or a live `InterruptHandle`): the next call that
  updates the control reads out of bounds. clingox's own trampoline for
  `Control::solve_with_events`/`solve_yield_with_events`/`solve_async_with_events`
  handles all of this for you: an `Err` or a panic from any of the four
  `SolveEventHandler` methods reaches your code as an ordinary error or a resumed
  panic, never a process abort or corrupted state, because the trampoline never
  returns `false` to clingo at all, for any of the four events. This is worth
  knowing if you ever write your own C or C++ code against `clingo_control_solve`
  directly.
- **`Application::run` and signals.** clingo's application (`clingo_main`)
  installs handlers for SIGINT, SIGTERM and seven other signals and never removes
  them. `Application::run` restores them when it returns, so this is safe in
  normal use. While a run is active, a signal or `--time-limit` expiring ends the
  process with exit code 1 without returning. A signal delivered to another
  thread between clingo clearing its application and the restore (which runs the
  moment clingo returns, before any of your `Drop` code) still
  runs clingo's handler with no application to talk to and crashes. Vendored and
  system builds alike; see [Clingo applications](../how-to/clingo-applications.md).
