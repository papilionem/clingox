// A lent model stays on the thread that reads it, even though the control may
// move (DESIGN S10, S12): `Model` is not `Sync`, so `&Model` is not `Send`.
use std::ops::ControlFlow;

fn main() {
    let mut ctl = clingox::Control::new().unwrap();
    ctl.for_each_model(&[], |model| {
        std::thread::scope(|scope| {
            scope.spawn(|| model.number());
        });
        Ok(ControlFlow::Continue(()))
    })
    .unwrap();
}
