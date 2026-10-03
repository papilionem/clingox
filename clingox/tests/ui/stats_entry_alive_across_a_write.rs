// An entry of the writable statistics borrows it shared, so writing while the
// entry is alive is statically impossible (DESIGN S5).
// Expected: E0502, `*step` borrowed as mutable while also borrowed as immutable.
use std::ops::ControlFlow;

use clingox::{Control, MutableStatistics, Part, Result, SolveEventHandler};

struct WriteWhileReading;

impl SolveEventHandler for WriteWhileReading {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> Result<ControlFlow<()>> {
        let entry = step.root();
        let _ = step.set_value("x", 1.0);
        let _kind = entry.kind();
        Ok(ControlFlow::Continue(()))
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let _ = ctl.solve_with_events(clingox::SolveOptions::new(), WriteWhileReading);
}
