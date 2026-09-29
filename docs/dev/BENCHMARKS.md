# Benchmarks

This page records what clingox's own layer costs, how it was measured, how it
compares with the `clingo` crate and with pyclingo, and what was changed to make
it cheaper. It is the answer to "is the wrapper slow?" for the workloads where a
wrapper can matter. It says nothing about clingo's grounding and solving speed,
which no binding changes.

The suite is `clingox/benches/layer.rs`, a criterion benchmark. The comparison
programs are not in the repository (see [Comparison](#comparison)).

## Scope

Grounding and solving run inside clingo and take the same time whichever
language calls them. What a binding adds is the work at the boundary: turning a
Rust value into a symbol and back, reading a model, iterating atoms, and
receiving a callback from clingo. Each workload here is chosen so that this
boundary is most of what is timed, and most come with a baseline that does the
same clingo work without the feature under test (a solve without a propagator, a
grounding without a callback). The difference between the two is the overhead.
Two end-to-end solves at the end of the suite are sanity references only.

## Machine, versions and method

| | |
|---|---|
| CPU | AMD Ryzen 9 3900X (12 cores, 24 threads), frequency governor `powersave` |
| Memory | 31 GiB |
| OS | Fedora Linux, kernel 7.2.7 |
| Rust | 1.98.1, `--release` (criterion's `bench` profile), edition 2024 |
| clingox | 508.2.0-beta.1, vendored clingo 5.8.2 with clingox's patches, features `derive` and `threads` |
| `clingo` crate | 0.8.0 with `clingo-sys` 0.7.2, which builds clingo **5.6.2** from source (`static-linking`) |
| pyclingo | 5.8.2, Python 3.14.7 |
| criterion | 0.8 |

Rust programs ran pinned to one core with `taskset`, with no other benchmark or
test suite running at the same time. Other work on the machine was not stopped, and the
load average during the runs was between 3 and 6 (a mix of builds and idle
processes on other cores). Numbers are therefore good to a few percent, not to
the last digit; where two results are within about 5% of each other, read them
as equal.

Each criterion group uses 20 samples, a 0.5 s warm-up and a 3 s measurement, and
the tables give the **median** of a run of the workload. Setup that is not part
of the workload (creating and grounding a control, for instance) runs outside
the timed region. pyclingo runs the same workloads with `time.perf_counter_ns`
and reports the median of 10 to 20 runs after 3 warm-up runs, with the garbage
collector run before each. Times are for one run of the whole workload, whose
size is in the group heading.

Run the suite with:

```text
cargo bench -p clingox --bench layer                       # everything
cargo bench -p clingox --bench layer -- models             # one group
cargo bench -p clingox --bench layer -- --save-baseline before
cargo bench -p clingox --bench layer -- --baseline before  # compare
```

It is not part of `cargo test` or `cargo xtask test`; `cargo xtask check` only
compiles and lints it.

## Comparison

The same workloads run against two other libraries. Their sources are not in the
repository, because the `clingo` crate builds its own clingo from source and
would add a second native build to every checkout; the workloads are small and
this page describes each of them.

- **The `clingo` crate 0.8.0** (`clingo-sys` 0.7.2) links clingo **5.6.2**, not
  5.8.2. Everything a solver does can differ between the two versions, so a
  ratio near 1.0 says the two wrappers cost the same, and a ratio far from 1.0 on
  a row where clingo does most of the work (a solve, a grounding) says something
  about clingo's versions and not about either wrapper. The row
  "`Control` add, ground, solve" is an example: pyclingo, which links 5.8.2, takes
  as long as clingox, and both take 55 to 60 us more than the crate, so that gap
  is clingo 5.8.
- **pyclingo 5.8.2** runs the identical clingo as clingox, so its rows compare the
  languages directly. It is the oracle for behaviour in the test suite and is
  included here as the tool most users of clingo start from.

Where a library has no equivalent of a workload, the cell says n/a. In the crate,
the syntax tree rewrites and the typed model reader have no counterpart the
benchmark could use, its `Application` trait is commented out in 0.8.0, and its
`ToSymbol` derive has no reverse. The workloads that do not map one to one:

- **Syntax trees, parse.** The crate's row counts the statements it is handed;
  clingox and pyclingo keep every statement, which is what a rewrite needs.
- **Configuration walk.** clingox reads by path (`solve.models`), which clasp
  resolves from the root on every call; the crate walks with numeric keys. The
  clingo version also differs (84 nodes in 5.8.2, 83 in 5.6.2), and pyclingo's
  walk visits 131 nodes, so its time is only indicative.
- **Statistics.** pyclingo's `statistics` returns a lazy view, so its cell
  measures creating the view, not reading the tree, so the table leaves it out.
- **Facts.** `add_facts` has no counterpart, so the other libraries convert their
  values to symbols, print them and add the text, which is what `add_facts` does.
- **Propagator and observer.** Each library gets a propagator that watches every
  atom and counts its calls, and an observer that counts its rule calls, both
  written in the library's own idiom.

## Results

Times are for one run of the workload, at the size in the group name. "Now" is
the code in this repository; "before" is the commit the optimisations started
from (`e9cb2f6`), measured with the same benchmark file in the same session, so
the ratio is not affected by drift between sessions. A ratio below 1 means clingox
now is faster than the other column.

| Workload | clingox before | clingox now | now/before | clingo crate | pyclingo | now/crate | now/py |
|---|--:|--:|--:|--:|--:|--:|--:|
| **Symbols (1000 per run)** | | | | | | | |
| create a number | 3.2 us | 3.4 us | 1.07 | 3.4 us | 547.4 us | 1.00 | 0.01 |
| create `p(i,1)` | 132.2 us | 63.3 us | 0.48 | 68.7 us | 2.71 ms | 0.92 | 0.02 |
| create a string | 103.0 us | 44.9 us | 0.44 | 43.7 us | 938.0 us | 1.03 | 0.05 |
| create `p(i,f(i,"s"),(1,))` | 499.5 us | 233.0 us | 0.47 | 250.8 us | 7.58 ms | 0.93 | 0.03 |
| parse a term | 2.73 ms | 2.01 ms | 0.74 | 1.89 ms | 2.50 ms | 1.06 | 0.80 |
| print a nested symbol | 984.6 us | 790.3 us | 0.80 | 775.6 us | 2.42 ms | 1.02 | 0.33 |
| read name, arguments and numbers of a tree | 681.3 us | 202.1 us | 0.30 | 179.5 us | 15.33 ms | 1.13 | 0.01 |
| hash, `<`, `==` of neighbours | 19.2 us | 17.9 us | 0.94 | 18.7 us | 835.5 us | 0.96 | 0.02 |
| `#[derive(ToSymbol)]` on `Edge` | 264.5 us | 136.5 us | 0.52 | 157.1 us | n/a | 0.87 | n/a |
| **Models (1024 models of 110 atoms)** | | | | | | | |
| enumerate, count | 614.2 us | 567.9 us | 0.92 | 350.0 us | 1.44 ms | 1.62 | 0.39 |
| read the shown symbols | 3.21 ms | 3.01 ms | 0.94 | 2.73 ms | 5.68 ms | 1.10 | 0.53 |
| read and print every symbol | 65.92 ms | 47.12 ms | 0.71 | 48.86 ms | 245.43 ms | 0.96 | 0.19 |
| `contains` for one atom | 763.8 us | 675.3 us | 0.88 | 432.7 us | 2.42 ms | 1.56 | 0.28 |
| `atoms::<P>()` (sort and convert) | 52.35 ms | 14.91 ms | 0.28 | n/a | n/a | n/a | n/a |
| `solve_all`, owned models | 16.34 ms | 14.79 ms | 0.91 | n/a | n/a | n/a | n/a |
| **Symbolic atoms (40 000 atoms)** | | | | | | | |
| iterate, read symbol and literal | 12.50 ms | 1.34 ms | 0.11 | 766.8 us | 114.79 ms | 1.75 | 0.01 |
| `of::<P>()` | 11.39 ms | 2.11 ms | 0.19 | n/a | n/a | n/a | n/a |
| **Facts (10 000 per run)** | | | | | | | |
| `add_facts` (convert, print, parse, ground) | 57.97 ms | 50.90 ms | 0.88 | 48.20 ms | 101.67 ms | 1.06 | 0.50 |
| the same facts as program text | 43.36 ms | 43.21 ms | 1.00 | 41.53 ms | 41.30 ms | 1.04 | 1.05 |
| conversion alone | 2.58 ms | 1.37 ms | 0.53 | 1.60 ms | 38.31 ms | 0.85 | 0.04 |
| **Ground callbacks (20 000 calls)** | | | | | | | |
| ground, no callback | 26.93 ms | 27.09 ms | 1.01 | 31.98 ms | 26.98 ms | 0.85 | 1.00 |
| ground with `@f(X)` | 33.29 ms | 30.68 ms | 0.92 | 38.28 ms | 113.75 ms | 0.80 | 0.27 |
| **Propagator** | | | | | | | |
| 65 536 models, no propagator | 18.55 ms | 18.12 ms | 0.98 | 17.24 ms | 77.89 ms | 1.05 | 0.23 |
| 65 536 models, watch all, count | 27.86 ms | 27.06 ms | 0.97 | 26.08 ms | 148.63 ms | 1.04 | 0.18 |
| pigeons 9/8, no propagator | 221.84 ms | 216.24 ms | 0.97 | 219.75 ms | 209.23 ms | 0.98 | 1.03 |
| pigeons 9/8, watch all, count | 222.88 ms | 220.53 ms | 0.99 | 223.88 ms | 226.87 ms | 0.99 | 0.97 |
| **Observer (30 000 rules)** | | | | | | | |
| ground, no observer | 82.25 ms | 81.47 ms | 0.99 | 98.26 ms | 77.37 ms | 0.83 | 1.05 |
| ground, counting observer | 90.19 ms | 86.66 ms | 0.96 | 98.12 ms | 239.89 ms | 0.88 | 0.36 |
| **Syntax trees (3 000 rules)** | | | | | | | |
| parse, keep every statement | 74.53 ms | 74.15 ms | 0.99 | 59.41 ms | 77.53 ms | 1.25 | 0.96 |
| visit every node (identity) | 24.17 ms | 9.03 ms | 0.37 | n/a | 1.21 s | n/a | 0.01 |
| rewrite every variable | 80.87 ms | 49.15 ms | 0.61 | n/a | 2.71 s | n/a | 0.02 |
| print every statement | 13.33 ms | 13.42 ms | 1.01 | n/a | 19.72 ms | n/a | 0.68 |
| build 3 000 fact rules | 5.68 ms | 5.64 ms | 0.99 | n/a | 51.16 ms | n/a | 0.11 |
| add the parsed statements | 21.53 ms | 21.32 ms | 0.99 | n/a | 15.11 ms | n/a | 1.41 |
| **Configuration and statistics** | | | | | | | |
| walk the configuration (84 nodes) | 118.0 us | 74.0 us | 0.63 | 22.5 us | 970.9 us | 3.28 | 0.08 |
| copy the statistics (164 values) | 114.4 us | 58.2 us | 0.51 | 29.6 us | n/a | 1.96 | n/a |
| **Application** | | | | | | | |
| `Control` add, ground, solve | 164.7 us | 157.6 us | 0.96 | 103.4 us | 163.9 us | 1.52 | 0.96 |
| `Application::run` doing the same | 186.0 us | 186.6 us | 1.00 | n/a | 189.1 us | n/a | 0.99 |
| **End to end** | | | | | | | |
| 8 queens, all 92 models | 2.58 ms | 2.51 ms | 0.98 | 2.46 ms | 2.66 ms | 1.02 | 0.94 |
| pigeons 8/7, unsatisfiable | 21.99 ms | 21.72 ms | 0.99 | 21.94 ms | 21.13 ms | 0.99 | 1.03 |

## The layer against the bare C API

To see what the wrapper adds, the same clingo calls were written directly against
the raw bindings (`clingox-sys`, no checks at all) and timed on the same clingo
5.8.2. This floor is the least any Rust program can pay for the workload.

| Workload | Bare C API | clingox | Extra per operation |
|---|--:|--:|--:|
| create `p(i,1)` | 47 ns | 63 ns | 16 ns |
| read a tree with `arguments`, `name`, `as_number` | 92 ns | 202 ns | 110 ns |
| iterate atoms, read symbol and literal | 18 ns | 34 ns | 16 ns |
| enumerate, per model | 358 ns | 555 ns | 197 ns |
| `contains` for one atom, per call | 52 ns | 105 ns | 53 ns |
| read the shown symbols, per model (110 atoms) | 2 660 ns | 2 942 ns | 282 ns |

The 110 ns is mostly type checks. Each of `name`, `arguments`, `sign` and
`as_number` asks clingo for the symbol's type before it reads, which is what makes
the accessors return `None` for the wrong type instead of failing, so a loop that
reads several parts of a symbol pays for that check each time. `Symbol::kind`
checks the type once and hands back the parts: the benchmark `symbol/kind` reads a
single symbol that way in 36 ns, where it took 185 ns before the optimisations.

## Reading the results

- **Solving and grounding are clingo's.** The two end-to-end solves agree between
  clingox, the crate and pyclingo to within 6%. The propagator on the pigeon-hole
  problem, which calls back about 12 000 times in a 220 ms search, does not move
  the total.
- **Symbol creation, printing and hashing match the `clingo` crate**, within 8% in
  both directions. Reading the parts of a symbol tree is 13% slower than the crate
  (202 ns against 180 ns), which is the repeated type check above. pyclingo is 20
  to 160 times slower on creation, inspection and hashing, 3 times slower on
  printing, and level on parsing, where clingo does the work.
- **Callbacks.** A ground callback costs clingox 180 ns per call on top of a
  grounding, against 315 ns for the crate. A propagator watching every atom costs
  136 ns per `propagate` call in clingox and 135 ns in the crate. These include the
  user's closure, which here reads a number and pushes one. An observer that
  counts costs about 30 ns per callback (about 180 000 rule and output atom
  calls in the workload), which includes the checks clingox makes on every atom
  and literal clingo passes and the crate does not.
- **Model iteration.** Each model costs clingox about 200 ns more than the raw
  calls, of which the two error-state resets are about half (estimated from the
  profile and from `symbol/kind`, see the proposals).
  Against the crate that is a factor 1.6 on a workload that does nothing else, and
  it disappears as soon as the program reads anything from the model: reading
  the shown symbols is 10% slower than the crate, and reading and printing every
  symbol is 4% faster.
- **Symbolic atoms.** Iteration is 34 ns per atom against 19 ns for the crate and
  18 ns bare. It was 312 ns before the optimisations.
- **Configuration and statistics** are the one place where the design costs. Every
  read resolves its path from the root, so a whole-tree walk is 3.3 times slower
  than the crate's, and copying the statistics 2 times. A single statistics read
  costs 0.45 us, and the configuration walk 0.9 us per node (two reads each); see
  the proposals for a cursor that would remove the difference.
- **The `clingo` version shows in fixed costs.** Creating a control and running a
  trivial program takes 158 us in clingox and 164 us in pyclingo, both on 5.8.2,
  and 103 us in the crate on 5.6.2. `Application::run` adds 29 us on top of that in
  clingox and 25 us in pyclingo, which is `clingo_main`'s own setup.

## What was optimised

The changes are the `Raw:` and `Api:` commits on the branch `final/bench`. No
check was removed: every literal check, range check, UTF-8
check and NUL check is still made, and every error is still read from a fresh
error state.

Profiling with `perf` on the starting code showed where the time went. On the
symbol readers, 40% of the cycles were in `clingo_set_error`, the C++ exception
machinery it uses to allocate an error object, and `strlen`: `call` resets
clingo's error state before every call so that a `false` return cannot report a
stale error, and the reset allocates. On callbacks, two mutex acquisitions per
call went to checking whether an earlier callback had failed.

| Change | What it does | Effect |
|---|---|---|
| `raw::query` for pure queries | A read that succeeds skips the reset. A read that fails runs again through `call`, so the error still comes from a fresh state. Used by the symbol, atom, model, theory, statistics, configuration, assignment and syntax tree readers. | `symbol/kind` 185 to 36 ns, tree inspection 681 to 202 us, atom iteration 12.5 to 1.34 ms, `atoms::<P>()` 52 to 15 ms, `derive(FromSymbol)` 390 to 108 us, syntax tree visit 24.2 to 9.0 ms, configuration walk 118 to 74 us, statistics reads 1.0 to 0.45 us each |
| Atomic flag in `Slot` | The slots that hold the first error or panic of a callback keep a flag next to the lock, so the check before each callback is a load. | Ground callback overhead 318 to 179 ns per call (together with the buffer below); propagator overhead 142 to 136 ns |
| Value buffer in ground callbacks | The `Vec` that collects the values of a call is kept between calls. | Same row as above |
| Flag in the message capture | `Capture::take` is called twice around every call and locked each time; it now checks a flag. | Not separable from noise |
| Version check remembered | `check_version` runs before every call; once it has passed it is one relaxed load. | 4.4% of the tree inspection profile before, 1.4% after |
| `with_c_str` | Names and paths under 64 bytes go to clingo through a stack copy instead of a `CString`, with the same NUL check and the same error. | `create_function` 132 to 63 ns, `create_string` 103 to 45 ns |
| `fill_string` fills the result in place | The buffer clingo fills becomes the returned `String`; it was allocated and copied three times. The UTF-8 check and the NUL check stay. | Printing a symbol 985 to 790 ns |
| One type check for name, arguments and sign | `Symbol::kind`, the typed model readers and the derive helpers check the type once. | `models/typed_atoms` and `derive_from_symbol`, on top of the row above |
| `#[inline]` on the small `Symbol` accessors | So callers in other crates can inline them. | Within noise |

The effect of each change alone was not measured, because most of them share the
same call sites; the table gives the profile share or the workload that moved.
The first row accounts for most of the gains. The unit tests for the new helpers
(`query`, `with_c_str`, `Slot`, the ground callback buffer) sit next to them, and
each was checked with a negative control: with the mutation in place (a `Slot`
that never sets its flag, a `query` that does not run a failure again, a capture
that never sets its flag) the tests fail.

## Proposals

These were measured or profiled but not done, because each changes a design
decision or reaches into every entry point.

1. **Reset the error state lazily for the calls that change something.**
   `raw::call` still resets before every call that adds, grounds or solves, and a
   solving step makes two (`resume` and `model`). The reset costs about 50 ns
   per call (`symbol/kind` makes three calls and saved 149 ns), so about 100 ns of
   the 200 ns per model. A thread-local flag that says
   "clingo's error state may be non-success" (set when a call fails and when
   clingox itself sets an error) would let the reset be skipped when it is clear.
   It needs every path that can set the state audited (clingo's own C++ code,
   the trampolines, the script and application entry points) and DESIGN S1
   restated, which is why it was not done here.
2. **A cursor for configuration and statistics.** clingo's API works with numeric
   keys; clingox works with paths and resolves each from the root. An
   `entries()` cursor over the tree would make a walk as cheap as the crate's
   (23 us instead of 74 us for the configuration) and leave the path methods as
   they are.
3. **`add_facts` through the backend.** `add_facts` prints every symbol, parses
   the text and grounds it, which is 7.7 ms of overhead over adding the same text
   for 10 000 facts. Adding atoms through the backend would skip the print and
   the parse, but the semantics differ (redefinition errors, `#show`, the part
   name) and the change needs its own design.
4. **Print symbols in Rust for `add_facts`.** clingo prints in two passes (the
   size, then the text). Writing the text from the parts of the symbol would halve
   that, but the printed form must equal clingo's for strings, negative numbers,
   tuples and `#inf`, so it needs an exhaustive test against clingo.
5. **Build settings.** clingox is a library, so LTO and `codegen-units` are the
   application's choice. They were not measured here.

## Reproducing

```text
cargo bench -p clingox --bench layer -- --save-baseline mine
```

The numbers depend on the machine; compare ratios, not absolute times. To repeat
the comparison, write the workloads of `layer.rs` against the `clingo` crate
(features `static-linking` and `derive`; on a system with a recent CMake, set
`CMAKE_POLICY_VERSION_MINIMUM=3.5` for its clingo 5.6.2 build) and against
pyclingo, keeping the group and function names, and run the three on one core
under a machine lock. `perf record` on the bench binary with
`--profile-time 5 <filter>` gives the profiles quoted above.
