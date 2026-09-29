// The blocking entry point drops the handler before it returns, so a handler
// that borrows the caller's locals compiles there. It must still be `Send`, because the search runs on
//
// clasp's thread. The yielding and asynchronous forms need
// `'static`: see `ui/borrowing_handler_rejected_by_solve_yield_with_events.rs`.
use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, SolveEventHandler};

struct Borrowing<'a> {
    count: &'a mut u32,
}

impl SolveEventHandler for Borrowing<'_> {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> clingox::Result<ControlFlow<()>> {
        *self.count += 1;
        Ok(ControlFlow::Continue(()))
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{a;b}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut count = 0_u32;

    ctl.solve_with_events(
        clingox::SolveOptions::new(),
        Borrowing { count: &mut count },
    )
    .unwrap();
    assert!(count > 0);
}
