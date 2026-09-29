// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. A `thread_local!` cannot receive the control.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

thread_local! {
    static STASH: std::cell::RefCell<Option<Control>> = const { std::cell::RefCell::new(None) };
}

fn main() {
    Application::new()
        .main(|ctl, _files| {
            STASH.with(|s| *s.borrow_mut() = Some(std::mem::replace(ctl, Control::new().unwrap())));
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
