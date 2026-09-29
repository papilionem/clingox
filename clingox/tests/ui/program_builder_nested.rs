// A second `with_program_builder` inside the closure needs the control
// mutably again while the outer call still holds it. Two sessions at once
// would leave the outer one's statements to be ended by the inner one.
// Expected: E0499, `ctl` borrowed mutably more than once.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|_builder| {
        ctl.with_program_builder(|_inner| Ok(()))?;
        Ok(())
    })
    .unwrap();
}
