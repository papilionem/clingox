// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. Even a replacement that is dropped inside the callback does not compile: the only `ScopedControl<'r>` is the trampoline's own.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    Application::new()
        .main(|ctl, _files| {
            let old = std::mem::replace(ctl, Control::new()?);
            drop(old);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
