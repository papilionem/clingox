// A `TheoryAtoms` view borrows the control (DESIGN S5, as `SymbolicAtoms` and
// `Statistics` already do), so grounding while it is alive is statically
// impossible.
// Expected: E0502, `ctl` borrowed as mutable while also borrowed as immutable.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.theory_atoms().unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = atoms.len();
}
