// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. A spawned thread needs `'static`.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::application::Application;

fn main() {
    Application::new()
        .main(|ctl, _files| {
            std::thread::spawn(move || {
                ctl.add_base("a.").unwrap();
            });
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
}
