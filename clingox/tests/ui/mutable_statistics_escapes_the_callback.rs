// `MutableStatistics<'a>` borrows the statistics object for exactly the
// duration of the `on_statistics` call that received it (see
// DESIGN S6). Keeping either the `step` or the `accumulated`
// tree past that call must not compile.
// Expected: a lifetime mismatch (E0521 or E0495).
use std::ops::ControlFlow;

use clingox::{Control, MutableStatistics, Part, Result, SolveEventHandler, SolveResult};

struct Escape<'a> {
    slot: &'a mut Option<&'a mut MutableStatistics<'a>>,
}

impl<'a> SolveEventHandler for Escape<'a> {
    fn on_statistics(
        &mut self,
        step: &mut MutableStatistics<'_>,
        _accumulated: &mut MutableStatistics<'_>,
    ) -> Result<ControlFlow<()>> {
        *self.slot = Some(step);
        Ok(ControlFlow::Continue(()))
    }

    fn on_finish(&mut self, _result: SolveResult) -> Result<ControlFlow<()>> {
        Ok(ControlFlow::Continue(()))
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut kept = None;
    ctl.solve_with_events(clingox::SolveOptions::new(), Escape { slot: &mut kept })
        .unwrap();
    let _ = kept;
}
