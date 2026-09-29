// `Assignment<'_>` is borrowed for exactly the duration of the callback
// that produced it (`PropagateControl::assignment(&self) -> Assignment<'_>`):
// it must not compile to store one past the
// `Propagator::check` call that received it. `Propagator::check` takes
// `&mut PropagateControl<'_>` with `&self` on the propagator itself (S11),
// so the escape attempt needs interior mutability to even reach the
// lifetime check, the same way a real misuse would; `RefCell` is used
// rather than `Cell` because the stored value is not `Copy`.
//
// Unlike `propagate_init_escapes_init.rs`, which only gets
// `!Sync` (E0277) and no lifetime mismatch (E0521/E0495), this case
// fails on **both**: a `!Send` error
// (`RefCell<T>` needs `T: Send` for the outer type to stay `Send`, and
// `Assignment` holds a raw pointer) **and** two separate "lifetime may not
// live long enough" errors, because `RefCell<T>` is invariant in `T`. This
// genuinely pins the lifetime, not only the `Send`/`Sync` bound. A
// minimal, non-`Propagator` reproduction that isolates the lifetime alone,
// with no `Send`/`Sync` involved, is
// `fn steal<'a>(control: &
// PropagateControl<'a>) -> Assignment<'a> { control.assignment() }`, E0621
// (see `propagate_init_assignment_escapes.rs`).
//
// Expected: at least the lifetime errors above.
//
//
//

use std::cell::RefCell;

use clingox::propagate::{Assignment, PropagateControl, Propagator};
use clingox::{Control, Part, Result};

struct Escape<'a> {
    slot: RefCell<Option<Assignment<'a>>>,
}

impl Propagator for Escape<'_> {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        *self.slot.borrow_mut() = Some(control.assignment());
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
