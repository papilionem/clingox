// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. A `static Mutex<Option<Control>>` cannot receive the control.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::Control;
use clingox::application::Application;

static STASH: std::sync::Mutex<Option<Control>> = std::sync::Mutex::new(None);

fn main() {
    Application::new()
        .main(|ctl, _files| {
            *STASH.lock().unwrap() = Some(std::mem::replace(ctl, Control::new()?));
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
