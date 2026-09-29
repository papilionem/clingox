// The pattern the documentation gives for state shared between `parse`,
// `validate_options` and `main`: a `RefCell` (or `Cell`) outside, a `Flag` for
// booleans.

use std::cell::RefCell;

use clingox::application::{Application, Flag, OptionSpec};

fn main() -> clingox::Result<()> {
    let seen = RefCell::new(Vec::<String>::new());
    let flag = Flag::new(false);
    let code = Application::new()
        .register_options(|options| {
            options.add(
                OptionSpec::new("Group", "value", "a value")
                    .multi()
                    .argument("<v>"),
                |value| {
                    seen.borrow_mut().push(value.to_owned());
                    Ok(())
                },
            )?;
            options.add_flag("Group", "switch", "a switch", &flag)
        })
        .validate_options(|| {
            assert!(seen.borrow().len() < 5);
            Ok(())
        })
        .main(|_ctl, _files| {
            assert!(!flag.get());
            Ok(())
        })
        .run(["--outf=3"])?;
    assert_eq!(code, 0);
    Ok(())
}
