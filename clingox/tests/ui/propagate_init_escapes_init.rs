// `PropagateInit<'i>` borrows the C object for exactly the duration of the
// `init` call that received it (see the safety analysis: it
// is not tied to `Control` by anything, and must not be stored
// past the callback). Keeping a reference to it past that call must not
// compile. `Propagator::init` takes `&self` (S11), not `&mut self`, so the
// escape attempt needs interior mutability to even reach the lifetime
// check, the same way a real misuse would; `RefCell` is used rather than
// `Cell` because the stored value is not `Copy`.
// Expected: a lifetime mismatch (E0521 or E0495), the same shape
// `extendable_model_escapes_the_callback.rs` pins for `ExtendableModel<'_>`.
use std::cell::RefCell;

use clingox::propagate::{PropagateInit, Propagator};
use clingox::{Control, Part, Result};

struct Escape<'a> {
    slot: RefCell<Option<&'a mut PropagateInit<'a>>>,
}

impl Propagator for Escape<'_> {
    fn init(&self, init: &mut PropagateInit<'_>) -> Result<()> {
        // `init`'s real lifetime is this call's own scope, unrelated to
        // `'a`: the assignment below must be rejected.
        *self.slot.borrow_mut() = Some(init);
        Ok(())
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Escape {
        slot: RefCell::new(None),
    })
    .unwrap();
    ctl.solve(&[]).unwrap();
}
