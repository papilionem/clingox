// While a solve handle exists, the control cannot be changed: grounding during
// a search is statically impossible (DESIGN S5).
// Expected: E0499, `ctl` borrowed mutably twice.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    ctl.add_base("b.").unwrap();
    let _model = handle.next_model();
}
