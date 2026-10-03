// A statistics entry borrows the control, so solving while it is alive is
// statically impossible (DESIGN S5), even though the entry outlives the
// view that made it.
// Expected: E0502, `ctl` borrowed as mutable while also borrowed as immutable.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let root = ctl.statistics().unwrap().root();
    let _result = ctl.solve(&[]);
    let _kind = root.kind();
}
