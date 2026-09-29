// A yield handle can be leaked (`mem::forget` is safe, DESIGN S4), which ends
// its borrows while the search, and the handler in it, is still open. So the
// yielding form takes only `'static` handlers (the blocking form drops the handler
// before it returns).
// Expected: E0597, `count` does not live long enough.
use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, Result, SolveEventHandler};

struct Borrowing<'a> {
    count: &'a mut u32,
}

impl SolveEventHandler for Borrowing<'_> {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        *self.count += 1;
        Ok(ControlFlow::Continue(()))
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut count = 0_u32;
    let handle = ctl
        .solve_yield_with_events(&[], Borrowing { count: &mut count })
        .unwrap();
    std::mem::forget(handle);
}
