# Benchmarks

This page records what clingox's own layer costs, how it was measured, how it
compares with the `clingo` crate, with pyclingo and with clingo's own C++ API,
and what was changed to make it cheaper. It is the answer to "is the wrapper slow?" for the workloads where a
wrapper can matter. It says nothing about clingo's grounding and solving speed,
which no binding changes.

The suite is `clingox/benches/layer.rs`, a criterion benchmark. The programs that
run the same workloads against the other libraries are kept outside the
published repository (see [Comparison](#comparison)). All figures were measured
on 2026-10-03, except the Windows table, which was measured earlier.

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
| CPU | AMD Ryzen 9 3900X (12 cores, 24 threads), frequency governor `powersave`, boost enabled |
| Memory | 31 GiB |
| OS | Fedora Linux, kernel 7.2.8 |
| Rust | rustc 1.99.0, `--release` (criterion's `bench` profile), edition 2024 |
| clingox | 508.2.0-beta.2 (`250a174`), vendored clingo 5.8.2 with clingox's patches, features `derive` and `threads` |
| clingox before | `16c6897`, the code of `e9cb2f6` (the commit the optimisations started from) plus the benchmark file, built again for this measurement |
| `clingo` crate | 0.8.0 with `clingo-sys` 0.7.2, which builds clingo **5.6.2** from source (`static-linking`), compiled by the same clang with the same flags as clingox's clingo |
| pyclingo | 5.8.2, Python 3.14.7 |
| clingo's C++ API | `clingo.hh` of the same vendored clingo 5.8.2, linked to the static libraries clingox-sys builds |
| C++ compiler | clang++ 22.1.8, `-O3 -DNDEBUG -ffunction-sections -fdata-sections -fPIC -m64 -std=c++17`, the compiler and flags of clingox-sys's CMake build of clingo (build type `Release`) |
| criterion | 0.8.2 |

Every program ran pinned to one core (core 11, whose SMT sibling was idle) with
`taskset`. Each session held a machine-wide lock, so no other benchmark or test
suite ran at the same time, and no other heavy run was active at the start or end
of either session. Other work on the machine was not stopped: the load average
was between 1.5 and 2.3 (desktop programs on other cores). Numbers are therefore
good to a few percent, not to the last digit; where two results are within about
5% of each other, read them as equal.

The measurement is two sessions of about 14 minutes each, on 2026-10-03. Both
ran every program (clingox, clingox before, the crate, the bare C API, C++ and
pyclingo), the first in that order and the second in the reverse order, so that a
drift of the machine does not favour one library. **Every table below comes from
these same two sessions**, so a ratio between two columns, in any table, compares
programs measured under the same conditions. Each figure is the median of the two
session medians (the mean of the two). A figure marked `(v)` differs between the
two sessions by more than 10%; only three figures do (see
[Variance](#variance)).

Each criterion group uses 20 samples, a 0.5 s warm-up and a 3 s measurement, and
the tables give the **median** of a run of the workload. Setup that is not part
of the workload (creating and grounding a control, for instance) runs outside
the timed region. pyclingo runs the same workloads with `time.perf_counter_ns`
and reports the median of 20 runs (10 when one run takes more than 0.1 s) after 3
warm-up runs, with the garbage collector run before each. The C++ program uses a
small harness that follows criterion's method: a 0.5 s warm-up with doubling
iteration counts, then 20 samples of 1, 2, ..., 20 times a fixed iteration count
sized for 3 s, and the median over the samples of the time per run. Workloads
that consume a control time each run alone, with the setup outside the timed
region and the control's destruction inside it, as in the Rust closures. Times
are for one run of the whole workload, whose size is in the group heading.

Before timing, every program prints a check value for every workload (the
models and atoms counted, the symbols read, the propagate, rule and callback calls,
the syntax tree nodes, the configuration nodes, the statistics values), and each
is compared with clingox's. Of the 123 comparisons that apply, 120 are identical and 3 differ,
all three from the crate's clingo 5.6.2: its configuration tree has 83 nodes
(285 value bytes) where clingo 5.8.2 has 84 (287), and it reports 162 statistics
values where 5.8.2 reports 164. These rows are timed anyway and carry a note
below the Results table.

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

The same workloads run against three other libraries. The programs that do this
exist, but they are kept outside the published repository: the `clingo` crate
builds its own clingo from source and would add a second native build to every
checkout, and the C++ program needs a C++ toolchain setup of its own. This page
describes each workload so that the comparison can be repeated (see
[Reproducing](#reproducing)).

- **The `clingo` crate 0.8.0** (`clingo-sys` 0.7.2) links clingo **5.6.2**, not
  5.8.2. Everything a solver does can differ between the two versions, so a
  ratio near 1.0 says the two wrappers cost the same, and a ratio far from 1.0 on
  a row where clingo does most of the work (a solve, a grounding) says something
  about clingo's versions and not about either wrapper. The row
  "`Control` add, ground, solve" is not such a case: clingo's C++ API on 5.8.2
  takes about as long as the crate on 5.6.2 (103.8 us and 103.1 us), and clingox
  takes 116.8 us (see [Against clingo's C++ API](#against-clingos-c-api)).
- **pyclingo 5.8.2** runs the identical clingo as clingox, so its rows compare the
  languages directly. It is the oracle for behaviour in the test suite and is
  included here as the tool most users of clingo start from.
- **clingo's C++ API (`clingo.hh`)** is the header-only wrapper that ships with
  clingo. It calls the same C API as clingox and turns a `false` return into a
  C++ exception, with no other checks, and it does not reset clingo's error state
  before a call. It is compiled against the same 5.8.2 static libraries that
  clingox-sys builds, with the same compiler and optimisation, so the language
  layer is the only difference.

Where a library has no equivalent of a workload, the cell says n/a. In the crate,
the syntax tree rewrites and the typed model reader have no counterpart the
benchmark could use, its `Application` trait is commented out in 0.8.0, and its
`ToSymbol` derive has no reverse. The workloads that do not map one to one:

- **Syntax trees, parse.** The crate's row counts the statements it is handed;
  clingox and pyclingo keep every statement, which is what a rewrite needs.
- **Configuration walk.** clingox walks by path (`solve.models`), which clasp
  resolves from the root on every call, and, in a row of its own, by entry,
  which holds clingo's key for each node. The crate walks with numeric keys and
  `clingo.hh` with keys too, so their one walk stands for both rows. The clingo
  version also differs (84 nodes in 5.8.2, 83 in 5.6.2). pyclingo's walk visits
  84 nodes with the kinds of clingox's walk (array or array-map by index, map by
  name).
- **Statistics.** The statistics rows are measured for every library. clingox's
  "copy the statistics" is `Statistics::snapshot`, which builds an owned tree,
  and the crate's row builds an owned tree as well; clingo reports 162 values in
  the crate's 5.6.2 and 164 in 5.8.2. The "walk by entry" row reads every value
  through entries without building a tree (the crate's and `clingo.hh`'s walks
  with keys). pyclingo's `Control.statistics` copies the tree into a `dict` and
  caches it until the next solve; its copy row clears the cache before each read,
  so it times the copy, and "100 reads of one statistics path" goes through the
  property each time (C calls to test the cache, then `dict` lookups). pyclingo
  has no entry handle, so its two entry rows are n/a. The "100 reads of one
  statistics entry" row reads one resolved entry 100 times, which is a key
  lookup in the crate and `clingo.hh`.
- **Facts.** `add_facts` has no counterpart, so the other libraries convert their
  values to symbols, print them and add the text, which is what `add_facts` does.
- **Propagator and observer.** Each library gets a propagator that watches every
  atom and counts its calls, and an observer that counts its rule calls, both
  written in the library's own idiom.
- **Multi-shot.** One control holds the program `{a}.`, grounded once; a run is
  one more solve. In the row "one more solve" nothing can interrupt the solve, so
  clingox runs it inside the call without a thread (see
  [proposal 6](#proposals)); the crate, pyclingo and `clingo.hh` solve
  blocking. In the row "`InterruptHandle` alive" clingox holds an
  `InterruptHandle`, which makes its solve asynchronous, and the other three
  libraries solve asynchronously as well, since that is what clingox does with a
  live handle.

The C++ program is written the way a C++ user would write it: `Model::symbols`,
range-for over a solve handle and over `SymbolicAtoms`, `Symbol::arguments`,
`Propagator` and `GroundProgramObserver` subclasses, `Control::ground` with a
callback, `Application` with `clingo_main`, and the `AST` namespace
(`parse_string`, `transform_ast`, `ProgramBuilder`). Before timing, it prints the
work it does, and each count equals clingox's on the same workload. Where
it differs:

- **Solving to the end** (the propagator, the application and the pigeon-hole
  rows) uses a blocking solve, `solve(..., asynchronous = false, yield = false)`,
  as pyclingo's `solve()` does. clingox, built with threads, also solves inside
  the call when nothing can interrupt the solve. `Application::run` still solves
  asynchronously and waits, which costs a thread start. Model
  enumeration uses the yielding range-for in both.
- **Symbolic atoms** reads the symbol and the literal only. clingox's iterator
  also reads whether the atom is a fact and whether it is external, and the bare
  C API row makes the same four calls as clingox.
- **Statistics** are copied into an owned tree, as clingox's snapshot does, and
  the entry rows walk with keys.
- **Hashing** uses `std::hash<Symbol>`, which returns clingo's hash. clingox's
  row feeds that hash into Rust's default hasher and finishes it for every pair.
- **Facts and conversion** build `edge(from,to,"label")` by hand, since C++ has no
  derive, and the add-facts row prints the symbols and adds the text, like the
  other libraries.
- **n/a**: the derive row, the typed model and atom readers and `solve_all` have
  no counterpart in `clingo.hh`.
- **`Application` in `clingo.hh`** also ran an asynchronous solve of the same
  program (`plain_control_async`): 153.4 us against 103.8 us blocking.

## Results

Times are for one run of the workload, at the size in the group name. "Now" is
the code in this repository; "before" is the commit the optimisations started
from (`e9cb2f6`, built as `16c6897`), measured with the same benchmark file in the
same sessions, so the ratio is not affected by drift between sessions. A ratio
below 1 means clingox now is faster than the other column. The multi-shot group
is new on this page. The entry walks and the statistics reads are now measured
for every library, and "build 3 000 fact rules" for the crate. The "before" build
has no entry walks and no multi-shot group, so those cells are n/a.

| Workload | clingox before | clingox now | now/before | clingo crate | pyclingo | now/crate | now/py |
|---|--:|--:|--:|--:|--:|--:|--:|
| **Symbols (1000 per run)** | | | | | | | |
| create a number | 3.1 us | 3.4 us | 1.08 | 3.2 us | 534.6 us | 1.07 | 0.01 |
| create `p(i,1)` | 128.9 us | 63.8 us | 0.50 | 85.1 us | 2.66 ms | 0.75 | 0.02 |
| create a string | 103.5 us | 45.8 us | 0.44 | 45.6 us | 897.6 us | 1.00 | 0.05 |
| create `p(i,f(i,"s"),(1,))` | 486.9 us | 239.3 us | 0.49 | 235.3 us | 7.21 ms | 1.02 | 0.03 |
| parse a term | 1.99 ms | 1.99 ms | 1.00 | 1.89 ms | 2.53 ms | 1.05 | 0.79 |
| print a nested symbol | 968.1 us | 787.1 us | 0.81 | 794.3 us | 2.45 ms | 0.99 | 0.32 |
| read name, arguments and numbers of a tree | 708.3 us | 196.5 us | 0.28 | 169.0 us | 16.57 ms | 1.16 | 0.01 |
| hash, `<`, `==` of neighbours | 19.7 us | 19.5 us | 0.99 | 20.2 us | 819.8 us | 0.97 | 0.02 |
| `#[derive(ToSymbol)]` on `Edge` | 255.4 us | 142.0 us | 0.56 | 157.5 us | n/a | 0.90 | n/a |
| **Models (1024 models of 110 atoms)** | | | | | | | |
| enumerate, count | 602.4 us | 564.2 us | 0.94 | 372.1 us | 2.36 ms | 1.52 | 0.24 |
| read the shown symbols | 3.15 ms | 3.04 ms | 0.97 | 2.82 ms | 6.49 ms | 1.08 | 0.47 |
| read and print every symbol | 66.65 ms | 45.74 ms | 0.69 | 46.18 ms | 245.95 ms | 0.99 | 0.19 |
| `contains` for one atom | 745.2 us | 657.5 us | 0.88 | 439.8 us | 3.28 ms | 1.50 | 0.20 |
| `atoms::<P>()` (sort and convert) | 53.06 ms | 15.21 ms | 0.29 | n/a | n/a | n/a | n/a |
| `solve_all`, owned models | 16.22 ms | 15.02 ms | 0.93 | n/a | n/a | n/a | n/a |
| **Symbolic atoms (40 000 atoms)** | | | | | | | |
| iterate, read symbol and literal | 12.65 ms | 1.31 ms | 0.10 | 723.4 us | 109.69 ms | 1.81 | 0.01 |
| `of::<P>()` | 11.57 ms | 2.08 ms | 0.18 | n/a | n/a | n/a | n/a |
| **Facts (10 000 per run)** | | | | | | | |
| `add_facts` (convert, print, parse, ground) | 57.21 ms | 50.98 ms | 0.89 | 48.09 ms | 101.25 ms | 1.06 | 0.50 |
| the same facts as program text | 43.24 ms | 43.38 ms | 1.00 | 41.25 ms | 39.87 ms | 1.05 | 1.09 |
| conversion alone | 2.56 ms | 1.38 ms | 0.54 | 1.54 ms | 36.44 ms | 0.90 | 0.04 |
| **Ground callbacks (20 000 calls)** | | | | | | | |
| ground, no callback | 27.40 ms | 26.63 ms | 0.97 | 31.08 ms | 26.56 ms | 0.86 | 1.00 |
| ground with `@f(X)` | 33.52 ms | 29.93 ms | 0.89 | 37.38 ms | 107.68 ms | 0.80 | 0.28 |
| **Propagator** | | | | | | | |
| 65 536 models, no propagator | 18.38 ms | 17.94 ms | 0.98 | 17.44 ms | 74.50 ms | 1.03 | 0.24 |
| 65 536 models, watch all, count | 27.61 ms | 27.15 ms | 0.98 | 26.00 ms | 142.18 ms | 1.04 | 0.19 |
| pigeons 9/8, no propagator | 219.17 ms | 223.60 ms | 1.02 | 220.91 ms | 198.87 ms | 1.01 | 1.12 |
| pigeons 9/8, watch all, count | 224.00 ms | 227.87 ms | 1.02 | 225.35 ms | 218.39 ms | 1.01 | 1.04 |
| **Observer (30 000 rules)** | | | | | | | |
| ground, no observer | 83.74 ms | 84.61 ms | 1.01 | 96.99 ms | 78.03 ms | 0.87 | 1.08 |
| ground, counting observer | 91.63 ms | 89.33 ms | 0.97 | 97.95 ms | 230.23 ms | 0.91 | 0.39 |
| **Syntax trees (3 000 rules)** | | | | | | | |
| parse, keep every statement | 77.78 ms | 76.04 ms | 0.98 | 59.04 ms | 72.26 ms | 1.29 | 1.05 |
| visit every node (identity) | 25.50 ms | 9.75 ms | 0.38 | n/a | 1.15 s | n/a | 0.01 |
| rewrite every variable | 83.49 ms | 50.90 ms | 0.61 | n/a | 2.66 s | n/a | 0.02 |
| print every statement | 13.80 ms | 13.76 ms | 1.00 | n/a | 19.66 ms | n/a | 0.70 |
| build 3 000 fact rules | 5.86 ms | 5.84 ms | 1.00 | 4.53 ms | 50.87 ms | 1.29 | 0.11 |
| add the parsed statements | 21.78 ms | 22.66 ms | 1.04 | n/a | 21.33 ms | n/a | 1.06 |
| **Configuration and statistics** | | | | | | | |
| walk the configuration (84 nodes) | 120.9 us | 77.6 us | 0.64 | 23.5 us | 729.0 us | 3.30 | 0.11 |
| walk the configuration by entry (84 nodes) | n/a | 26.8 us | n/a | 23.5 us | n/a | 1.14 | n/a |
| copy the statistics (164 values) | 113.0 us | 60.3 us | 0.53 | 39.2 us | 691.4 us | 1.54 | 0.09 |
| walk the statistics by entry (164 values) | n/a | 59.9 us | n/a | 30.2 us | n/a | 1.98 | n/a |
| 100 reads of one statistics path | 104.1 us | 47.4 us | 0.46 | 19.3 us | 351.8 us | 2.45 | 0.13 |
| 100 reads of one statistics entry | n/a | 3.2 us | n/a | 2.0 us | n/a | 1.62 | n/a |
| **Application** | | | | | | | |
| `Control` add, ground, solve | 159.9 us | 116.8 us | 0.73 | 103.1 us | 184.5 us (v) | 1.13 | 0.63 |
| `Application::run` doing the same | 186.2 us | 195.9 us | 1.05 | n/a | 208.4 us | n/a | 0.94 |
| **Multi-shot (one control, `{a}.`)** | | | | | | | |
| one more solve | n/a | 5.5 us | n/a | 4.4 us | 26.8 us (v) | 1.24 | 0.20 |
| one more solve, `InterruptHandle` alive | n/a | 36.6 us | n/a | 32.6 us | 98.1 us | 1.12 | 0.37 |
| **End to end** | | | | | | | |
| 8 queens, all 92 models | 2.56 ms | 2.66 ms | 1.04 | 2.46 ms | 2.77 ms | 1.08 | 0.96 |
| pigeons 8/7, unsatisfiable | 21.98 ms | 23.24 ms | 1.06 | 22.00 ms | 21.09 ms | 1.06 | 1.10 |

Notes on the rows:

- **Configuration rows, crate.** The crate's clingo 5.6.2 has 83 configuration
  nodes where 5.8.2 has 84. Its one key walk stands for both walk rows.
- **Statistics rows, crate.** 5.6.2 reports 162 values where 5.8.2 reports 164.
  The crate's "copy the statistics" row builds an owned tree, as clingox's
  snapshot does; the earlier version of this table had 29.6 us for a cell that
  only walked the tree, which is today's "walk the statistics by entry" cell
  (30.2 us).
- **pyclingo, statistics.** The copy row times the copy of a `dict` that
  `Control.statistics` otherwise caches; the path-read row goes through the
  property each time (see [Comparison](#comparison)).
- **pyclingo, configuration walk.** Today's walk visits 84 nodes. The earlier
  version of this table had 970.9 us for a walk of 131 nodes.
- **Multi-shot, `InterruptHandle` alive.** An asynchronous solve in the crate,
  pyclingo and `clingo.hh`, since that is what clingox does with a live handle.
- **Before column.** "100 reads of one statistics path" (104.1 us) has a before
  figure for the first time.
- **`(v)`.** Marks a figure whose two sessions differ by more than 10%.

### Variance

Three figures differ between the two sessions by more than 10%:

| Library | Workload | Session 1 | Session 2 | Spread |
|---|---|--:|--:|--:|
| `clingo.hh` | 100 reads of one statistics path | 14.2 us | 11.2 us | 27% |
| pyclingo | one more solve (multi-shot) | 23.8 us | 29.9 us | 26% |
| pyclingo | `Control` add, ground, solve | 175.6 us | 193.3 us | 10.1% |

No clingox, before, crate or bare figure is flagged. The largest spread among
clingox's own figures is 7.8% (reading a symbol tree), then 7.7% (100 reads of one
statistics path).

## Against clingo's C++ API

Same two sessions as the table above, so the clingox column is the "now" column
of the Results table. A ratio above 1 means clingox is slower than `clingo.hh`.
The notes below the Results table on the multi-shot row and on `(v)` apply here.

| Workload | clingox | clingo.hh | clingox/C++ |
|---|--:|--:|--:|
| **Symbols (1000 per run)** | | | |
| create a number | 3.4 us | 2.2 us | 1.56 |
| create `p(i,1)` | 63.8 us | 60.5 us | 1.05 |
| create a string | 45.8 us | 30.3 us | 1.51 |
| create `p(i,f(i,"s"),(1,))` | 239.3 us | 158.7 us | 1.51 |
| parse a term | 1.99 ms | 1.91 ms | 1.04 |
| print a nested symbol | 787.1 us | 772.3 us | 1.02 |
| read name, arguments and numbers of a tree | 196.5 us | 94.0 us | 2.09 |
| hash, `<`, `==` of neighbours | 19.5 us | 12.0 us | 1.62 |
| `#[derive(ToSymbol)]` on `Edge` | 142.0 us | n/a | n/a |
| **Models (1024 models of 110 atoms)** | | | |
| enumerate, count | 564.2 us | 381.8 us | 1.48 |
| read the shown symbols | 3.04 ms | 3.02 ms | 1.01 |
| read and print every symbol | 45.74 ms | 42.15 ms | 1.09 |
| `contains` for one atom | 657.5 us | 446.1 us | 1.47 |
| `atoms::<P>()` (sort and convert) | 15.21 ms | n/a | n/a |
| `solve_all`, owned models | 15.02 ms | n/a | n/a |
| **Symbolic atoms (40 000 atoms)** | | | |
| iterate, read symbol and literal | 1.31 ms | 526.8 us | 2.48 |
| `of::<P>()` | 2.08 ms | n/a | n/a |
| **Facts (10 000 per run)** | | | |
| `add_facts` (convert, print, parse, ground) | 50.98 ms | 49.61 ms | 1.03 |
| the same facts as program text | 43.38 ms | 43.79 ms | 0.99 |
| conversion alone | 1.38 ms | 1.11 ms | 1.25 |
| **Ground callbacks (20 000 calls)** | | | |
| ground, no callback | 26.63 ms | 28.24 ms | 0.94 |
| ground with `@f(X)` | 29.93 ms | 30.40 ms | 0.98 |
| **Propagator** | | | |
| 65 536 models, no propagator | 17.94 ms | 17.19 ms | 1.04 |
| 65 536 models, watch all, count | 27.15 ms | 26.01 ms | 1.04 |
| pigeons 9/8, no propagator | 223.60 ms | 219.46 ms | 1.02 |
| pigeons 9/8, watch all, count | 227.87 ms | 222.66 ms | 1.02 |
| **Observer (30 000 rules)** | | | |
| ground, no observer | 84.61 ms | 82.56 ms | 1.02 |
| ground, counting observer | 89.33 ms | 85.82 ms | 1.04 |
| **Syntax trees (3 000 rules)** | | | |
| parse, keep every statement | 76.04 ms | 76.42 ms | 0.99 |
| visit every node (identity) | 9.75 ms | 19.91 ms | 0.49 |
| rewrite every variable | 50.90 ms | 52.86 ms | 0.96 |
| print every statement | 13.76 ms | 12.63 ms | 1.09 |
| build 3 000 fact rules | 5.84 ms | 3.13 ms | 1.86 |
| add the parsed statements | 22.66 ms | 18.54 ms | 1.22 |
| **Configuration and statistics** | | | |
| walk the configuration (84 nodes) | 77.6 us | 18.1 us | 4.30 |
| walk the configuration by entry (84 nodes) | 26.8 us | 18.1 us | 1.48 |
| copy the statistics (164 values) | 60.3 us | 28.5 us | 2.11 |
| walk the statistics by entry (164 values) | 59.9 us | 21.8 us | 2.75 |
| 100 reads of one statistics path | 47.4 us | 12.7 us (v) | 3.73 |
| 100 reads of one statistics entry | 3.2 us | 1.0 us | 3.06 |
| **Application** | | | |
| `Control` add, ground, solve | 116.8 us | 103.8 us | 1.13 |
| `Application::run` doing the same | 195.9 us | 122.5 us | 1.60 |
| **Multi-shot (one control, `{a}.`)** | | | |
| one more solve | 5.5 us | 4.6 us | 1.20 |
| one more solve, `InterruptHandle` alive | 36.6 us | 34.0 us | 1.08 |
| **End to end** | | | |
| 8 queens, all 92 models | 2.66 ms | 2.57 ms | 1.04 |
| pigeons 8/7, unsatisfiable | 23.24 ms | 22.52 ms | 1.03 |

In the entry rows, the C++ column walks or reads with keys.

## The layer against the bare C API

To see what the wrapper adds, the same clingo calls were written directly against
the raw bindings (`clingox-sys`, no checks at all, in Rust) and timed on the same
clingo 5.8.2. This floor is the least any Rust program can pay for the workload.
The bare program's iterator for the atoms makes the four calls clingox's does
(symbol, literal, fact, external), and its model loop uses yield mode like
`for_each_model`. The numbers below come from the same two sessions as the tables
above, so all three columns were measured together. Per item: `p(i,1)` and the
tree per symbol (1000 per run), atoms per atom (40 000), models per model (1024),
and `contains` as (`contains` minus count) per model.

| Workload | Bare C API | `clingo.hh` | clingox | clingox over bare |
|---|--:|--:|--:|--:|
| create `p(i,1)` | 58 ns | 61 ns | 64 ns | 6 ns |
| read a tree with `arguments`, `name`, `as_number` | 76 ns | 94 ns | 197 ns | 121 ns |
| iterate atoms, read symbol and literal | 19 ns | 13 ns | 33 ns | 14 ns |
| enumerate, per model | 363 ns | 373 ns | 551 ns | 188 ns |
| `contains` for one atom, per call | 50 ns | 63 ns | 91 ns | 41 ns |
| read the shown symbols, per model (110 atoms) | 2 694 ns | 2 946 ns | 2 971 ns | 276 ns |

`clingo.hh` stays close to the bare calls: it is inlined into the caller, checks
nothing but the return value, and never resets clingo's error state. Its atom
row is below the floor because it reads two properties of each atom where the
floor and clingox read four. It is above the floor on three rows: reading a tree
(94 ns against 76 ns), `contains` (63 ns against 50 ns, a difference of two
workloads, so about 15 ns either way is noise) and the shown symbols (2 946 ns
against 2 694 ns, 9% slower although it makes the same two calls). These were not
traced further.

The bare calls for `p(i,1)` took 58 ns in these sessions where an earlier
measurement had 47 ns; the crate (68.7 to 85.1 us per 1000) and `clingo.hh` (48.0
to 60.5 us) moved the same way, and clingox did not (63.3 to 63.8 us). So
"clingox over bare" for that row is 6 ns now and was 25 ns. The cause of the
slower bare, crate and C++ calls was not traced; the three programs that moved
call `clingo_symbol_create_function` with nothing around it, so the change is in
that call's cost and not in a wrapper.

The 121 ns is mostly type checks. Each of `name`, `arguments`, `sign` and
`as_number` asks clingo for the symbol's type before it reads, which is what makes
the accessors return `None` for the wrong type instead of failing, so a loop that
reads several parts of a symbol pays for that check each time. `Symbol::kind`
checks the type once and hands back the parts: the benchmark `symbol/kind` reads a
single symbol that way in 35 ns, where it took 188 ns before the optimisations.

## Reading the results

- **Solving and grounding are clingo's.** The two end-to-end solves agree between
  clingox, the crate and pyclingo to within 10% (8 queens 2.66 ms, 2.46 ms and
  2.77 ms; pigeons 8/7 23.24 ms, 22.00 ms and 21.09 ms), and with `clingo.hh` to
  within 4%. The propagator on the pigeon-hole problem, which calls back about
  12 000 times in a 220 ms search, does not move the total (223.6 ms without and
  227.9 ms with the propagator). On that 9/8 problem pyclingo takes 198.9 ms
  against clingox's 223.6 ms. clingox's two end-to-end figures are 4 to 6% above
  the "before" build (2.66 ms against 2.56 ms, 23.24 ms against 21.98 ms), and
  3 to 4% above `clingo.hh` (2.57 ms and 22.52 ms). The two sessions agree within
  1% on these clingox figures. The solves now run inside the call instead of on
  clasp's thread; whether that is the cause was not traced.
- **Symbol creation, printing and hashing match the `clingo` crate**, within 8% in
  both directions, except `p(i,1)` (clingox 25% faster, because the crate's figure
  moved, see the bare C API section) and the derive row (10% faster). Reading the
  parts of a symbol tree is 16% slower than the crate (197 ns against 169 ns),
  which is the repeated type check described in the bare C API section. pyclingo is 20 to 160 times
  slower on creation, inspection and hashing, 3 times slower on printing, and
  1.3 times slower on parsing (2.53 ms against 1.99 ms), where clingo does the
  work.
- **Callbacks.** A ground callback costs clingox 165 ns per call on top of a
  grounding, against 315 ns for the crate and 108 ns for `clingo.hh`. A propagator
  watching every atom costs 140 ns per `propagate` call in clingox, 131 ns in the
  crate and 135 ns in `clingo.hh`. These include the user's closure, which here
  reads a number and pushes one. An observer that counts costs about 26 ns per
  callback in clingox (about 180 000 rule and output atom calls in the
  workload), 5 ns in the crate and 18 ns in `clingo.hh`; clingox's figure includes
  the checks it makes on every atom and literal clingo passes and the crate does
  not.
- **Model iteration.** Each model costs clingox about 190 ns more than the raw
  calls (551 ns against 363 ns), of which the two error-state resets are about
  half (estimated from the profile and from `symbol/kind`, see the proposals).
  Against the crate that is a factor 1.5 on a workload that does nothing else, and
  it disappears as soon as the program reads anything from the model: reading
  the shown symbols is 8% slower than the crate, and reading and printing every
  symbol is 1% faster.
- **Symbolic atoms.** Iteration is 33 ns per atom against 18 ns for the crate and
  19 ns bare. It was 316 ns before the optimisations.
- **Configuration and statistics** are where the design costs when a program
  reads by path. Every path read resolves its path from the root, so a whole-tree
  walk by path is 3.3 times slower than the crate's (77.6 us against 23.5 us), and
  copying the statistics 1.5 times (60.3 us against 39.2 us). A single statistics
  read by path costs 0.47 us, and the configuration walk 0.92 us per node (two
  reads each). Through entries (proposal 2) the configuration walk takes 26.8 us,
  1.14 times the crate's walk and 1.48 times `clingo.hh`'s (319 ns per node
  against 280 ns and 215 ns), and one statistics read takes 32 ns against 20 ns
  and 10 ns (100 reads: 3.2 us against 2.0 us and 1.0 us). Walking the statistics
  through entries takes 59.9 us, no faster than the snapshot (60.3 us), and 2.0
  times the crate's walk (30.2 us) and 2.75 times `clingo.hh`'s (21.8 us).
- **Fixed costs of a control.** Creating a control and running a trivial program
  takes 116.8 us in clingox, 103.1 us in the crate on 5.6.2, 103.8 us in
  `clingo.hh` on 5.8.2 and 184.5 us (`(v)`) in pyclingo on 5.8.2, so
  clingox is 1.13 times `clingo.hh`. It was 157.6 us before proposal 6 was done
  (1.58 times `clingo.hh`): clingox, built with threads, started each solve
  asynchronously and waited for it, which starts a thread, and the same C++
  program with an asynchronous solve takes 153.4 us instead of 103.8 us. Since
  then a blocking solve that nothing can interrupt (no `InterruptHandle`, no
  timeout) runs without the thread. On one control with the program `{a}.`, one
  more blocking solve takes 5.5 us in clingox (4.4 us in the crate, 4.6 us in
  `clingo.hh`, 26.8 us in pyclingo), and 36.6 us with an `InterruptHandle` alive,
  which makes the solve asynchronous (32.6 us, 34.0 us and 98.1 us for the other
  three, which solve asynchronously in that row). `Application::run` still solves
  asynchronously, so at 195.9 us it takes 79 us more than a plain control in
  clingox, where `clingo_main` adds 18.7 us in C++ and 23.9 us in pyclingo.
- **Against clingo's C++ API**, the table in
  [Against clingo's C++ API](#against-clingos-c-api) separates clingox's own cost
  from the language, since both run the identical clingo:
  - Wherever clingo does the work, the two agree to within 6%: grounding with and
    without callbacks or an observer, the propagator runs, adding facts, parsing
    and rewriting syntax trees, and the end-to-end solves. Parsing and printing a
    symbol are within 4%, printing every statement of a syntax tree and reading
    and printing every symbol of a model within 9%. A propagator costs the same per
    `propagate` call (140 ns in clingox, 135 ns in C++).
  - clingox is slower on the calls that do little work each, where its checks
    and error handling are a large share. Creating `p(i,1)` takes 64 ns against
    61 ns; creating a number 3.4 ns against 2.2 ns, a string 46 ns against 30 ns
    (the copy and NUL check of the name); reading a symbol tree 197 ns against
    94 ns (the type check before each accessor, see above); a model 551 ns against
    373 ns and `contains` 91 ns against 63 ns (the error-state resets of
    proposal 1); an atom 33 ns against 13 ns, of which 5 ns are the two extra
    reads C++ does not make; a ground callback 165 ns against 108 ns on top of the
    grounding; an observer callback about 26 ns against 18 ns (the checks on every
    atom and literal). The hashing row is 1.62 times slower because the Rust
    workload runs Rust's default hasher over clingo's hash for every pair.
  - Configuration and statistics by path are 2.1 to 4.3 times slower than C++,
    which walks with keys as the crate does: 924 ns per configuration node
    against 215 ns, and 474 ns per statistics read against 127 ns. Through
    entries the factors are 1.5 (configuration walk), 2.75 (statistics walk) and
    3.1 (100 reads of one entry, 3.2 us against 1.0 us).
  - Syntax trees: a visit that rebuilds every node is 2.0 times faster in
    clingox (9.75 ms against 19.91 ms), because `clingo.hh`'s `transform_ast` reads
    every attribute into a variant and copies the node handles. Building 3 000
    fact rules is 1.86 times slower (2.7 ms more, about 0.9 us per rule, not
    profiled), and adding the parsed statements through a program builder 1.22
    times slower. In an earlier profile of the program builder, clingox's own
    functions were about 1% of the profile; the rest was clingo's conversion, the
    allocator and the kernel, with more time in the kernel than in the C++
    process, and the difference was not traced further.

## Changes since the previous tables

The rows that moved by more than 10% against the previous version of this page,
with the reason where one is known.

clingox:

- `Control` add, ground, solve: 157.6 us to 116.8 us (-26%). This is the
  blocking solve without a thread (proposal 6). The ratio to `clingo.hh` went
  from 1.58 to 1.13 and to the crate from 1.52 to 1.13.
- `Application::run` is unchanged (195.9 us, against 186.6 us and 191.8 us before),
  because it still solves asynchronously.
- Creating 1000 numbers: 4.4 us to 3.4 us in the C++ table (-23%); the Results
  table had 3.4 us, so the earlier C++ session's 4.4 us was an outlier. Creating
  `p(i,1)` went from 71.8 us to 63.8 us there (-11%), against 63.3 us in the
  earlier Results table.
- The entry walks were on this page before, from a separate measurement, and did
  not change: the configuration by entry 26.9 us to 26.8 us, the statistics by
  entry about 60 us to 59.9 us, and 100 entry reads from "3.1 to 4.7 us" to 3.2 us.

The other libraries, where the cause was not traced unless stated:

- Crate, create `p(i,1)`: 68.7 us to 85.1 us (+24%). `clingo.hh` moved the same
  way (48.0 us to 60.5 us, +26%) and so did the bare calls (47 ns to 58 ns), while
  clingox did not (see
  [The layer against the bare C API](#the-layer-against-the-bare-c-api)).
- Crate, copy the statistics: 29.6 us to 39.2 us (+32%). The workload changed:
  the crate now builds an owned tree like clingox.
- pyclingo, configuration walk: 970.9 us to 729.0 us (-25%). The workload
  changed: 84 nodes instead of 131.
- pyclingo, enumerate and count 1.44 ms to 2.36 ms (+64%); `contains` 2.42 ms to
  3.28 ms (+36%); read the shown symbols 5.68 ms to 6.49 ms (+14%); add the parsed
  statements 15.11 ms to 21.33 ms (+41%); `Application::run` 189.1 us to
  208.4 us (+10%, borderline). The cause was not traced. Today's models loop is
  `with ctl.solve(yield_=True) as h: for m in h`, and the program builder is
  `with ast.ProgramBuilder(ctl) as b` with one `b.add` per statement, with the
  control from the setup released inside the timed region.
- `clingo.hh`, add the parsed statements: 14.68 ms to 18.54 ms (+26%); rewrite
  every variable: 46.67 ms to 52.86 ms (+13%). Not traced. As a result the ratio
  of clingox to C++ for the program builder went from 1.46 to 1.22, and for the
  rewrite from 1.05 to 0.96. `clingo.hh` also got faster on creating
  `p(i,f(i,"s"),(1,))` (176.7 us to 158.7 us, -10%) and on creating a number
  (2.5 us to 2.2 us, -12%), which are small absolute changes.
- clingox before, parse a term: 2.73 ms to 1.99 ms (-27%), now equal to clingox
  now, so the ratio of 0.74 became 1.00. Not traced; the "before" build is
  `16c6897` built again with today's compiler.

The end-to-end solves are 4 to 6% above the "before" build (2.66 ms against
2.56 ms for 8 queens, 23.24 ms against 21.98 ms for pigeons 8/7) and 3 to 4%
above `clingo.hh`, steady in both sessions, and the pigeons 9/8 rows are 2% above
"before". These are below the 10% line used above, and their cause was not
traced.

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
| `raw::query` for pure queries | A read that succeeds skips the reset. A read that fails runs again through `call`, so the error still comes from a fresh state. Used by the symbol, atom, model, theory, statistics, configuration, assignment and syntax tree readers. | `symbol/kind` 188 to 35 ns, tree inspection 708 to 197 us, atom iteration 12.65 to 1.31 ms, `atoms::<P>()` 53 to 15 ms, `derive(FromSymbol)` 401 to 103 us, syntax tree visit 25.5 to 9.75 ms, configuration walk 121 to 78 us, statistics reads 1.04 to 0.47 us each |
| Atomic flag in `Slot` | The slots that hold the first error or panic of a callback keep a flag next to the lock, so the check before each callback is a load. | Ground callback overhead 306 to 165 ns per call (together with the buffer below); propagator overhead 141 to 140 ns |
| Value buffer in ground callbacks | The `Vec` that collects the values of a call is kept between calls. | Same row as above |
| Flag in the message capture | `Capture::take` is called twice around every call and locked each time; it now checks a flag. | Not separable from noise |
| Version check remembered | `check_version` runs before every call; once it has passed it is one relaxed load. | 4.4% of the tree inspection profile before, 1.4% after |
| `with_c_str` | Names and paths under 64 bytes go to clingo through a stack copy instead of a `CString`, with the same NUL check and the same error. | `create_function` 128.9 to 63.8 ns, `create_string` 103.5 to 45.8 ns |
| `fill_string` fills the result in place | The buffer clingo fills becomes the returned `String`; it was allocated and copied three times. The UTF-8 check and the NUL check stay. | Printing a symbol 968 to 787 ns |
| One type check for name, arguments and sign | `Symbol::kind`, the typed model readers and the derive helpers check the type once. | `models/typed_atoms` and `derive_from_symbol`, on top of the row above |
| `#[inline]` on the small `Symbol` accessors | So callers in other crates can inline them. | Within noise |

The figures in the table are the "before" and "now" columns of the 2026-10-03
measurement. The effect of each change alone was not measured, because most of them share the
same call sites; the table gives the profile share or the workload that moved.
The first row accounts for most of the gains. The unit tests for the new helpers
(`query`, `with_c_str`, `Slot`, the ground callback buffer) sit next to them, and
each was checked with a negative control: with the mutation in place (a `Slot`
that never sets its flag, a `query` that does not run a failure again, a capture
that never sets its flag) the tests fail.

## Proposals

These were measured or profiled. Proposals 2 and 6 are done; the others are not,
because each changes a design decision or reaches into every entry point.

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
2. **A cursor for configuration and statistics (done).** clingo's API works
   with numeric keys; clingox's path methods resolve each path from the root.
   `Configuration::root`/`entry` and `Statistics::root`/`entry` now hand out
   entries that hold the key, with a `children()` iterator, and the path methods
   are unchanged. Measured on 2026-10-03 (two sessions, pinned to one core): the
   configuration walk takes 26.8 us by entry against 77.6 us by path in the same
   sessions (ratio 0.35; the crate's walk is 23.5 us and `clingo.hh`'s 18.1 us),
   and 100 reads of one resolved statistics entry take 3.2 us against 47.4 us by
   path (the crate 2.0 us, `clingo.hh` 1.0 us). The statistics walk by entry is
   59.9 us, no faster than the snapshot (60.3 us), because every map child keeps
   its `map_has_subkey` check, so that no clingo logic error can reach a caller;
   the crate's walk takes 30.2 us and `clingo.hh`'s 21.8 us. A faster snapshot is
   future work.
3. **`add_facts` through the backend.** `add_facts` prints every symbol, parses
   the text and grounds it, which is 7.6 ms of overhead over adding the same text
   for 10 000 facts (50.98 ms against 43.38 ms). Adding atoms through the backend would skip the print and
   the parse, but the semantics differ (redefinition errors, `#show`, the part
   name) and the change needs its own design.
4. **Print symbols in Rust for `add_facts`.** clingo prints in two passes (the
   size, then the text). Writing the text from the parts of the symbol would halve
   that, but the printed form must equal clingo's for strings, negative numbers,
   tuples and `#inf`, so it needs an exhaustive test against clingo.
5. **Build settings.** clingox is a library, so LTO and `codegen-units` are the
   application's choice. They were not measured here.
6. **The thread start of every solve. Done.**
   With threads, `Control::solve` used to start the search in async mode and
   wait for it, and clasp starts a new thread for each async search
   (`clasp_facade.cpp:378`). The search runs no slower (65 536 models: 17.2 ms
   blocking and async), but each call cost about 50 us more: a trivial add,
   ground and solve takes 153.4 us async and 103.8 us blocking in C++ (measured on
   2026-10-03, pinned to one core). For one large solve that is nothing; for a
   multi-shot program with thousands of tiny solves it can dominate. Measured on
   2026-10-03, a trivial add, ground and solve on a control takes 116.8 us in
   clingox (157.6 us before the change), and one more solve on a grounded
   control takes 5.5 us blocking and 36.6 us with an `InterruptHandle` alive
   (the crate 4.4 us and 32.6 us, `clingo.hh` 4.6 us and 34.0 us). Two ways out
   were considered:
   - **Solve blocking (mode 0) while no interrupt can reach the control. Done.**
     `Control::solve`, `solve_with` without a timeout and `solve_with_events`
     now run the search inside `clingo_control_solve`, in the phase `Inside`,
     when `Arc::get_mut` on the control's shared interrupt state succeeds: no
     `InterruptHandle`, no timeout thread, no application printer slot and no
     open search holds it. In that phase an interrupt from the control's own
     thread (the logger, a propagator, a model printer) returns `false` instead
     of stopping the search, because it cannot tell whether clasp's strategy
     has attached and an early one would be queued in `qSig` and end the next
     solve call at its start (`raw::interrupt`), but no interrupt can be sent:
     a new holder of the state needs `&self` and the solve holds `&mut self`.
     The checks listed here were made: handing out a handle during a blocking
     solve is impossible (no callback receives the control); a later
     `interrupt_handle()` switches the next solve back to async; clasp's
     warnings are the same in both modes; `blocking_solve_mode` alternates
     mode-0 solves with interrupted ones on one control, also under TSan (the
     older racing-solve tests always hold a handle, so they run async). The `Application` path keeps async (195.9 us, 79 us above a plain control), because the printer
     slot holds the state, and gains nothing; publishing the slot only when a
     printer is installed is a separate change, not made yet.
   - **Reuse the thread. Not done.** clasp creates a fresh `mt::thread` per
     async search and offers no option to keep one, through the C API or
     otherwise, so this would need a patch to clasp (a persistent worker per
     facade) and would have to keep `doStart`'s wait for the strategy to
     attach, on which the `Running` phase relies. It would only help the
     searches that still run async.

## Windows: clingo with and without `/GL`

This table was measured earlier (2026-09-30) and was not repeated for the
measurement of 2026-10-03: it comes from a `windows-2025` CI job with MSVC, which
the machine of the other tables cannot run.

clasp and libpotassco build with `/GL` on MSVC in Release, which makes `link.exe`
restart with `/LTCG` and print a warning for every binary. The question was
whether removing `/GL` costs speed. One `windows-2025` job (x86_64, MSVC 14.51)
built clingox twice, with and without `/GL` (the two `VC_RELEASE_OPTIONS` lines
edited in the vendored source), and ran the same workloads alternately, five rounds
each (2026-09-30). Medians in milliseconds, criterion's middle estimate:

| Workload | With `/GL` | Without | Ratio |
|---|---|---|---|
| `end_to_end/queens8_all` | 12.29 | 12.14 | 0.988 |
| `end_to_end/pigeons8_7_unsat` | 121.3 | 121.4 | 1.001 |
| `propagator/pigeons_baseline` (9 pigeons, 8 holes) | 1265 | 1281 | 1.013 |
| `observer/baseline_no_observer` (30 000 rules) | 389 | 403 | 1.035 |
| `facts/add_facts_10k` | 263 | 265 | 1.008 |
| 11 queens, all 2680 models (about 1.7 s) | 1737 | 1791 | 1.031 |
| pigeons 9 in 8, unsatisfiable (about 1.5 s) | 1536 | 1588 | 1.034 |

The rounds of one variant differ by 1 to 2% (about 10% on the 30 000-rule row); the
two larger programs vary by about 10 ms in 1.6 s. Relinking a small test binary took
about 0.9 s either way.

Decision: keep `/GL`. Without it the two larger solves are 3.1 and 3.4% slower, beyond
the noise, and the small rows stay within 1.5%. The warning is harmless and can be
silenced by the application (known issues, Platforms). A `/LTCG` link argument emitted
by `clingox-sys` would not help: a build script's link arguments apply to the
package's own targets, not to the crates that depend on it.


## Reproducing

```text
cargo bench -p clingox --bench layer -- --save-baseline mine
```

This runs clingox's own suite and needs only this repository. The numbers depend
on the machine; compare ratios, not absolute times. For stable figures, pin the
run to one core with `taskset` and run it twice, once after another workload
and once before, as the two sessions above did, taking the median of the two.

The programs for the other columns are not shipped with the repository. To repeat
the comparison, write the workloads of `layer.rs` against the `clingo` crate
(features `static-linking` and `derive`; on a system with a recent CMake, set
`CMAKE_POLICY_VERSION_MINIMUM=3.5` for its clingo 5.6.2 build) and against
pyclingo, keeping the group and function names, and run the three on one core
under a machine lock. For the C++ column, write them against `clingo.hh` and
build the program with the compiler and flags recorded in the `CMakeCache.txt` of
clingox-sys's build directory, including the `include` directory and linking the
static libraries (`clingo`, `gringo`, `reify`, `clasp`, `potassco`, then
`pthread`) that the build installed under `target/release/build/clingox-sys-*/out`.
The bare column is the same workloads written in Rust against `clingox-sys`. Print
a check value for every workload before timing it, and compare the values with
clingox's, so that a time is read only for a workload that does the same work.
`perf record` on the bench binary with `--profile-time 5 <filter>` gives the
profiles quoted above.
