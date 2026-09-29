// The control that `Application::main` lends is `&mut ScopedControl<'r>` for a
// brand `'r` that is invariant and universally quantified in the callback's
// bound, so nothing that would let it (or anything borrowed from it) outlive the
// callback may compile. The `files` slice borrows clingo's arrays for the call only.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::application::Application;

fn main() {
    let mut keep: Option<&[&str]> = None;
    Application::new()
        .main(|_ctl, files| {
            keep = Some(files);
            Ok(())
        })
        .run(["--outf=3"])
        .unwrap();
    let _ = keep;
}
