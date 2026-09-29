// The backend and the program builder are separate sessions, and clingo
// tolerates one inside the other at the C level; clingox does not: opening a
// backend from inside the closure needs the control mutably again while the
// program builder's call still holds it.
// Expected: E0499, `ctl` borrowed mutably more than once.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|_builder| {
        ctl.with_backend(|_backend| Ok(()))?;
        Ok(())
    })
    .unwrap();
}
