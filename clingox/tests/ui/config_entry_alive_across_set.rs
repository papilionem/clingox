// A configuration entry borrows the view, so changing the configuration while
// the entry is alive is statically impossible (DESIGN S5).
// Expected: E0502, `conf` borrowed as mutable while also borrowed as immutable.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    let mut conf = ctl.configuration();
    let root = conf.root().unwrap();
    let _ = conf.set("solve.models", "0");
    let _kind = root.kind();
}
