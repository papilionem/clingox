// A `FunctionCall` lends clingo's arguments for the duration of one call, so it
// cannot be kept after the callback returns.
// Expected: E0521, borrowed data escapes outside of the closure.
use clingox::{Control, FunctionCall, Part, Symbol};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("p(@f(1)).").unwrap();
    let mut kept: Vec<&[Symbol]> = Vec::new();
    ctl.ground_with(&[Part::base()], |call: &mut FunctionCall<'_>| {
        let pushed = call.push(Symbol::number(1));
        kept.push(call.args());
        pushed
    })
    .unwrap();
    drop(kept);
}
