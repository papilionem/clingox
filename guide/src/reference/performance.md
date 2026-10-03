# Performance

clingox adds a thin layer to clingo, and the work that dominates almost every
program, grounding and solving, runs inside clingo at clingo's speed. The layer
costs something where a program crosses into clingo many times: reading symbols,
iterating atoms, receiving callbacks. This page says how much, and how to
measure your own program.

## What to expect

The numbers below come from the benchmark suite in the repository, run on one
machine on 2026-10-03 against the `clingo` crate (clingo 5.6.2), pyclingo 5.8.2
and clingo's C++ API on the same clingo 5.8.2.
[`docs/dev/BENCHMARKS.md`](https://github.com/papilionem/clingox/blob/main/docs/dev/BENCHMARKS.md)
has the method, the full tables and the caveats, including the version
difference between clingo 5.6.2 and 5.8.2.

- **Solving and grounding are clingo's.** With no callbacks, an end-to-end
  solve takes the same time through clingox, the `clingo` crate and pyclingo,
  within 10%, and within 4% of the C++ API.
- **Symbols and models are close to the `clingo` crate.** Creating, printing and
  hashing symbols is within 8% of the crate, and reading the parts of a symbol
  tree within 16%. pyclingo is 20 to 160 times slower on creation, inspection and
  hashing, and 3 times slower on printing. Enumerating models costs about 190 ns
  per model more than the raw C calls, which shows only when the program reads
  nothing from the model.
- **Callbacks cost about 25 to 165 nanoseconds each.** A ground callback adds
  165 ns, a propagator's `propagate` 140 ns and an observer callback about 26 ns
  to what your own code does. Keep the callback itself cheap and the layer does
  not show.
- **A blocking solve that nothing can interrupt does not start a thread.** When
  the control has no `InterruptHandle` and the solve has no timeout, `solve`
  runs the search inside the call, on the calling thread. A trivial solve on an
  already grounded control then takes about 5 us (5.5 us, against 4.6 us for
  C++). With a live `InterruptHandle`, or with a timeout, the solve starts clasp's
  search thread as before and takes about 35 us (36.6 us, against 34.0 us for
  C++ doing an asynchronous solve). Adding, grounding and solving a trivial
  program on a new control takes 116.8 us, against 103.8 us for C++. The
  difference only matters for a program that makes thousands of tiny solves; one
  large solve takes the same time either way. Create an `InterruptHandle` only when
  you need one. `Application::run` still solves asynchronously and takes about 79 us
  more than a plain control.
- **Walks and repeated reads go through entries.** A path method resolves its
  path from the root on every call, so a single statistics read costs about
  0.5 us. An entry (`Configuration::root` or `entry`, `Statistics::root` or
  `entry`) holds clingo's key for one place in the tree, so a read is one call
  into clingo and a step of a walk with `children` is one to three calls. Walking
  the default configuration (84 nodes, every value read) took 26.8 us through
  entries and 77.6 us by path, and reading one statistics entry 100 times took
  3.2 us through an entry and 47.4 us by path. Use a path for a single read and an
  entry for a walk or a read that repeats. Through entries the configuration walk
  is 1.5 times the C++ API's; by path it is 4.3 times.
- **Walking statistics through entries is not faster than a snapshot.** Every
  child of a map is checked against the map before it is read, so that no
  walk can raise a clingo error that would poison the control; the walk of a
  tree of 164 values took 59.9 us, as long as `Statistics::snapshot` (60.3 us),
  which makes the same check. Take the snapshot when you want every number, and
  entries when you want some of them or read them again.

Every check clingox makes stays in the fast paths: a literal is validated, a
string is checked for NUL bytes and UTF-8, and an error is read from a fresh
error state.

## Where the time goes

Four habits keep a program close to the floor.

- Read a model once. `Model::symbols` returns every symbol in one call, and
  `Model::atoms::<T>` returns them as typed values; calling `contains` for each
  atom you care about costs one call into clingo per atom.
- Read a symbol with `Symbol::kind` when you need several of its parts. Each
  of `name`, `arguments`, `sign` and `as_number` checks the type first, and
  `kind` checks it once.
- Add facts in bulk. `Control::add_facts` takes an iterator, and every call adds
  and grounds a part of its own, so one call with many facts pays that fixed cost
  once.
- Keep `Propagator::propagate` short. It runs for every change to a watched
  literal, on the solver's thread, and a propagator that watches every atom of a
  program that enumerates 65 536 models is called 65 536 times.

## Measuring your program

The suite is a criterion benchmark in `clingox/benches/layer.rs`. It is not part
of `cargo test`:

```text
cargo bench -p clingox --bench layer
cargo bench -p clingox --bench layer -- models
```

For your own program, compare a run with the callback and one without, as the
suite does: the difference is the cost of the callback. Build with `--release`;
a debug build of your own code says nothing about clingox, and clingo itself is
always built optimised.
