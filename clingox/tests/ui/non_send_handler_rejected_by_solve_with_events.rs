// A blocking solve runs on clasp's thread where clasp has threads, and
// parallel solving reports models from solver threads, so even the blocking
// and yielding entry points need a `Send` handler.
//
// Expected: E0277, `Rc<Cell<u32>>` cannot be sent between threads safely.
use std::cell::Cell;
use std::ops::ControlFlow;
use std::rc::Rc;

use clingox::{Control, ExtendableModel, Part, Result, SolveEventHandler, SolveOptions};

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
    let _ = ctl.solve_with_events(SolveOptions::new(), handler);
}
