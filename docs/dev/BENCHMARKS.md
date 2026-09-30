# Benchmarks

This page records what clingox's own layer costs, how it was measured, how it
compares with the `clingo` crate, with pyclingo and with clingo's own C++ API,
and what was changed to make it cheaper. It is the answer to "is the wrapper slow?" for the workloads where a
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
| clingo's C++ API | `clingo.hh` of the same vendored clingo 5.8.2, linked to the static libraries clingox-sys builds |
| C++ compiler | clang++ 22.1.8, `-O3 -DNDEBUG -ffunction-sections -fdata-sections -fPIC -m64 -std=c++17`, the compiler and flags of clingox-sys's CMake build of clingo (build type `Release`) |
| criterion | 0.8 |

Rust and C++ programs ran pinned to one core with `taskset`, with no other
benchmark or test suite running at the same time. Other work on the machine was not stopped, and the
load average during the runs was between 3 and 6 (a mix of builds and idle
processes on other cores), and between 0.8 and 2.5 during the later session that
measured clingo's C++ API. Numbers are therefore good to a few percent, not to
the last digit; where two results are within about 5% of each other, read them
as equal.

Each criterion group uses 20 samples, a 0.5 s warm-up and a 3 s measurement, and
the tables give the **median** of a run of the workload. Setup that is not part
of the workload (creating and grounding a control, for instance) runs outside
the timed region. pyclingo runs the same workloads with `time.perf_counter_ns`
and reports the median of 10 to 20 runs after 3 warm-up runs, with the garbage
collector run before each. The C++ program uses a small harness that follows
criterion's method: a 0.5 s warm-up with doubling iteration counts, then 20
samples of 1, 2, ..., 20 times a fixed iteration count sized for 3 s, and the
median over the samples of the time per run. Workloads that consume a control
time each run alone, with the setup outside the timed region and the control's
destruction inside it, as in the Rust closures. Times are for one run of the
whole workload, whose size is in the group heading.

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

The same workloads run against three other libraries. Their sources are not in
the repository, because the `clingo` crate builds its own clingo from source and
would add a second native build to every checkout, and the C++ program needs a
C++ toolchain setup of its own; the workloads are small and this page describes
each of them.

- **The `clingo` crate 0.8.0** (`clingo-sys` 0.7.2) links clingo **5.6.2**, not
  5.8.2. Everything a solver does can differ between the two versions, so a
  ratio near 1.0 says the two wrappers cost the same, and a ratio far from 1.0 on
  a row where clingo does most of the work (a solve, a grounding) says something
  about clingo's versions and not about either wrapper. The row
  "`Control` add, ground, solve" is not such a case: clingo's C++ API on 5.8.2
  takes as long as the crate on 5.6.2, and the 55 to 60 us that clingox takes
  more come from its asynchronous solve (see
  [Against clingo's C++ API](#against-clingos-c-api)).
- **pyclingo 5.8.2** runs the identical clingo as clingox, so its rows compare the
  languages directly. It is the oracle for behaviour in the test suite and is
  included here as the tool most users of clingo start from.
- **clingo's C++ API (`clingo.hh`)** is the header-only wrapper that ships with
  clingo. It calls the same C API as clingox and turns a `false` return into a
  C++ exception, with no other checks, and it does not reset clingo's error state
  before a call. It is compiled against the same 5.8.2 static libraries that
  clingox-sys builds, with the same compiler and optimisation, so the language
  layer is the only difference. It is measured in a later session than the
  crate and pyclingo, together with a new run of clingox and of the bare C API.

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

The C++ program is written the way a C++ user would write it: `Model::symbols`,
range-for over a solve handle and over `SymbolicAtoms`, `Symbol::arguments`,
`Propagator` and `GroundProgramObserver` subclasses, `Control::ground` with a
callback, `Application` with `clingo_main`, and the `AST` namespace
(`parse_string`, `transform_ast`, `ProgramBuilder`). Before timing, it prints the
work it does (models, shown symbols, atoms and their literals, callback, rule and
propagate calls, syntax tree nodes and renamed variables, configuration nodes,
statistics values), and each count equals clingox's on the same workload. Where
it differs:

- **Solving to the end** (the propagator, the application and the pigeon-hole
  rows) uses a blocking solve, `solve(..., asynchronous = false, yield = false)`,
  as pyclingo's `solve()` does. clingox, built with threads, solves
  asynchronously and waits, which costs a thread start per solve. Model
  enumeration uses the yielding range-for in both.
- **Symbolic atoms** reads the symbol and the literal only. clingox's iterator
  also reads whether the atom is a fact and whether it is external, and the bare
  C API row makes the same four calls as clingox.
- **Statistics** are copied into an owned tree, as clingox's snapshot does. The
  crate's row only walks them.
- **Hashing** uses `std::hash<Symbol>`, which returns clingo's hash. clingox's
  row feeds that hash into Rust's default hasher and finishes it for every pair.
- **Facts and conversion** build `edge(from,to,"label")` by hand, since C++ has no
  derive, and the add-facts row prints the symbols and adds the text, like the
  other libraries.
- **n/a**: the derive row, the typed model and atom readers and `solve_all` have
  no counterpart in `clingo.hh`.

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

## Against clingo's C++ API

This table comes from a later session than the one above. It ran clingox (the
same code as "now"), the bare C API and the C++ program twice, in opposite
orders, and each column gives the lower of its two medians, since other work on
the machine can only add time. Across the 44 workloads, clingox's own numbers in
this session were a median 1.6% slower than in the session above, and 38 were
within 5%. Five were 6 to 14% slower (creating and printing symbols, the derive
and the statistics path reads), and creating 1000 numbers 31% (3.4 to 4.4 us). Compare
the ratios within one table, not absolute times across the two.

A ratio above 1 means clingox is slower than `clingo.hh`.

| Workload | clingox | clingo.hh | clingox/C++ |
|---|--:|--:|--:|
| **Symbols (1000 per run)** | | | |
| create a number | 4.4 us | 2.5 us | 1.80 |
| create `p(i,1)` | 71.8 us | 48.0 us | 1.50 |
| create a string | 45.8 us | 30.4 us | 1.51 |
| create `p(i,f(i,"s"),(1,))` | 254.9 us | 176.7 us | 1.44 |
| parse a term | 2.03 ms | 1.85 ms | 1.09 |
| print a nested symbol | 839.6 us | 764.5 us | 1.10 |
| read name, arguments and numbers of a tree | 210.4 us | 88.6 us | 2.37 |
| hash, `<`, `==` of neighbours | 18.4 us | 11.9 us | 1.55 |
| `#[derive(ToSymbol)]` on `Edge` | 146.7 us | n/a | n/a |
| **Models (1024 models of 110 atoms)** | | | |
| enumerate, count | 587.6 us | 382.0 us | 1.54 |
| read the shown symbols | 3.10 ms | 3.06 ms | 1.01 |
| read and print every symbol | 49.25 ms | 42.01 ms | 1.17 |
| `contains` for one atom | 692.7 us | 449.1 us | 1.54 |
| `atoms::<P>()` (sort and convert) | 15.21 ms | n/a | n/a |
| `solve_all`, owned models | 15.00 ms | n/a | n/a |
| **Symbolic atoms (40 000 atoms)** | | | |
| iterate, read symbol and literal | 1.37 ms | 501.8 us | 2.72 |
| `of::<P>()` | 2.08 ms | n/a | n/a |
| **Facts (10 000 per run)** | | | |
| `add_facts` (convert, print, parse, ground) | 51.15 ms | 48.93 ms | 1.05 |
| the same facts as program text | 43.51 ms | 43.44 ms | 1.00 |
| conversion alone | 1.40 ms | 1.10 ms | 1.27 |
| **Ground callbacks (20 000 calls)** | | | |
| ground, no callback | 27.14 ms | 28.15 ms | 0.96 |
| ground with `@f(X)` | 30.79 ms | 30.35 ms | 1.01 |
| **Propagator** | | | |
| 65 536 models, no propagator | 18.28 ms | 17.19 ms | 1.06 |
| 65 536 models, watch all, count | 27.39 ms | 26.43 ms | 1.04 |
| pigeons 9/8, no propagator | 217.93 ms | 219.71 ms | 0.99 |
| pigeons 9/8, watch all, count | 222.09 ms | 222.41 ms | 1.00 |
| **Observer (30 000 rules)** | | | |
| ground, no observer | 81.95 ms | 84.92 ms | 0.97 |
| ground, counting observer | 88.21 ms | 87.84 ms | 1.00 |
| **Syntax trees (3 000 rules)** | | | |
| parse, keep every statement | 75.58 ms | 72.97 ms | 1.04 |
| visit every node (identity) | 9.10 ms | 21.26 ms | 0.43 |
| rewrite every variable | 49.01 ms | 46.67 ms | 1.05 |
| print every statement | 13.38 ms | 12.74 ms | 1.05 |
| build 3 000 fact rules | 5.58 ms | 3.12 ms | 1.79 |
| add the parsed statements | 21.38 ms | 14.68 ms | 1.46 |
| **Configuration and statistics** | | | |
| walk the configuration (84 nodes) | 73.4 us | 17.1 us | 4.30 |
| copy the statistics (164 values) | 59.7 us | 28.2 us | 2.12 |
| 100 reads of one statistics path | 47.3 us | 11.8 us | 4.01 |
| **Application** | | | |
| `Control` add, ground, solve | 163.2 us | 103.3 us | 1.58 |
| `Application::run` doing the same | 191.8 us | 122.9 us | 1.56 |
| **End to end** | | | |
| 8 queens, all 92 models | 2.57 ms | 2.54 ms | 1.01 |
| pigeons 8/7, unsatisfiable | 21.94 ms | 22.43 ms | 0.98 |

"100 reads of one statistics path" is `stats_path_reads` in the suite; the table
above leaves it out because the other libraries were not measured on it.

## The layer against the bare C API

To see what the wrapper adds, the same clingo calls were written directly against
the raw bindings (`clingox-sys`, no checks at all) and timed on the same clingo
5.8.2. This floor is the least any Rust program can pay for the workload. The
numbers below come from the session of the C++ comparison, so all three columns
were measured together.

| Workload | Bare C API | `clingo.hh` | clingox | clingox over bare |
|---|--:|--:|--:|--:|
| create `p(i,1)` | 47 ns | 48 ns | 72 ns | 25 ns |
| read a tree with `arguments`, `name`, `as_number` | 95 ns | 89 ns | 210 ns | 115 ns |
| iterate atoms, read symbol and literal | 18 ns | 13 ns | 34 ns | 16 ns |
| enumerate, per model | 363 ns | 373 ns | 574 ns | 211 ns |
| `contains` for one atom, per call | 50 ns | 65 ns | 103 ns | 53 ns |
| read the shown symbols, per model (110 atoms) | 2 691 ns | 2 991 ns | 3 029 ns | 338 ns |

`clingo.hh` costs what the bare calls cost: it is inlined into the caller, checks
nothing but the return value, and never resets clingo's error state. Its atom
row is below the floor because it reads two properties of each atom where the
floor and clingox read four. The `contains` row is a difference of two
workloads, so 15 ns either way is noise. On the shown symbols, `clingo.hh` was
10% slower than the bare calls in both passes although it makes the same two
calls; this was not traced further.

The 115 ns is mostly type checks. Each of `name`, `arguments`, `sign` and
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
- **Fixed costs come from the asynchronous solve, not the `clingo` version.**
  Creating a control and running a trivial program takes 158 us in clingox and
  164 us in pyclingo, both on 5.8.2, and 103 us in the crate on 5.6.2. `clingo.hh`
  on 5.8.2 also takes 103 us, so the version is not the cause. clingox, built
  with threads, starts each solve asynchronously and waits for it, which starts
  a thread; the same C++ program with an asynchronous solve takes 159 us instead
  of 108 us (median of 2 000 runs, pinned to one core). pyclingo solves without a
  thread, so its extra time is its own. `Application::run` adds 29 us on top of
  that in clingox and 25 us in pyclingo, which is `clingo_main`'s own setup;
  `clingo_main` from C++ adds 20 us.
- **Against clingo's C++ API**, the table in
  [Against clingo's C++ API](#against-clingos-c-api) separates clingox's own cost
  from the language, since both run the identical clingo:
  - Wherever clingo does the work, the two agree to within 6%: grounding with and
    without callbacks or an observer, the propagator runs, adding facts, parsing
    and printing syntax trees, and the end-to-end solves. Parsing and printing a
    symbol are within 10%. A propagator costs the same per
    `propagate` call (139 ns in clingox, 141 ns in C++).
  - clingox is slower on the calls that do little work each, where its checks
    and error handling are a large share. Creating `p(i,1)` takes 72 ns against
    48 ns (the copy and NUL check of the name); reading a
    symbol tree 210 ns against 89 ns (the type check before each accessor, see
    above); a model 574 ns against 373 ns and `contains` 103 ns against 65 ns
    (the error-state resets of proposal 1); an atom 34 ns against 13 ns, of which
    5 ns are the two extra reads C++ does not make; a ground callback 182 ns
    against 110 ns on top of the grounding; an observer callback about 35 ns
    against 16 ns (the checks on every atom and literal). Creating 1000 numbers
    takes 4.4 us against 2.5 us, 2 ns per number. The hashing row is 1.55 times
    slower because the Rust workload runs Rust's default hasher over clingo's
    hash for every pair.
  - Configuration and statistics by path are 2 to 4.3 times slower than C++,
    which walks with keys as the crate does: 874 ns per configuration node
    against 203 ns, and 473 ns per statistics read against 118 ns.
  - Syntax trees: a visit that rebuilds every node is 2.3 times faster in
    clingox (9.1 ms against 21.3 ms), because `clingo.hh`'s `transform_ast` reads
    every attribute into a variant and copies the node handles. Building 3 000
    fact rules is 1.8 times slower (about 160 ns more per constructor call, not
    profiled), and adding the parsed statements through a program builder 1.46
    times slower. For the program builder, clingox's own functions are about 1%
    of the profile; the rest is clingo's conversion, the allocator and the kernel,
    with more time in the kernel than in the C++ process, and the difference was
    not traced further.

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
6. **The thread start of every solve.** With threads, `Control::solve` starts
   the search in async mode and waits for it, and clasp starts a new thread for
   each async search (`clasp_facade.cpp:378`). The search runs no slower (65 536
   models: 17.2 ms blocking and async), but each call costs about 50 us more: a
   trivial add, ground and solve took 159 us async and 108 us blocking in C++,
   pinned to one core or not. For one large solve that is nothing; for a
   multi-shot program with thousands of tiny solves it can dominate. Two ways
   out, neither tried:
   - **Solve blocking (mode 0) while no interrupt can reach the control.** If no
     `InterruptHandle` has been handed out for this control and the call has no
     timeout, nothing on another thread can interrupt it, so the async start
     buys nothing. The search would then run inside `clingo_control_solve` in
     the phase `Inside`, as on a build without threads, where an interrupt from
     the control's own thread (the logger, a propagator, a model printer)
     returns `false` instead of stopping the search, because it
     cannot tell whether clasp's strategy has attached and an early one would
     be queued in `qSig` and end the next solve call at its start
     (`raw::interrupt`). What needs checking: that handing out a handle
     during a blocking solve (from a callback) is refused or waits; that the
     `Application` path, where clingo owns the control and the printer
     interrupts through its `SolveSync`, keeps working; that a later
     `interrupt_handle()` switches the control back to async for its next
     solve; whether clasp's warnings differ between the two modes; and the TSan and racing-solve tests of S13 in both modes.
   - **Reuse the thread.** clasp creates a fresh `mt::thread` per async search
     and offers no option to keep one, through the C API or otherwise, so this
     would need a patch to clasp (a persistent worker per facade) and would
     have to keep `doStart`'s wait for the strategy to attach, on which the
     `Running` phase relies.

## Windows: clingo with and without `/GL`

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

The numbers depend on the machine; compare ratios, not absolute times. To repeat
the comparison, write the workloads of `layer.rs` against the `clingo` crate
(features `static-linking` and `derive`; on a system with a recent CMake, set
`CMAKE_POLICY_VERSION_MINIMUM=3.5` for its clingo 5.6.2 build) and against
pyclingo, keeping the group and function names, and run the three on one core
under a machine lock. For the C++ column, write them against `clingo.hh` and
build the program with the compiler and flags recorded in the `CMakeCache.txt` of
clingox-sys's build directory, including the `include` directory and linking the
static libraries (`clingo`, `gringo`, `reify`, `clasp`, `potassco`, then
`pthread`) that the build installed under `target/release/build/clingox-sys-*/out`. `perf record` on the bench binary with
`--profile-time 5 <filter>` gives the profiles quoted above.
