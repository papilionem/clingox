// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. A `SymbolicAtoms` borrowed from the callback's control cannot be stored outside it.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::application::Application;

fn main() {
    let slot = std::cell::RefCell::new(Vec::new());
    Application::new()
        .main(|ctl, _files| {
            slot.borrow_mut().push(ctl.symbolic_atoms()?);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let _ = slot;
}
