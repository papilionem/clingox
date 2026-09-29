// `parse` and `validate_options` are separate closures, so both cannot
// borrow one variable when one of them changes it. The documented pattern is a
// `RefCell` or a `Cell` (see `ui_pass/application_options_refcell.rs`). This
// case pins the limit.
// Expected: E0502 cannot borrow `seen` as immutable because it is also borrowed
// as mutable.

use clingox::application::{Application, OptionSpec};

fn main() {
    let mut seen = Vec::new();
    let _ = Application::new()
        .register_options(|options| {
            options.add(OptionSpec::new("Group", "value", "a value"), |value| {
                seen.push(value.to_owned());
                Ok(())
            })
        })
        .validate_options(|| {
            assert!(seen.len() < 5);
            Ok(())
        })
        .run(["--outf=3"]);
}
