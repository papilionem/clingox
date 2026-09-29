// A solve handle borrows its control and cannot outlive it (DESIGN S5).
// Expected: E0597, `ctl` does not live long enough.
use clingox::{Control, SolveHandle};

fn main() {
    let handle: SolveHandle<'_>;
    {
        let mut ctl = Control::new().unwrap();
        handle = ctl.solve_yield(&[]).unwrap();
    }
    drop(handle);
}
