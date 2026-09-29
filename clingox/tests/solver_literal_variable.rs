//! `SolverLiteral::variable` (smoke finding B3): the solver variable of a
//! literal, its absolute value, as a dense `u32` index for a propagator's own
//! per-variable tables (the clingo crate's `lit.get_integer()`).
//!
//! Oracle: pyclingo 5.8.2, `init.solver_literal(atom.literal)` on
//! `{a;b;c}. d :- a, b.` returns 2, 3, 4, 5 for a, b, c, d, and -2, -3, -4,
//! -5 for their negations; the variable is the absolute value.

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use std::sync::{Arc, Mutex};

use clingox::propagate::{PropagateInit, Propagator};
use clingox::{Control, Part, Result};

#[derive(Default)]
struct Seen {
    /// (atom, variable, variable of the negation, positive, assignment size)
    atoms: Vec<(String, u32, u32, bool, usize)>,
}

struct Reader(Arc<Mutex<Seen>>);

impl Propagator for Reader {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        let size = init.assignment().size();
        let mut seen = self.0.lock().unwrap();
        for atom in &init.symbolic_atoms()? {
            let atom = atom?;
            let literal = init.solver_literal(atom.literal())?;
            seen.atoms.push((
                atom.symbol().to_string(),
                literal.variable(),
                literal.negate().variable(),
                literal.is_positive(),
                size,
            ));
        }
        Ok(())
    }
}

fn seen(program: &str) -> Vec<(String, u32, u32, bool, usize)> {
    let shared = Arc::new(Mutex::new(Seen::default()));
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Reader(Arc::clone(&shared)))
        .unwrap();
    let _ = ctl.solve(&[]).unwrap();
    let mut atoms = std::mem::take(&mut shared.lock().unwrap().atoms);
    atoms.sort();
    atoms
}

#[test]
fn the_variable_is_the_absolute_value_pyclingo_reports() {
    let atoms = seen("{a;b;c}. d :- a, b.");
    let variables: Vec<(&str, u32)> = atoms.iter().map(|(n, v, ..)| (n.as_str(), *v)).collect();
    assert_eq!(variables, [("a", 2), ("b", 3), ("c", 4), ("d", 5)]);
}

#[test]
fn a_literal_and_its_negation_share_the_variable() {
    for (name, variable, negated, positive, _) in seen("{a;b;c}. d :- a, b.") {
        assert_eq!(variable, negated, "{name}");
        assert!(
            positive,
            "{name}: pyclingo returns positive solver literals"
        );
    }
}

#[test]
fn variables_are_dense_indexes_into_the_assignment() {
    for (name, variable, _, _, size) in seen("{p(1..20)}.") {
        assert!(variable >= 1, "{name}: a variable is never 0");
        // The size counts variables from 1, so the largest variable equals it.
        assert!(
            (variable as usize) <= size,
            "{name}: {variable} within {size}"
        );
    }
}
