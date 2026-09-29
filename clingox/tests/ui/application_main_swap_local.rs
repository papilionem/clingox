// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. `mem::swap` needs a second `ScopedControl<'r>`; an owned control is `Control` and the brand is invariant.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    Application::new()
        .main(|ctl, _files| {
            let mut owned = Control::new()?;
            std::mem::swap(ctl, &mut owned);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
