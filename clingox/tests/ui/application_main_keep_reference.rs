// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. The `&mut ScopedControl<'r>` itself cannot be kept.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    let mut keep: Option<&mut Control> = None;
    Application::new()
        .main(|ctl, _files| {
            keep = Some(ctl);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let _ = keep;
}
