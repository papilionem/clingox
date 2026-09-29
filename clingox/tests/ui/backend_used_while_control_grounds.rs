// `Backend<'c>` borrows `&'c mut Control` (DESIGN S5), so the control cannot
// ground or solve from inside the closure `with_backend` passes it a backend
// through: `ctl` is already borrowed mutably for the `with_backend` call
// itself, and the closure tries to borrow it again to call `ground`.
// Expected: E0499, `ctl` borrowed mutably more than once.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.with_backend(|_backend| {
        ctl.ground(&[Part::base()])?;
        Ok(())
    })
    .unwrap();
}
