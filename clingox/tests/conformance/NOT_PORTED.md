# Upstream items not ported

Every item listed here duplicates a ported item or is not applicable to a
library. Everything else upstream ships is ported.

## The counting unit

Every source below uses one fixed, mechanical unit, recounted directly from
the submodule by `cargo xtask conformance-count` (wired into `cargo xtask
check`, so a stale table here fails the build instead of drifting quietly):

- `app/clingo/tests/{lp,python,lua}/`: one `.lp` file is one item.
- `examples/c/`: one `.c` file is one item (`CMakeLists.txt` is not a
  program and does not count).
- `libclingo/tests/*.cc` (`clingo.cc`, `symbol.cc`, `astv2.cc`,
  `propagator.cc`, `variant.cc`): one `TEST_CASE(...)` or one `SECTION(...)`
  is one item, at every nesting depth. A `SECTION` that only sets up state
  for its own nested `SECTION`s (`with control`, for example, which wraps
  every leaf under the "solving" `TEST_CASE`) is still counted, and is
  "ported" exactly when at least one of its descendants is ported: its own
  body runs on every one of them, so it is exercised whenever any child is,
  and is never listed as its own separate not-ported row.
- `libpyclingo/clingo/tests/test_*.py`: one `def test_...` method is one
  item.

Counting a nested `SECTION` under its parent's name, or counting
`astv2.cc`/`propagator.cc`/`variant.cc` by `TEST_CASE` alone, gives a smaller
total than the fixed unit above (105 items for `libclingo/tests/`, 72 real
`def test_` methods for pyclingo). `cargo xtask conformance-count --old-unit`
prints the totals of the earlier, hand-written inventory for comparison.

## Per-source counts

| Source | Total | Ported | Not ported |
|---|---|---|---|
| `app/clingo/tests/lp/` | 8 | 8 | 0 |
| `app/clingo/tests/python/` | 51 | 50 | 1 |
| `app/clingo/tests/lua/` | 55 | 19 | 36 |
| `examples/c/` | 13 | 13 | 0 |
| `libclingo/tests/` | 105 | 104 | 1 |
| `libpyclingo/clingo/tests/test_*.py` | 72 | 70 | 2 |

**Total: 304 upstream items, 264 ported, 40 not ported.**

The propagator items are all accounted for: 12 Python fixtures, 13 Lua
fixtures (all duplicates of a Python original, none needing its own port),
`propagator.c`, and the 26 `libclingo/tests/propagator.cc` items (1 ported
directly, the other 25 covered by the propagator test suites and by
`propagator.lp`'s port, cited by name below rather than re-ported) plus
`clingo.cc`'s `model-add-clause`. The two pyclingo rows that are not
ported (`test_model`, `test_propagator_init`) are explained in the pyclingo
table below.

## lp/ fixtures (8 total, 8 ported)

All 8 LP fixtures are ported. The incmode loop from `incmode.cc` is
implemented in `conformance/mod.rs::run_incmode`, so `elevator.lp` and
`show.lp` pass.

### Ported (8)

aggregates, elevator, external, istop, numbers, project, show, subset

## Python fixtures (51 total, 50 ported, 1 not ported)

All 51 Python fixtures contain `#script (python)` blocks; the script logic
is reimplemented in Rust in `conformance_scripts.rs`.

### Ported, plain fixtures (19)

assumptions1, assumptions2, assumptions3, assumptions4, blocksworld1,
cancel, domain, external-lookup, externals, iclingo, infsup, interrupt,
logger, multi, parse-term, queens, ret, setconfig, show

`logger` compares with `logger.sol` minus the trailing newline of each
message, which `Message::text` removes by design.

### Ported, assumption-literal fixtures (2)

conflicting, test-numeric

`test-numeric` needs both literal forms (`Assumption`'s literal form and
`assign_external_literal`/`release_external_literal`); `conflicting` needs
only `is_conflicting`.
Both are ported in `conformance_scripts.rs`.

### Ported, ground program observer fixtures (2)

observer, observer-replace

Checked against `app/clingo/tests/python/observer.lp`/`observer-replace.lp`
directly: the `#script (python)` block reifies every observer callback into
ground terms and round-trips them through `@get()`, which needs no new API
beyond the observer itself, but a byte-for-byte port of the reification
pipeline is a large amount of code for coverage `clingox/tests/api_observer.rs`
already has, oracle value for oracle value, from focused fixtures (choose a
fixture where confusable values differ, rather than one mega-fixture). `observer.lp`'s own program (the `1 {a;b}. #minimize {...}.
#project a. #show x : a, b. #external a. #heuristic a : b. [1@2,sign] #edge
(a,b) : a, b.` plus the `#theory` block) is exactly the same shape covered,
piece by piece, by `an_observer_with_replace_false_sees_init_begin_rule_and_
the_program_still_solves`, `output_term_reports_a_shown_term_and_its_
condition`, `external_reports_the_declared_kind_for_free_true_and_false`,
`heuristic_and_acyc_edge_report_their_directives`, `weight_rule_minimize_and_
project_report_their_directives` and the theory round-trip tests, each
checked directly against clingo 5.8.2. `observer-replace.lp`'s only difference (`replace=True`,
and the final result is `UNKNOWN`) is `replace_true_reports_identical_calls_
but_nothing_reaches_the_solver`. This is the same "used directly as the oracle
and fixture source, counted as ported through that test" treatment
`backend_heuristic.lp`/`backend_project.lp` get.

### Ported, solve-event fixtures (3)

core1, core2, extend-model

Checked against `app/clingo/tests/python/core1.lp`/`core2.lp`/
`extend-model.lp` directly (`conformance_scripts.rs::script_core1`/
`script_core2`/`script_extend_model`). `core1`/`core2` need no
`SolveEventHandler` at all: pyclingo's `on_core` is not a clingo solve
event (clingo's C API has exactly four: model, unsat, statistics, finish),
it is Python-side sugar that calls `SolveHandle::core` itself after a
blocking call reports unsatisfiable
(`libpyclingo/clingo/control.py:1076-1084`). `extend-model`
needs `ExtendableModel::extend`, read back inside `on_model` through
`ShowType::THEORY` (clingox has no application layer to print the
extension the way the real `clingo` binary that `run.py` invokes does).

### Ported, backend fixtures (5)

add_atom, backend_acyc, backend_assume, backend_heuristic, backend_project

Six fixtures need the conformance suite's own `.sol`-comparison harness, not
just `Backend` itself: five of the six are Python fixtures, all ported
(`conformance_scripts.rs::script_add_atom`/`script_backend_acyc`/
`script_backend_assume`/`script_backend_heuristic`/`script_backend_project`),
checked directly against clingo 5.8.2. `backend_project.lp` needs its
`.cmd` file's `--project=project` argument alongside `--models=0`; the Lua
sixth fixture (`cover-lua`) is a duplicate, see the Lua section below.
`backend_heuristic.lp` narrows its own model limit back to `1` from inside
the script (`configuration.solve.models = "1"`), which the port reproduces
with `Control::configuration().set(...)` before the one `ScriptRun::solve`
call, honoured because `ScriptRun::solve` is built on `for_each_model`, not
`solve_all` (`conformance/mod.rs`'s own note on why).

### Ported, large multi-shot fixtures (5)

cover-py, project, statistics, sokoban, test

Four large multi-shot integration fixtures, plus `statistics`, all ported in
full.

- `cover-py.lp` (`conformance_scripts.rs::script_cover_py`): a nested,
  throwaway `Control` incrementally extends a path in a fixed graph (one
  more "leaf" vertex per step, each earlier choice hard-fixed through a
  `@fix(k)` ground callback) until no further extension is possible; the
  last satisfiable step's model then seeds a small vertex-cover subproblem
  on the outer control through three 0-ary ground callbacks
  (`@vertex()`/`@edge()`/`@cover()`). `cover-lua.lp` is a duplicate (see
  below).
- `project.lp` (`conformance_scripts.rs::script_project`): an embedded
  literal event sequence assembled into `seq/2` facts and an `nbs`
  constant, then a 4-step incremental search; `Control::get_const("ml")`
  never finds such a constant in this program, so the depth is the
  fixture's own hard-coded default, 4. Ported unchanged, including the
  primed variables (`P'`, `Q'`), which are ordinary clingo identifiers.
- `statistics.lp` (`conformance_scripts.rs::script_statistics`): pyclingo's
  dict-assignment convenience (`accu.update({...})`, an update-*function*
  value applied to an existing entry, and `accu["x"] = [99, 9]` merging
  into, not replacing, an existing longer array) has no `MutableStatistics`
  equivalent, but the *observable* final tree it builds does not need the
  convenience layer to reproduce: it is built directly from
  `set_value`/`push_array`/`add_map_key` (the same
  approach as `api_statistics_writing.rs` for `test_conf.py::test_user_stats`), checked against the
  Python oracle's own traced merge semantics, then read back into the same
  `dict(...)`/`list(...)` ground-term shape the fixture's own `tosymbol`
  builds.
- `sokoban.lp` (`conformance_scripts.rs::script_sokoban_optimizing`): a
  harder, optimising Sokoban variant from the Lua family below
  (`--opt-mode=optN`, two prioritised `#minimize` statements), whose `.cmd`
  (`-q1,2`) quiets every improving-but-not-yet-optimal model; checked
  directly against the reference application (`python3 -m clingo`) on
  synthetic optimisation fixtures, `-q1`'s actual filter is "do not print a
  model whose shown projection repeats the one just printed" (`Vec::dedup`
  on cost-tied models), not "keep only the last model" as a first reading
  suggested. The driving loop keeps going one step past the first
  satisfiable one (`e` counts down from 2) before stopping, and has a
  `t > 20` unsatisfiable escape hatch; both ported literally, including
  `release_external(volatile(-1))` on the very first iteration, a safe
  no-op checked directly (that atom is never part of any grounding).
- `test.lp` (`conformance_scripts.rs::script_test_python`): the real
  application's own model output is interleaved with the script's own
  `writeln` calls in the upstream `.sol` (`run.py`'s `normalize()` treats
  every printed block after an "Answer: N" marker as one line, sorting
  every line within a step together — `Step: 6`'s observed order, `SAT`
  then the real atom line then the script's own `hasA(...)` line then
  `on_finish`, is exactly ASCII order). No harness change was needed:
  `ScriptRun::push_step` already accepts a script-printed line wrapped as
  its own single-element `vec![line]`, which `normalise_fixture` joins back
  to that line unchanged before sorting it against the step's real model
  line. `SolveEventHandler`, not a captured stdout stream, supplies
  `on_model`/`on_finish`; the Lua original (`script_test_lua`) registers
  only `on_model` (no `on_finish`), the fixtures' one real difference,
  confirmed by diffing their `.sol` files (identical except for the
  `on_finish` lines).

### Ported, propagator clause fixtures (2)

tag, check-py

- `tag.lp` (`api_propagator_control.rs::
  clause_type_volatile_and_volatile_static_do_not_survive_a_new_step`/
  `clause_type_static_survives_a_new_step`): `ClauseType`'s own observable
  cross-step lifetime (a propagator adding an unconditional volatile clause
  once per solving step, controlled by a flag flipped between two `solve()`
  calls), generalised to all four `ClauseType` variants rather than only the
  volatile one the fixture itself exercises.
- `check-py.lp` (`propagator_check.rs::
  check_forces_every_unassigned_literal_true_in_turn`): `check`'s own default
  `CheckMode::Total`, switched to `Fixpoint`, forcing every unassigned
  literal true one at a time from `check` until the unique all-true model is
  reached.

### Ported, propagator fixtures (12)

add-clause-py, add_minimize, add_watch, add_weight, assignment, propagator,
wc1, wc2, wc3, wc4, wc5, wc6

All twelve in `conformance_scripts.rs`, checked directly against clingo
5.8.2 (each fixture's own `.sol`, byte for byte). Notes on the three least
mechanical ports:

- `add-clause-py.lp` (`script_add_clause_py`): the fixture's own third
  solving step adds an empty clause (an unconditional conflict) and then
  keeps calling `add_clause`/`propagate` regardless; pyclingo tolerates
  this, returning `False` each time, but clingox refuses any further call
  on an `init` that already reported `Flow::Stop` with
  `ErrorKind::InvalidInput` (`api_propagator_init.rs::
  a_further_call_after_stop_is_refused_by_clingox_itself`). The port stops
  issuing further calls once `Stop` is seen instead of reproducing every
  call literally; the fixture's own outcome (`UNSAT`) is already settled by
  the empty clause alone, so nothing about the result changes.
- `propagator.lp` (`script_propagator`, the sequence-mining fixture): ported
  line for line from the Python script, including the backend-pruning pass
  the driver adds after grounding (`grouped_pat`/`grouped_seq`/
  `projected_pat`), which turned out not to be redundant, checked directly
  (a version without it gives a different, wrong optimum on this program).
  Two findings surfaced while porting it:
  1. A 0-arity theory function (`&pat{...}`'s own designating term) comes
     back from `TheoryAtom::term()` as `TheoryTerm::Symbol`, not an
     empty-argument `TheoryTerm::Compound` the way `&seq(U){...}`'s
     one-argument term does; both are handled in the port.
  2. `solve_quiet_except_optimum` (the shared `-q1` emulation also used by
     `sokoban.lp`) does not filter by "same cost as the last
     model reported," which only approximates `-q1`'s real effect. Traced
     directly against the vendored `clasp` source: `Output::onModel` (`clasp/src/clasp_output.cpp`) prints a
     model exactly when clasp's own `Model::opt` bit is set ("whether the
     model is optimal w.r.t costs (0: unknown)", `clasp/clasp/
     enumerator.h`), which is *not* the same thing as "tied with the last
     model's cost": on this fixture, an improving model reported before
     optimality is proven shares its cost with the true optimum but is not
     itself flagged optimal, so a cost-based filter cannot tell
     the two apart (confirmed directly against pyclingo 5.8.2: the second
     of three cost-`-1` models has `optimality_proven == False`). The
     filter therefore uses `Model::optimality_proven` directly, which mirrors
     clasp's own bit exactly.
  `libclingo/tests/propagator.cc`'s own `propgator-sequence-mining`
  `TEST_CASE`/`sequence mining` `SECTION` (2 items, see the libclingo table
  below) is the identical algorithm and embedded ASP program; it is counted
  ported through this one test, not re-ported, matching the C++/java
  convention used for `solve_iter`.
- `add_watch.lp`: Lua's own script (a byte-identical duplicate, see below)
  watches thread 1 and asserts `thread_id == 1` where Python watches
  thread 0 and asserts `thread_id == 0` -- the Lua binding's own 1-based
  convention for what is otherwise the same single-threaded test.

### Not portable (1)

| Item | Reason |
|---|---|
| `free` | Not applicable to a library: the fixture uses `weakref`/`gc.collect()` to check that a Python `Propagator` wrapper object with no remaining Python references gets garbage-collected. Rust has no garbage collector and no equivalent lifecycle to observe; a `Propagator` clingox holds is owned exactly like any other Rust value, with no analogous "did this get freed" question a test could ask. |

## Lua fixtures (55 total, 19 ported, 36 not ported)

### Ported, plain fixtures (10)

icolor, incshow, mutex-bug, project_bug, robots, solitaire_para,
solitaire_sort, theory-term-types, toh, unsat-sync

`icolor.lp` and `incshow.lp` have no `#script` block at all: they are plain
`#include <incmode>` fixtures, ported with the same incmode driver
(`conformance/mod.rs::run_incmode`) rather than a script port.

### Ported, backend and theory fixtures (2)

project_bug2, theory

`project_bug2.lp` (`conformance_scripts.rs::script_project_bug2`) grounds two
program parts with a backend fact added between them; it is unique to Lua,
not a duplicate of any Python fixture (there is no `project_bug2.lp` under
`python/`). `theory.lp` (`conformance_scripts.rs::script_theory`) needs only
`TheoryAtoms` iteration and the ground callback (`p(@get())`); it has
no Python duplicate. Both checked directly against clingo 5.8.2 (`theory.lp`
also against pyclingo's own `clingo.Tuple_`/`clingo.String` construction, to
settle the exact element sort order, since the Lua binding's implicit
string-to-`String`-symbol conversion inside a `Tuple` constructor needed an
explicit `Symbol::string` call on the Rust side — see the port's own header
comment in `conformance_scripts.rs`).

### Ported, large multi-shot fixtures (7)

conformant1, conformant2, conformant3, sokoban, sokoban_back,
sokoban_para, test

`conformant1`-`3` (`conformance_scripts.rs::script_conformant1`/
`script_conformant2`/`script_conformant3`, about 200 to 330 lines of script
logic each, one shared driver, checked directly: byte-identical script
logic in all three) solve a conformant-planning instance: ground the
newest `step(t)`/`state(t)` parts, solve, and once a candidate plan is
found, ground and solve `check(t)` once, verifying it against every
admissible initial state under uncertainty, before accepting it.
`Control::get_const("nocheck")` never finds such a constant in any of the
three (checked directly), so the check step always runs on the first
candidate; the fixture's own quirk is ported literally too — `check` stays
`true` after one attempt regardless of whether it passed, so a later
step's first candidate is accepted unchecked.

`sokoban.lp`/`sokoban_back.lp`/`sokoban_para.lp`
(`conformance_scripts.rs::script_sokoban`/`script_sokoban_back`/
`script_sokoban_para`) share one incremental-deepening driver (byte-identical
script logic in all three, checked directly: only the ASP program itself
differs — forward pushes, backward pushes, a parallel-push variant). Each
step grounds one more `cumulative(k)`, moves the `volatile(k)` external
forward, and stops at the first satisfiable step, whose every model (there
can be more than one plan of the same length) becomes that step's own line.

`test.lp` (`conformance_scripts.rs::script_test_lua`) is the Lua sibling of
the Python fixture of the same name (see "Ported, large multi-shot fixtures (5)"
under Python fixtures); it registers only `on_model` (no `on_finish`),
confirmed by diffing the two `.sol` files (identical except for the
`on_finish` lines the Python original alone produces).

### Duplicates of Python fixtures (23)

add_atom, assumptions1, assumptions2, assumptions3, assumptions4,
blocksworld1, conflicting, core1, core2, cover-lua, domain, extend-model,
externals, iclingo, infsup, logger, observer, observer-replace, parse-term,
queens, setconfig, show, test-numeric

The Lua `blocksworld1` calls `cleanup()` between steps, which the Python
version does not. Otherwise the program and script are equivalent. Lua's
`observer.lp`/`observer-replace.lp` use the identical ASP program text as
their Python originals (checked directly, `diff` on the two program bodies)
and the same reification script logic in Lua instead of Python; they are
covered the same way the Python originals are (see "Ported, ground program observer fixtures (2)" above),
through `clingox/tests/api_observer.rs`, not through a separate Lua port.
`conflicting` and `test-numeric` are ported through their Python originals; the
Lua scripts are byte-for-byte equivalent logic (`tostring` capitalises
differently than Python's `str`, which is the only difference for
`conflicting`). `add_atom` is the same script logic and the same
`.sol` as its Python original, byte for byte; ported through
`script_add_atom` only, matching this convention.

`core1`/`core2`/`extend-model`/`cover-lua`: diffed
directly against their Python originals. `core1.lp`/`core2.lp` reimplement the identical core-blocking
algorithm in Lua's own async-handle idiom instead of Python's `on_core`
sugar (same ASP program body, byte-identical `.sol`); `extend-model.lp`
reimplements the same `symbols(theory=true)` comparison in Lua instead of
Python (same ASP program body, byte-identical `.sol`); `cover-lua.lp` is
the same nested-control incremental search
and vertex-cover encoding as `cover-py.lp`, only the scripting language
differs (byte-identical `.sol`). All four are ported through their Python
originals only, matching this convention; none needed its own separate
Lua port.

### Duplicates of Python propagator fixtures (13)

add-clause-lua, add_minimize, add_watch, add_weight, assignment, check-lua,
propagator, wc1, wc2, wc3, wc4, wc5, wc6

Every one diffed directly against its Python original:
`add_minimize.lp`, `add_weight.lp` and `wc1.lp`-`wc6.lp` are byte-identical
ASP programs and `.sol` files, the Lua script logic differing only in
syntax (`add_watch.lp` additionally differs in that Lua watches thread 1
and asserts `thread_id == 1` where Python watches thread 0 and asserts
`thread_id == 0`, the Lua binding's own 1-based convention, not a different
test). `assignment.lp`'s Lua script additionally indexes
`assignment[i]`/`trail[i]` through `__pairs`/`__ipairs`, sugar over the
same values `Assignment::at`/`Trail::at` already read in the Python port,
not a distinct invariant. `check-lua.lp` is a duplicate of `check-py.lp`
(already ported: `propagator_check.rs::
check_forces_every_unassigned_literal_true_in_turn`), same program and
script logic, same `.sol`. `propagator.lp` (the sequence-mining fixture)
reimplements the identical algorithm in Lua from scratch (its own helper
functions, since Lua lacks Python's built-ins), over the identical embedded
ASP program, checked directly; `add-clause-lua.lp` is the same three-step
`add_clause`/`propagate` sequence as `add-clause-py.lp`, byte for byte.
All thirteen are ported through their Python originals only
(`conformance_scripts.rs`); none needed its own separate Lua port.

## C examples (13 total, 13 ported, 0 not ported)

`CMakeLists.txt` is not counted: only the 13 real `.c` files are.

| Item | Reason |
|---|---|
| version | PORTED |
| control | PORTED |
| symbol | PORTED |
| configuration | PORTED |
| symbolic-atoms | PORTED |
| solve-async | PORTED |
| theory-atoms | PORTED (`conformance_examples.rs::example_theory_atoms`) |
| backend | PORTED (`conformance_examples.rs::example_backend`) |
| model | PORTED (`conformance_examples.rs::example_model`), checked directly against clingo 5.8.2: no arguments are passed, so clasp's default heuristic picks the model (`b` true, `a` false) the port's expected values are checked against |
| statistics | PORTED (`conformance_examples.rs::example_statistics`), the clasp statistics tree plus a user-written `user_accu` entry, read back the same way `libclingo_statistics`/`libclingo_set_statistics` already check it |
| application | PORTED (`conformance_examples.rs::example_application_*`, ten cases in child processes), checked against the C program itself: `application.c`, compiled and run against the libclingo 5.8.2 inside pyclingo, prints the transcripts the port asserts. Standard input is covered by `example_application_reads_a_program_from_standard_input` (piped facts) and `control_load_dash_slash_is_a_path_not_standard_input` |
| propagator | PORTED (`conformance_examples.rs::example_propagator`), the pigeon-hole propagator: 8 holes, 9 pigeons is UNSAT by the pigeonhole principle, checked directly against clingo 5.8.2. `libclingo/tests/propagator.cc`'s own `pigeon`/`unsat`/`sat` sections (the identical `PigeonPropagator` algorithm on smaller arguments) are ported alongside it in the same file rather than duplicated (`libclingo_propagator_pigeon_unsat`/`_sat`) |
| ast | PORTED (`conformance_examples.rs::example_ast`), checked against the C program itself: `ast.c`, compiled and run against the libclingo 5.8.2 inside pyclingo, prints the transcript the port asserts (default of one model per solve, so the `enable` run shows `b`) |

## libclingo C++ tests (105 items total, 104 ported, 1 not ported)

Recounted per the fixed unit above: `clingo.cc` (50: 2 `TEST_CASE` + 48
`SECTION`), `symbol.cc` (3: 1 `TEST_CASE` + 2 `SECTION`), `astv2.cc` (25: 4
`TEST_CASE` + 21 `SECTION`), `propagator.cc` (26: 2 `TEST_CASE` + 24
`SECTION`), `variant.cc` (1: 1 `TEST_CASE`, no `SECTION`).

### Ported, `clingo.cc`/`symbol.cc` (52 of 53)

- **`symbol.cc` (3 of 3):** the `symbol` `TEST_CASE` itself (a wrapper; its
  own body is just the two child `SECTION`s below, so it is ported exactly
  when they are), `signature`, `symbol` (`conformance_libclingo.rs::
  libclingo_signature`/`libclingo_symbol_types`).
- **`clingo.cc`, control and solving (24):** `parse_term`, the `solving` `TEST_CASE` itself
  (wrapper), `with control` (wrapper), `solve` (the `:101` occurrence, via
  its `add` child), `add`, `get_statistics`, `configuration`, `optimize`,
  `model`, `model-cost`, `assumptions`, `symbolic atoms` (the `:524`
  occurrence), `incremental`, `solve_iter`, `c++` and `java` (`:577`/`:582`,
  two nested children of `solve_iter`; both are duplicate C++-only iteration
  styles over the identical underlying assertions `solve_iter` itself
  already checks — clingox's `SolveHandle` has one idiomatic way to pull
  models, not two, so both are ported through the one existing
  `libclingo_solve_iter` test, not a separate one each), `logging`, `ground
  callback`, `ground callback fail`, `bug_classical_1`, `with single-shot
  control` (wrapper), `single-shot`.
- **`clingo.cc`, theory atoms (2):** `theory-atoms`, `theory-not`.
- **`clingo.cc`, observer (2):** `ground program observer`, `theory data bug`.
- **`clingo.cc`, signed signature lookup (1):** the second `solve` `SECTION`
  (`:126`, distinct from the `:101` one above; Catch2 allows two sibling
  `SECTION`s with the same literal name). Tests `SymbolicAtoms::by_signature`
  with a *signed* `Signature` (`Signature::with_sign`), not merely a name and
  arity (`conformance_libclingo.rs::libclingo_symbolic_atoms_by_signed_signature`).
- **`clingo.cc`, backend, models, events and cleanup (23):** `load` (`Control::load`), `set_statistics`
  (`SolveEventHandler::on_statistics`), `backend`, `backend-project`,
  `backend-external`, `backend-acyc`, `backend-weight-rule`,
  `backend-add-atom`, `backend-theory` (7 sections), `model-cautious`
  (`Model::kind`), `enable_enumeration_assumption` (both the `:611` and
  the byte-for-byte-identical `:620` occurrence; ported once), `cleanup`,
  `cleanup again`, `const`, `events`, `stop`, `goon` (3 sections:
  the shared `events` setup plus its two leaves), `on_unsat`,
  `pos_strat`, `bug_classical_2`, `bug_on_model`. All
  checked directly against clingo 5.8.2; see the port's own header comments
  in `conformance_libclingo.rs` for the oracle transcript of each.

### Ported, `clingo.cc` model context (1 of 50)

`model-add-clause` (`conformance_libclingo.rs::libclingo_model_add_clause`):
`Model::context`/`SolveControl::add_clause`. `1{a;b}1.`: exactly one
of `a`/`b` holds in any model, so negating whichever the current model
contains can never forbid a second one; `n == 1`, matching the section's
own `REQUIRE`, checked directly. `clingo.cc`/`symbol.cc` are now fully
ported (53 of 53).

### Ported, `propagator.cc` (26 of 26)

`propagator.cc`'s two `TEST_CASE`s (`propagator`, 24 nested sections;
`propgator-sequence-mining`, 1 nested section) are both fully ported, most
of them through the propagator test suites rather than re-ported here
(as for the "c++"/"java" sections above: a section whose assertions an
existing test already covers by structure, not merely by accident, is counted
ported through that test):

- **`propagator` (wrapper, ported exactly when its children are).**
- **`pigeon` (wrapper), `unsat`, `sat`:** `conformance_examples.rs`'s
  `libclingo_propagator_pigeon_unsat`/`_sat`, the same `PigeonPropagator`
  algorithm as `examples/c/propagator.c` (ported alongside it in that same
  file; see the C examples table above).
- **`assignment`:** `conformance_libclingo.rs::
  libclingo_propagator_assignment`. The section's own checks against its
  literal `1` (clasp's internal "trivially true" sentinel) are not ported:
  `SolverLiteral` has no public constructor from a raw value
  (`SolverLiteral::from_raw_valid` is crate-private, DESIGN S17: an
  out-of-range raw literal is a segfault hazard), so that literal is not
  reachable from outside the crate; every other assertion is ported.
- **`mode`:** `propagator_check.rs::
  check_forces_every_unassigned_literal_true_in_turn`: the identical
  "force every unassigned literal true from `check`, one at a time, until
  the unique all-true model is reached" algorithm as this section's own
  `TestMode`, under the same check mode -- checked directly against
  `clingo.hh`'s own `enum PropagatorCheckMode`: the C++ binding's
  `PropagatorCheckMode::Partial` is `clingo_propagator_check_mode_fixpoint`
  (value `2`), i.e. clingox's `CheckMode::Fixpoint`, *not* `Both` (value
  `3`, the C++ binding's own separate `Both` variant). Only the atom count differs
  (`{p(1..10)}.` there, `{p(1..9)}.` here), not the mechanism.
- **`add_watch`:** `api_propagator_init.rs::
  add_watch_to_thread_restricts_the_watch_to_one_thread`, a more thorough
  version of the same claim (a literal watched on one thread is only ever
  reported changed on that thread) over 5 runs, 8 literals and 4 threads,
  rather than this section's own fixed 2-thread condition-variable dance,
  which risks flakiness as any thread-ordering-sensitive test does.
- **`exception`, `exception-t2`:** `propagator_panics.rs`'s
  `a_panic_in_check_is_caught_and_resumes_after_the_solve` (single thread)
  and `a_panic_in_propagate_does_not_crash_the_process_with_four_threads`/
  `a_panic_in_undo_does_not_crash_the_process_with_four_threads` (the
  `exception-t2` sub-section's own `parallel_mode=2` angle, covered at 4
  threads instead of 2).
- **`add_clause` (wrapper), `learnt`, `static`, `volatile`,
  `volatile static`:** `api_propagator_control.rs::
  clause_type_volatile_and_volatile_static_do_not_survive_a_new_step`/
  `clause_type_static_survives_a_new_step`, which already generalise
  this exact "forbid once, toggle a flag, solve twice" shape to all four
  `ClauseType` variants -- the same shape `tag.lp`'s own port uses (see
  "Ported, propagator clause fixtures (2)" under Python fixtures above).
- **`add_clause_init` (wrapper), `conflict`, `propagate` (4 identically
  named sections):** `api_propagator_init.rs::
  add_clause_returning_stop_makes_the_program_unsatisfiable`,
  `add_literal_is_usable_in_add_clause_within_the_same_init_frozen_or_not`,
  `propagate_in_init_makes_added_clauses_effective_before_solving`:
  `PropagateInit::add_clause`'s own boolean return, `assignment().is_true`
  immediately reflecting it, and `propagate()` making it effective before
  solving proper starts, the exact claims this section's four `propagate`
  sub-sections each make once each.
- **`add_weight_constraint` (wrapper), `equal`:**
  `api_propagator_init.rs::weight_constraint_equivalence_holds_in_both_directions`
  plus `conformance_scripts.rs::script_wc3`/`script_wc6` (the
  `WeightConstraintKind::Equivalence` fixtures, which enumerate the full
  model set this section only checks `models.size() == 3` of).
- **`add_minimize` (wrapper), `minimize`:** `api_propagator_init.rs::
  add_minimize_extends_the_minimize_constraint` plus
  `conformance_scripts.rs::script_add_minimize` (the fixture this
  section's own program and literals come from).
- **`propgator-sequence-mining` (wrapper), `sequence mining`:**
  `conformance_scripts.rs::script_propagator`: the identical
  `SequenceMiningPropagator` algorithm and the identical embedded ASP
  program as `app/clingo/tests/python/propagator.lp`'s own fixture,
  checked directly by diffing both against this port's own transcription;
  ported through that one test, not duplicated, matching the C++/java
  `solve_iter` convention above.

### Not ported, `variant.cc` (1 of 1)

The `variant` `TEST_CASE` is a C++ utility test, not applicable to a library
with no `Variant` type of its own.

### Ported, `astv2.cc` (25 of 25)

All in `conformance_libclingo.rs`, each `SECTION` one test, every expected
value recomputed with pyclingo 5.8.2:

- **`parse-ast-v2` (8):** the `TEST_CASE` itself (a wrapper, ported when its
  sections are) and `statement`, `theory definition`, `body literal`, `head
  literal`, `literal`, `terms`, `theory terms`
  (`libclingo_parse_ast_v2_*`).
- **`add-ast-v2` (7):** the `TEST_CASE` (wrapper) and `statement`, `body
  literal`, `head literal`, `literal`, `terms`, `theory` (`libclingo_add_ast_v2_*`).
  The one `#script (lua)` line under `#ifdef WITH_LUA` in `statement` is
  compiled out here as it is upstream without Lua: the build has no Lua, and
  neither has the pyclingo oracle.
- **`build-ast-v2` (4):** the `TEST_CASE` (wrapper) and `string array`, `ast
  array`, `ast compare` (`libclingo_build_ast_v2_*`). Upstream edits a live
  C++ view of the node's array; the port edits the array through
  `insert_*_at`/`delete_*_at` and reads it back from the node.
- **`unpool-ast-v2` (6):** the `TEST_CASE` (wrapper) and `terms`, `head
  literal`, `body literal`, `statements`, `options` (`libclingo_unpool_ast_v2_*`).
  `options` covers all four flag combinations, `unpool(false, false)` being
  `Unpool::NONE`.

## pyclingo tests (72 methods total, 70 ported, 2 not ported)

### Ported, basic API (32 methods)

- `test_symbol.py` (9): test_parse, test_str, test_repr, test_cmp,
  test_match, test_number, test_function, test_infsup, test_string
- `test_control.py` (4): test_default, test_ground, test_ground_error,
  test_error_handling
- `test_solving.py` (7): test_solve_result_str, test_model_str,
  test_solve_cb, test_solve_yield, test_solve_async_yield,
  test_solve_interrupt, test_enum
- `test_conf.py` (2): test_config, test_simple_stats
- `test_atoms.py` (2): test_symbolic_atom, test_symbolic_atoms
- `test_aspif.py` (8): test_preamble, test_rule, test_minimize,
  test_external, test_assume, test_heuristic, test_edge, test_comment

### Ported, theory atoms (4 methods)

- `test_atoms.py` (3): test_theory_term, test_theory_element, test_theory_atom
  (`conformance_pyclingo.rs::pyclingo_theory_term`/`pyclingo_theory_element`/
  `pyclingo_theory_atom`)
- `test_aspif.py` (1): test_theory (`conformance_pyclingo.rs::pyclingo_aspif_theory`).
  Checked directly against clingo 5.8.2: `Control::add` parses raw aspif text
  through the same magic-header path pyclingo's `theory()` helper uses, so
  this needs no `Backend`, only `Control::add`, `symbolic_atoms`
  and `theory_atoms`.

### Ported, solve events and statistics (3 methods)

- `test_solving.py::test_solve_core` (`api_solve_core.rs`, already covering
  the same `3 { p(1..10) } 3.` core-reading scenario via `SolveHandle::core`
  : the pyclingo method needs no `SolveEventHandler`, only a
  blocking-shaped solve reporting unsatisfiable).
- `test_control.py::test_lower` (`api_solve_events.rs`): the lower-bound
  array `on_unsat` reports for a core-guided optimisation search.
- `test_conf.py::test_user_stats` (`api_statistics_writing.rs`): the
  primitive `set_value`/`push_array`/`add_map_key` calls that build the
  same `user_step`/`user_accu` tree pyclingo's own richer dict-assignment
  sugar builds; the update-function convenience
  (`step["test"] = {"a": lambda a: a + 1, ...}`) is pyclingo-only and not
  ported, since `MutableStatistics` exposes no equivalent (a deliberate
  difference, not a bug; the same gap surfaces in the `statistics.lp`
  fixture above).

### Ported, async solving and backend (5 methods)

- `test_solving.py::test_solve_async` (`conformance_pyclingo.rs::
  pyclingo_solve_async`). Ported through `Control::solve_async_with_events`, since plain
  `Control::solve_async` gives no model access at all by design; the
  event-taking form is the only one that can reproduce the models this test
  checks.
- `test_backend.py` (4): test_backend, test_theory, test_adding_theory,
  test_theory_with_guard (`conformance_pyclingo.rs::pyclingo_backend_observer`/
  `pyclingo_theory_observer`/`pyclingo_adding_theory`/
  `pyclingo_theory_observer_with_guard`). Every backend directive
  (`test_backend`) and every theory-authoring call (`test_adding_theory`)
  reaches a registered `GroundProgramObserver` with the same values,
  checked by structural equality against the `Atom`/`ProgramLiteral` values
  the port itself creates rather than by raw numeric id, since clingox keeps
  raw ids private (DESIGN S17) unlike pyclingo's plain integers.

### Ported, control operations (3 methods)

- `test_solving.py::test_remove_minimize`
  (`conformance_pyclingo.rs::pyclingo_remove_minimize`): `Control::
  remove_minimize` across three incrementally added `#minimize`
  statements, checked by `Model::cost` at each stage, ending with an empty
  cost once every minimize statement is removed.
- `test_solving.py::test_cautious_consequences`
  (`conformance_pyclingo.rs::pyclingo_cautious_consequences`):
  `Model::is_consequence` (returns `Consequence`) during cautious
  enumeration (`configuration.solve.enum_mode = "cautious"`). Checked
  directly against clingo 5.8.2: the first (intermediate) model reports
  `c`'s consequence status as `Consequence::Unknown`, not yet refined to
  `False`, which the final model (`Model::number() == 2`) does report.
- `test_solving.py::test_update_projection`
  (`conformance_pyclingo.rs::pyclingo_update_projection`):
  `Control::update_project` replacing (`append: false`) and
  extending (`append: true`) the projection atoms. The Python original
  builds one projection set from a mix of a literal and a plain symbol;
  `update_project` takes symbols only, so both are passed as symbols,
  equivalent since clingox looks up each symbol's own literal internally
  either way.

### Ported, propagators (5 methods)

- `test_propagator.py::TestSymbol::test_propagator_control`
  (`api_propagator_control.rs::propagator_control_full_surface_matches_the_
  oracle`): the full `PropagateControl` surface exercised together --
  `thread_id`, `assignment`, `Trail`, `decision`, `has_watch`, this
  type's own `propagate`, and `add_clause` returning `Stop`.
- `test_propagator.py::TestSymbol::test_propagator_mode`
  (`propagator_check.rs::check_and_undo_fire_together_under_fixpoint_and_
  always`): `check`/`undo` firing counts together under `CheckMode::
  Fixpoint` and `UndoMode::Always` (`num_check == num_undo + 1`).
- `test_propagator.py::TestSymbol::test_propagator`
  (`api_propagator_control.rs::add_literal_and_watch_methods_work_from_
  check`): `add_literal`/`has_watch`/`add_watch`/`remove_watch` used
  together from `check`.
- `test_propagator.py::TestAddAssertingClause::test_default` and
  `::test_locked` (`api_propagator_control.rs::
  asserting_clause_forces_a_conflict_without_changing_the_decision_level`,
  both `ClauseType::Learnt` and `::Static` via the Python original's own
  `lock` parameter): `add_clause` as an asserting/conflict clause, `decide`
  used only as a search-order driver, not as a behavioural test of `decide`
  itself.

### Ported, heuristics and clauses (3 methods)

- `test_propagator.py::test_heurisitc` (upstream's own spelling;
  `conformance_pyclingo.rs::pyclingo_heuristic`): needs only `decide`'s
  dispatch and `Assignment`, so it is ported separately from the
  rest of `test_propagator.py` (the other six methods need the fuller
  `PropagateControl`; five are ported above, and `test_propagator_init` is
  covered elsewhere, see the last table).
- `test_solving.py::test_control_clause` and `::test_control_nogood`
  (`conformance_pyclingo.rs::pyclingo_control_clause`, one test closes
  both rows): checked directly, neither method registers a propagator at all; both need
  only `Model::context`/`SolveControl::add_clause`, through a plain
  `yield_=True` model loop. The two upstream methods are behaviourally
  identical once translated: `test_control_nogood`'s own
  `add_nogood(clause)` is pyclingo's `add_clause([invert(lit) for lit in
  clause])` (`libpyclingo/clingo/solving.py:159-176`), and its clause
  already contains the pre-negation of the same literal
  `test_control_clause` passes to `add_clause` directly, so both compile
  to the identical clingox `add_clause` call; clingox has no `add_nogood`
  convenience (not obviously worth a one-line wrapper, RULES §4), so one
  port covers both.

### Ported, AST (14 methods)

All of `test_ast.py`, in `conformance_pyclingo.rs`. The upstream file was run
against the installed pyclingo 5.8.2 first (14 of 14 pass), and the port
reproduces its `_str` and `_deepcopy` helpers: each statement is rebuilt
bottom up with the typed constructors, copied, deep copied, printed, reparsed
and added through the program builder.

- `test_terms`, `test_theory_terms`, `test_literals`, `test_head_literals`,
  `test_body_literals`, `test_statements` (`pyclingo_ast_terms` and so on, 167
  `_str` cases in all).
- `test_compare`, `test_compare_bug` (`pyclingo_ast_compare`,
  `pyclingo_ast_compare_bug`).
- `test_ast_sequence`, `test_str_sequence` (`pyclingo_ast_sequence`,
  `pyclingo_str_sequence`): pyclingo's `ASTSequence`/`StrSequence` are Python
  list views; the port makes the same edits with the array setters on the node.
- `test_transformer` (`pyclingo_ast_transformer`): a `Visitor`.
- `test_comment_order` (`pyclingo_ast_comment_order`).
- `test_repr` (`pyclingo_ast_repr`): upstream evaluates `repr(stms)`, Python
  source that calls the constructors with the node's attributes; Rust has no
  evaluable `Debug` text, so the port rebuilds every node with its typed
  constructor and compares the lists, locations included.
- `test_unpool` (`pyclingo_ast_unpool`): all four flag combinations, `unpool(other=False,
  condition=False)` being `Unpool::NONE`.

### Ported, application (1 method)

- `test_application.py::test_app` (`conformance_pyclingo.rs::
  pyclingo_application_app`): in process, as `--outf=3` silences the run, with
  the queue a list. Run under pyclingo 5.8.2 the upstream file gives the models
  in the order `a`, `b`, `a b`, not the order the assertion states; the port
  asserts the measured order.

### Not ported (2 methods)

| Method | Reason |
|---|---|
| `test_solving.py::test_model` | The `on_last`-shaped second half calls `Model.context`/`Model.is_true`/`Model.thread_id` on the *same* model `on_model` already saw, but with `optimality_proven` now `True` — reached through `handle.last()` after the search has fully finished (`libpyclingo/clingo/control.py:1083-1088`), a live model handed back by `clingo_solve_handle_t` *after* a completed, non-yielding solve. `SolveHandle` exposes `next_model`/`get`/`cancel`/`close`/`core`; `Control::solve_optimal` is the nearest thing, but it returns a snapshotted `OwnedModel`, which has none of `context`/`is_true`/`thread_id`. This is a small, standalone `SolveHandle` gap: there is no way to reach a live, still-accessorized `Model` after a completed solve. Not attempted as a partial port: dropping one asserted call to route around a missing wrapper is not the same test. |
| `test_propagator.py::TestSymbol::test_propagator_init` | Covered rather than ported: it needs `PropagateInit`/`Assignment`, and is already substantively covered by `api_propagator_init.rs` (`add_weight_constraint`, `add_minimize`, `decision_level`, `root_level`, `has_conflict`, `size`, `is_total`, `has_literal` all directly exercised there) and `api_trail.rs`/`api_assignment.rs`. Not ported as a literal copy, to avoid duplicating that coverage. |
