// A `parse` closure lives until clingo has finished, so it may borrow what
// the caller of `run` owns but not a local of the registration closure, which
// is gone by then.
// Expected: E0373 closure may outlive the current function.
//

use clingox::application::{Application, OptionSpec};

fn main() {
    let _ = Application::new()
        .register_options(|options| {
            let mut total = 0;
            options.add(OptionSpec::new("Group", "count", "counts"), |value| {
                total += value.len();
                Ok(())
            })
        })
        .run(["--outf=3"]);
}
