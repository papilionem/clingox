// A configuration entry stays on the thread that made it (DESIGN S12):
// it holds a raw key and a shared borrow of a control that is not `Sync`.
// Expected: E0277, cannot be sent between threads safely.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    let conf = ctl.configuration();
    let root = conf.root().unwrap();
    std::thread::scope(|scope| {
        scope.spawn(move || root.kind());
    });
}
