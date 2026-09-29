// The default printer is lent for one call and cannot be kept. The container is made `Sync` by a local `unsafe impl`, so the lifetime, not `Send`, is what is tested (without it the error is E0277).
// Expected: E0521 (borrowed data escapes outside of closure).

#![allow(unsafe_code)]

use std::cell::Cell;

use clingox::application::{Application, DefaultPrinter};

struct Slot<T>(Cell<Option<T>>);
// SAFETY: never used from two threads; the case only has to type-check.
unsafe impl<T> Sync for Slot<T> {}
impl<T> Slot<T> {
    fn set(&self, value: T) {
        self.0.set(Some(value));
    }
}

fn main() {
    let keep: Slot<&mut DefaultPrinter<'static>> = Slot(Cell::new(None));
    let _ = Application::new().print_model(|_model, printer| {
        keep.set(printer);
        Ok(())
    });
}
