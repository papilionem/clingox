// A model cannot outlive the handle that lent it: closing the handle ends the
// search the model belongs to (DESIGN S6, S7).
// Expected: E0505, `handle` moved while borrowed.
use clingox::{Control, Part};

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().unwrap();
    let _result = handle.close();
    println!("{}", model.number());
}
