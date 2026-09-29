// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. Swapping with a control declared outside would move the borrowed one out.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    let mut outer: Control = Control::new().unwrap();
    Application::new()
        .main(|ctl, _files| {
            std::mem::swap(ctl, &mut outer);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
