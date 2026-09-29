// The same borrow keeps `Control::add` out of the session: clingo would accept
// a text add while the program builder is open and silently drop part of the
// session's statements.
// Expected: E0499, `ctl` borrowed mutably more than once.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.with_program_builder(|_builder| {
        ctl.add_base("a.")?;
        Ok(())
    })
    .unwrap();
}
