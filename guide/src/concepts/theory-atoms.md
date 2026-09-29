# Theory atoms

A `#theory` block lets a program declare its own syntax for statements clingo
does not interpret itself, such as `&sum { 1; 2 } <= 3.`. clingo parses these
statements and grounds them like any other part of the program, but it does
not act on them: what a theory atom means is up to the application, usually
through a propagator or by translating it into ordinary rules with the
backend (both later additions to clingox; see the introduction's status).
`Control::theory_atoms` is how a program reads back what clingo parsed, the
same way `Control::symbolic_atoms` reads back ordinary atoms.

```rust
use clingox::{Control, Part, TheoryTerm};

let mut ctl = Control::new()?;
ctl.add_base(
    "#theory t { term { }; &sum/0 : term, {<=}, term, head }. \
     &sum { 1; 2 } <= 3.",
)?;
ctl.ground(&[Part::base()])?;

let atoms = ctl.theory_atoms()?;
assert_eq!(atoms.len()?, 1);
let atom = atoms.iter().next().unwrap()?;

// The elements are the terms before `:`, one per `;`-separated item.
let elements = atom.elements()?;
assert_eq!(elements.len(), 2);
for element in &elements {
    let tuple = element.tuple()?;
    assert_eq!(tuple.len(), 1);
    let term = atoms.term(tuple[0])?;
    assert!(matches!(term, TheoryTerm::Number(1) | TheoryTerm::Number(2)));
}

// The guard is the connective and term after `:`.
let (connective, term) = atom.guard()?.expect("&sum has a guard");
assert_eq!(connective, "<=");
assert_eq!(term, TheoryTerm::Number(3));
# Ok::<(), clingox::Error>(())
```

## The four views

- **`TheoryAtoms`**, from `Control::theory_atoms`, is a view of every theory
  atom of the current grounding, in the spirit of `SymbolicAtoms`. `len`,
  `is_empty` and `iter` work as they do there; `term_kind` and `term` read a
  term by its `Id` (below).
- **`TheoryAtom`** is one theory atom: its own `term` (`a` in `&a { ... }`),
  its `elements`, its `guard`, and its program `literal`.
- **`TheoryElement`** is one element of an atom's braces: its `tuple` (the
  terms before `:`), its `condition` (an owned `Vec` of the literals after
  `:`, copied out because clingo hands them back through a buffer it reuses
  for every element), and its `condition_id`, `None` for an element with no
  condition and otherwise the id a propagator can map to a solver literal.
- **`TheoryTerm`** is a resolved term: `Number`, `Symbol`, or `Compound` for a
  tuple, list, set or function, whose arguments are themselves resolved
  `TheoryTerm`s. `TheoryAtoms::term` builds the whole tree in one call, so
  there is no id left over that a caller could feed to the wrong accessor.
  `Display` prints it exactly as clingo does, including a theory operator
  printed infix (`(1+2)`) or prefix (`(-1)`) instead of `name(args)`.

## Lifetimes and ids

`TheoryAtoms`, `TheoryAtom`, `TheoryElement` and `TheoryTerm` all borrow the
`Control`, exactly as `SymbolicAtoms` and `Statistics` do: the control cannot
ground or solve again while one of them is alive, which the compiler enforces
at the borrow the same way it would for a held `SymbolicAtoms`.

Term, element and atom ids (`Id`) are clingo's own numbers, consecutive from
zero. clingo resets all of them after each `Control::solve`, so grounding a
fresh theory atom afterward can reuse an earlier id. `Id` has no public
constructor: every value in reach came from `TheoryAtoms::iter` or from an
accessor that read it off an atom clingox already resolved, so a program never
constructs one to look up on its own.

## A guard's absence, and a directive atom's missing literal

`TheoryAtom::guard` is `None` when the atom has none, such as plain `&a { 1 }.`
with no `<=`, `=` or other connective after the braces.

`TheoryAtom::literal` is `None` for an atom declared with the `directive`
role, such as `&sum/0 : term, {<=}, term, directive` used as a bare statement
that is never part of a rule body or head. clingo gives such an atom no
program literal at all; `Some(literal)` is what every other atom gets.
