# The backend

Grounding turns program text into ground rules, constraints and directives that
the solver understands. `Control::with_backend` gives a program direct access
to those same directives, so it can add already-ground statements without
writing any text for the grounder to parse.

```rust
use std::ops::ControlFlow;

use clingox::backend::Head;
use clingox::{Control, Part};

let mut ctl = Control::with_args(["--models=0"])?;
ctl.add_base("")?;
let (a, aux) = ctl.with_backend(|b| {
    let a = b.add_atom(Some("a".parse()?))?;
    let aux = b.add_aux_atom()?;
    b.add_rule(Head::Normal(&[a]), &[])?; // a.
    b.add_rule(Head::Choice(&[aux]), &[a.pos()])?; // { aux } :- a.
    b.add_rule(Head::Constraint, &[aux.neg()])?; // :- not aux.
    b.add_minimize(0, &[(aux.pos(), 1)])?;
    Ok((a, aux))
})?;
ctl.ground(&[Part::base()])?;

ctl.for_each_model(&[], |model| {
    assert!(model.is_true(a.pos())?);
    assert!(model.is_true(aux.pos())?);
    assert_eq!(model.cost()?, [1]);
    Ok(ControlFlow::Continue(()))
})?;
# Ok::<(), clingox::Error>(())
```

This is the same program `a. {aux} :- a. :- not aux. :~ aux. [1@0]` would
ground from text, built one directive at a time instead. `aux` never gets a
symbol (`add_aux_atom` is `add_atom(None)`), so it can only be observed
through its literal, not by name.

## Opening and closing

`with_backend` opens the backend before its closure runs and closes it
afterward, whatever happens inside: the closure's `Ok`, its `Err`, or a panic.
The `Backend<'_>` argument borrows the control mutably for the closure's whole
duration, so grounding, solving or opening a second backend cannot happen
until the closure returns; the compiler rejects an attempt to do so at
compile time, the same way it rejects keeping the `Backend<'_>` handle itself
past the closure. A plain value the closure computes and returns, such as an
`Atom`, is not a reference and can be kept and used freely afterward, as `a`
and `aux` are above.

## Atoms and literals

`Atom` is a newtype for an atom of the ground program, a separate id space
from `ProgramLiteral`. `Atom::pos` and `Atom::neg` turn an atom into the
positive or negative literal a rule body, a minimize constraint, an
assumption or a heuristic condition takes.

`add_atom(Some(symbol))` interns by symbol: calling it for a symbol that
program text also grounds, in either order, gives back the very same atom.
`add_aux_atom` (`add_atom(None)`) creates an atom with no symbol at all; it
never appears in `SymbolicAtoms`, and a model can only report its truth value
by literal (`Model::is_true`), never by symbol.

## Directives

Besides `add_rule` and `add_weight_rule`, the backend adds every other ground
directive clingo's aspif format has: `add_minimize` for optimisation,
`add_project` for projected enumeration (once `solve.project` is enabled),
`add_external` for `#external`-style atoms (`ExternalKind::Free`, `True`,
`False` or `Release`), `add_assumptions` for a one-shot assumption that only
applies to the next solve call, `add_heuristic` for a domain heuristic
modification, and `add_edge` for the acyclicity graph.

`add_weight_rule` checks its lower bound and every weight itself, before
calling clingo: both must be positive, and anything else is
`ErrorKind::InvalidInput`. clingo accepts a zero weight or bound silently,
which makes the rule vacuous or drops a literal from the sum without warning.
It rejects a negative weight only with an error that would leave the control
unusable, although the rule never took effect.

## Theory terms and atoms

The backend also builds theory terms, elements and atoms directly:
`add_theory_number`, `add_theory_string`, `add_theory_sequence`,
`add_theory_function` and `add_theory_symbol` build terms; `add_theory_element`
groups a tuple and a condition; `add_theory_atom` and
`add_theory_atom_with_guard` build the atom itself. They return the same `Id`
type `Control::theory_atoms` uses to read theory atoms back (see
[Theory atoms](theory-atoms.md)), so a backend-authored theory atom round-trips
through the same reading API as one grounded from `#theory` text.

`add_theory_atom`'s `atom` parameter is `TheoryAtomTarget`: `Directive` for an
atom that gets no program literal and is never part of a rule,
`Fresh` for a new atom with a real literal, or `Atom(existing)` to attach the
theory term to an atom already in scope.

```rust
use clingox::backend::TheoryAtomTarget;
use clingox::{Control, Part, TheoryTerm};

let mut ctl = Control::new()?;
ctl.add_base("#theory t { term { }; &a/0 : term, {>=}, term, directive }.")?;
ctl.ground(&[Part::base()])?;
ctl.with_backend(|b| {
    let n = b.add_theory_number(42)?;
    let term = b.add_theory_function("g", &[n])?;
    let guard = b.add_theory_number(7)?;
    b.add_theory_atom_with_guard(TheoryAtomTarget::Fresh, term, &[], ">=", guard)
})?;

let atoms = ctl.theory_atoms()?;
let atom = atoms.iter().next().unwrap()?;
assert!(atom.literal()?.is_some(), "a fresh atom gets a real literal");
let (connective, term) = atom.guard()?.expect("the atom has a guard");
assert_eq!((connective, term), (">=", TheoryTerm::Number(7)));
# Ok::<(), clingox::Error>(())
```
