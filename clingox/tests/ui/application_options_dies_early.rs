// The callbacks may borrow for the application's lifetime `'a`, so the
// application cannot outlive what they borrow.
// Expected: E0597 `local` does not live long enough.
//

use std::cell::Cell;

use clingox::application::{Application, OptionSpec};

fn main() {
    let application;
    {
        let local = Cell::new(0);
        application = Application::new().register_options(|options| {
            options.add(OptionSpec::new("Group", "value", "a value"), |_| {
                local.set(1);
                Ok(())
            })
        });
    }
    let _ = application.run(["--outf=3"]);
}
