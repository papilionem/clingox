// `SolveControl<'m>` borrows the same underlying model object
// `Model::context` was called on, for exactly the duration of the `&Model`
// borrow that produced it (`SolveControl<'m>`'s
// lifetime is tied to the Model it came from and cannot outlive the
// callback or the model-inspection call that produced it, mirroring
// `ExtendableModel<'m>`'s own rule). Keeping one past its
// callback must not compile.
//
// Expected: a lifetime mismatch (E0521 or E0495), the same shape
// `extendable_model_escapes_the_callback.rs` pins for
// `ExtendableModel<'_>`.
//
//

use std::ops::ControlFlow;

use clingox::{Control, ExtendableModel, Part, Result, SolveControl, SolveEventHandler};

struct Escape<'a> {
    slot: &'a mut Option<SolveControl<'a>>,
}

impl<'a> SolveEventHandler for Escape<'a> {
    fn on_model(&mut self, model: &mut ExtendableModel<'_>) -> Result<ControlFlow<()>> {
        // `model.context()`'s real lifetime is this call's own scope,
        // unrelated to `'a`: the assignment below must be rejected.
        *self.slot = Some(model.context());
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
