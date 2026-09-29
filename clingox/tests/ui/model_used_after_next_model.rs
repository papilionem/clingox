// A model dies when the search resumes (DESIGN S6): the model lent by one
// `next_model` call cannot be used after the next call.
// Expected: E0499, `handle` borrowed mutably twice.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("{a}.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let first = handle.next_model().unwrap().unwrap();
    let _second = handle.next_model().unwrap();
    println!("{}", first.number());
}
