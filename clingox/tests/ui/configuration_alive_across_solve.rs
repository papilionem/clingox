// A configuration view holds the control mutably (DESIGN S5), so the control
// cannot solve while it is alive.
// Expected: E0499, `ctl` borrowed mutably twice.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut conf = ctl.configuration();
    let _result = ctl.solve(&[]);
    let _ = conf.set("solve.models", "0");
}
