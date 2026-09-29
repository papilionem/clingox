// The logger may run on any thread (DESIGN S10), so it must be `Send`. An `Rc`
// captured by the logger is not.
// Expected: E0277, `Rc<Cell<u32>>` cannot be sent between threads safely.
use std::cell::Cell;
use std::rc::Rc;

use clingox::Control;

fn main() {
    let count = Rc::new(Cell::new(0_u32));
    let counter = Rc::clone(&count);
    let _ctl = Control::builder()
        .logger(move |_, _| counter.set(counter.get() + 1))
        .build();
}
