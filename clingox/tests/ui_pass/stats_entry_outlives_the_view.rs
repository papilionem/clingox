// A statistics entry has the lifetime of the control borrow, not of the view
// that made it (DESIGN S5), so it can be kept after the view is gone.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let root = ctl.statistics().unwrap().root();
    let call = root.entry("summary.call").unwrap().value().unwrap();
    assert!(call.abs() < f64::EPSILON, "{call}");
}
