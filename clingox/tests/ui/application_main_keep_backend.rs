// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. The `&mut Backend` of `with_backend` cannot leave its closure, in any callback.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::application::Application;

fn main() {
    let mut keep: Vec<&mut clingox::backend::Backend<'static>> = Vec::new();
    Application::new()
        .main(|ctl, _files| {
            ctl.with_backend(|backend| {
                keep.push(backend);
                Ok(())
            })?;
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let _ = keep;
}
