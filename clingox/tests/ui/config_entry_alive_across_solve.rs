// A configuration entry keeps the view alive, and the view holds the control
// mutably, so the control cannot solve while the entry is alive (DESIGN S5).
// Expected: E0499, `ctl` borrowed mutably twice.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let conf = ctl.configuration();
    let root = conf.root().unwrap();
    let _result = ctl.solve(&[]);
    let _kind = root.kind();
}
