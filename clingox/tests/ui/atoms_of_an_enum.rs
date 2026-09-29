// An enum spans several predicates, so it does not implement `Predicate` and
// cannot be read with `atoms::<T>()`. Expected: the unsatisfied bound
// `Shape: Predicate` at `atoms::<Shape>`.
#![forbid(unsafe_code)]

use clingox::{FromSymbol, OwnedModel};

#[derive(FromSymbol)]
enum Shape {
    Point,
    Circle(i32),
}

fn read(model: &OwnedModel) {
    let _ = model.atoms::<Shape>();
}

fn main() {
    let _: fn(&OwnedModel) = read;
}
