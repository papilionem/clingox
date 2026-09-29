// An iterator over symbolic atoms borrows the control, so grounding while it
// is alive is statically impossible (DESIGN S5).
// Expected: E0502, `ctl` borrowed as mutable while also borrowed as immutable.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let atoms = ctl.symbolic_atoms().unwrap().iter();
    ctl.ground(&[Part::base()]).unwrap();
    for atom in atoms {
        let _ = atom;
    }
}
