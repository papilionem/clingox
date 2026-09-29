// `Control::solve_async_with_events` runs the handler's callbacks on
// clasp's own thread (DESIGN S10: "solve-event handler (`solve_async`) |
// solver threads | `Send + 'static`, owned"; DESIGN S6),
// so a handler
// and the blocking and yielding forms reject it too. Here the
// bound is `Send + 'static`; see
// `ui/non_send_handler_rejected_by_solve_with_events.rs` for the blocking form.
// Expected: E0277, `Rc<Cell<u32>>` cannot be sent between threads safely.
use std::cell::Cell;
use std::ops::ControlFlow;
use std::rc::Rc;

use clingox::{Control, ExtendableModel, Part, Result, SolveEventHandler};

struct NotSend {
    count: Rc<Cell<u32>>,
}

impl SolveEventHandler for NotSend {
    fn on_model(&mut self, _model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        self.count.set(self.count.get() + 1);
        Ok(ControlFlow::Continue(()))
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let handler = NotSend {
        count: Rc::new(Cell::new(0)),
    };
    let _ = ctl.solve_async_with_events(&[], handler);
}
