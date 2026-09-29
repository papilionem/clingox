// An `Atom` (or a value built from one, such as a `ProgramLiteral`) is a
// plain owned `Copy` value, not a reference into the backend: computing it
// inside `with_backend`'s closure and using it afterward, once the backend
// is closed, must compile. This is the usable half of the same rule
// `backend_escapes_the_closure.rs` pins the other side of: the `Backend<'_>`
// handle itself cannot leave the closure, but what it produces can.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("").unwrap();
    let atom = ctl
        .with_backend(|backend| backend.add_atom(None))
        .unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let literal = atom.pos();
    let _ = ctl.solve(&[literal.into()]);
}
