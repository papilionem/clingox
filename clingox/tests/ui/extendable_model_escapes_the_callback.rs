// `ExtendableModel<'m>` borrows the model pointer for exactly the duration
// of the `on_model` call that received it (nothing here can be
// stored past the callback). Keeping a
// reference to it past that call must not compile.
// Expected: a lifetime mismatch (E0521 or E0495), the same shape
// `backend_escapes_the_closure.rs` pins for `Backend<'_>`.
use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, Result, SolveEventHandler};

struct Escape<'a> {
    slot: &'a mut Option<&'a mut ExtendableModel<'a>>,
}

impl<'a> SolveEventHandler for Escape<'a> {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        // `model`'s real lifetime is this call's own scope, unrelated to
        // `'a`: the assignment below must be rejected.
        *self.slot = Some(model);
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
