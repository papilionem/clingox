// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. As `replace_stash`, with the stash type inferred.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    let mut stash = None;
    Application::new()
        .main(|ctl, _files| {
            stash = Some(std::mem::replace(ctl, Control::new()?));
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let _ = stash;
}
