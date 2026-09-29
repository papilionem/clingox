// `ProgramBuilder<'c>` borrows `&'c mut Control` (DESIGN S5), so the control
// cannot ground from inside the closure `with_program_builder` passes it a
// builder through: `ctl` is already borrowed mutably for the
// `with_program_builder` call itself, and the closure tries to borrow it again
// to call `ground`. Grounding while the session is open would ground an empty
// program (clingo does not complain).
// Expected: E0499, `ctl` borrowed mutably more than once.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|_builder| {
        ctl.ground(&[Part::base()])?;
        Ok(())
    })
    .unwrap();
}
