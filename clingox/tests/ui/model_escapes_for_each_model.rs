// The model passed to the `for_each_model` closure is only lent for that call;
// keeping it needs `snapshot()` (DESIGN S6).
// Expected: E0521, borrowed data escapes the closure.
use std::ops::ControlFlow;

use clingox::{Control, Model, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{a}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut kept: Vec<&Model> = Vec::new();
    let _result = ctl.for_each_model(&[], |model| {
        kept.push(model);
        Ok(ControlFlow::Continue(()))
    });
    println!("{}", kept.len());
}
