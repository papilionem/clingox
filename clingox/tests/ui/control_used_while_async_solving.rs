// While an async solve handle exists, the control cannot be changed: the search
// runs on another thread and reads the program (DESIGN S5).
// Expected: E0499, `ctl` borrowed mutably twice.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_async(&[]).unwrap();
    ctl.add_base("b.").unwrap();
    let _done = handle.wait(std::time::Duration::ZERO);
}
