// A statistics view borrows the control, so solving while it is alive is
// statically impossible (DESIGN S5).
// Expected: E0502, `ctl` borrowed as mutable while also borrowed as immutable.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let stats = ctl.statistics().unwrap();
    let _result = ctl.solve(&[]);
    let _total = stats.value("summary.times.total");
}
