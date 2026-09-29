// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. `mem::forget` of the replaced control would leak a borrowed control with open records.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    Application::new()
        .main(|ctl, _files| {
            std::mem::forget(std::mem::replace(ctl, Control::new()?));
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
