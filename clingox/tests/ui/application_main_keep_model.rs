// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. The `&Model` that `SolveHandle::next_model` lends borrows the handle, which borrows the control.
// Expected: E0521 borrowed data escapes outside of closure and E0597 borrowed value does not live long enough.
//

use clingox::application::Application;

fn main() {
    let slot: std::cell::RefCell<Option<&clingox::Model>> = std::cell::RefCell::new(None);
    Application::new()
        .main(|ctl, _files| {
            let mut handle = ctl.solve_yield(&[])?;
            *slot.borrow_mut() = handle.next_model()?;
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let _ = slot;
}
