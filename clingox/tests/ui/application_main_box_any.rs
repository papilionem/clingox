// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. A `Box<dyn Any>` needs `'static`.
// Expected: error: lifetime may not live long enough (no error code).
//

use clingox::Control;
use clingox::application::Application;

fn main() {
    let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
    Application::new()
        .main(|ctl, _files| {
            keep.push(Box::new(std::mem::replace(ctl, Control::new()?)));
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
