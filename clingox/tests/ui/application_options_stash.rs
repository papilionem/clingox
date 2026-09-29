// The `Options` handle is lent to the registration closure only, so it
// cannot be kept: it borrows clingo's option context, which is meaningful only
// while clingo calls the closure.
// Expected: E0521 borrowed data escapes outside of closure.
//

use clingox::application::Application;

fn main() {
    let mut stash = None;
    let _ = Application::new()
        .register_options(|options| {
            stash = Some(options);
            Ok(())
        })
        .run(["--outf=3"]);
}
