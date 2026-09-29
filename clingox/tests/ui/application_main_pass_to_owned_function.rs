// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. A borrowed control is not a `Control` (the owned kind).
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn takes_owned(_ctl: &mut Control) {}

fn main() {
    Application::new()
        .main(|ctl, _files| {
            takes_owned(ctl);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
