// `register_observer` takes `&mut self` (the control) at the same time it
// takes the observer by value; a struct that holds `&mut Control` so it can
// call back into it from a callback already borrows `ctl` before
// `register_observer` tries to borrow it again (DESIGN S5, mirrors
// `backend_used_while_control_grounds.rs`). `GroundProgramObserver`'s own
// `'static` bound would refuse this independently, since
// `Reentrant<'_>` cannot outlive the borrow it holds, but the borrow checker
// already rejects the double borrow first, at the call site itself.
// Expected: E0499, `ctl` borrowed mutably more than once.
use clingox::backend::Atom;
use clingox::observer::GroundProgramObserver;
use clingox::{Control, ProgramLiteral};

struct Reentrant<'a> {
    ctl: &'a mut Control,
}

impl GroundProgramObserver for Reentrant<'_> {
    fn rule(&mut self, _choice: bool, _head: &[Atom], _body: &[ProgramLiteral]) -> clingox::Result<()> {
        self.ctl.solve(&[])?;
        Ok(())
    }
}

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    let obs = Reentrant { ctl: &mut ctl };
    ctl.register_observer(obs, false).unwrap();
}
