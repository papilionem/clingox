// A statistics entry cannot outlive the control whose statistics it reads
// (DESIGN S5).
// Expected: E0597, `ctl` does not live long enough.
use clingox::{Control, StatsEntry};

fn main() {
    let root: StatsEntry<'_>;
    {
        let ctl = Control::new().unwrap();
        root = ctl.statistics().unwrap().root();
    }
    let _kind = root.kind();
}
