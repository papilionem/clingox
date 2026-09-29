// `Trail<'_>` borrows the same `&'a clingo_assignment_t` as the
// `Assignment<'_>` it comes from (`Assignment::trail`), so it cannot outlive the callback that
// produced its `Assignment` either, one step removed. Mirrors
// `assignment_escapes_check.rs` exactly, for `Trail` instead of
// `Assignment` directly (see that file's own comment for why this shape,
// unlike `propagate_init_escapes_init.rs`, genuinely pins a lifetime
// mismatch and not only a `Send`/`Sync` bound).
//
// Expected: a compile failure, though not necessarily the identical
// diagnostic shape as `assignment_escapes_check.rs`: `control.assignment()`
// produces a temporary `Assignment<'_>` here, and `.trail()` borrows from
// that temporary (one level of indirection deeper than the direct
// `Assignment` case), so `E0716` ("temporary value dropped while still
// borrowed") is at least as likely as the lifetime-mismatch/`!Send` pair
// `assignment_escapes_check.rs` gets; both shapes reject the same misuse.
//
//
//
//
//
//

use std::cell::RefCell;

use clingox::propagate::{PropagateControl, Propagator, Trail};
use clingox::{Control, Part, Result};

struct Escape<'a> {
    slot: RefCell<Option<Trail<'a>>>,
}

impl Propagator for Escape<'_> {
    fn check(&self, control: &mut PropagateControl<'_>) -> Result<()> {
        *self.slot.borrow_mut() = Some(control.assignment().trail());
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
