# Performance

clingox adds a thin layer to clingo, and the work that dominates almost every
program, grounding and solving, runs inside clingo at clingo's speed. The layer
costs something where a program crosses into clingo many times: reading symbols,
iterating atoms, receiving callbacks. This page says how much, and how to
measure your own program.

## What to expect

The numbers below come from the benchmark suite in the repository, run on one
machine against the `clingo` crate (clingo 5.6.2) and against pyclingo 5.8.2.
[`docs/dev/BENCHMARKS.md`](https://github.com/papilionem/clingox/blob/main/docs/dev/BENCHMARKS.md)
has the method, the full tables and the caveats, including the version
difference between clingo 5.6.2 and 5.8.2.

- **Solving and grounding are clingo's.** With no callbacks, an end-to-end
  solve takes the same time through clingox, the `clingo` crate and pyclingo,
  within 6%.
- **Symbols and models are close to the `clingo` crate.** Creating, printing and
  hashing symbols is within 8% of the crate, and reading the parts of a symbol
  tree within 13%. pyclingo is 20 to 160 times slower on creation, inspection and
  hashing, and 3 times slower on printing. Enumerating models costs about 200 ns
  per model more than the raw C calls, which shows only when the program reads
  nothing from the model.
- **Callbacks cost about 100 to 200 nanoseconds each.** A ground callback adds
  180 ns, a propagator's `propagate` 136 ns and an observer callback about 30 ns
  to what your own code does. Keep the callback itself cheap and the layer does
  not show.
- **Walks and repeated reads go through entries.** A path method resolves its
  path from the root on every call, so a single statistics read costs about
  0.5 us. An entry (`Configuration::root` or `entry`, `Statistics::root` or
  `entry`) holds clingo's key for one place in the tree, so a read is one call
  into clingo and a step of a walk with `children` is one to three calls. In one session,
  walking the default configuration (84 nodes, every value read) took 26 us
  through entries and 75 us by path, and reading one statistics entry 100 times
  took 4.6 us through an entry and 50 us by path. Use a path for a single read
  and an entry for a walk or a read that repeats.
- **Walking statistics through entries is not faster than a snapshot.** Every
  child of a map is checked against the map before it is read, so that no
  walk can raise a clingo error that would poison the control; the walk of a
  tree of 187 entries took 59 us, as long as `Statistics::snapshot` (58 us),
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
