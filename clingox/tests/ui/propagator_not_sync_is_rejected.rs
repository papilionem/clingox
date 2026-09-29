// `Propagator: Send + Sync` (DESIGN S11): every method takes `&self`, and
// clasp can call `propagate` for two different solver threads on the same
// `&self` at once, so a propagator that is `Send` but not `Sync` must be
// rejected. `Cell<u32>` is exactly that: `Send` because `u32: Send`, never
// `Sync` (DESIGN 8.2's compile-fail table names this case: "a propagator
// that is not `Sync`"; weakening the bound to `Propagator: Send` would make
// this test pass, which is why the bound is `Send + Sync`).
// Expected: E0277, `Cell<u32>` cannot be shared between threads safely.
use std::cell::Cell;

use clingox::propagate::Propagator;
use clingox::{Control, Part};

struct Counter {
    seen: Cell<u32>,
}

impl Propagator for Counter {}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    ctl.register_propagator(Counter { seen: Cell::new(0) })
        .unwrap();
}
