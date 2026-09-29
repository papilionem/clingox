//! `OwnedModel::priorities` (smoke finding B4): the priority levels of the
//! cost, highest first, like `OwnedModel::cost` and like `Model::priorities`
//! on a borrowed model.
//!
//! Oracle: pyclingo 5.8.2, `model.priority` next to `model.cost`, on the
//! fixtures the borrowed `Model::priorities` tests use (`api_model_extras.rs`).

#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, reason = "tests assert on invariants")]

use clingox::{Control, OwnedModel, Part};

fn only_model(program: &str) -> OwnedModel {
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let (_, mut models) = ctl.solve_all().unwrap();
    assert_eq!(models.len(), 1, "the fixture has one model");
    models.remove(0)
}

#[test]
fn priorities_are_listed_highest_priority_first() {
    // cost [1, 1], priority [2, 1]
    let model = only_model("a. b. :~ a. [1@1] :~ b. [1@2]");
    assert_eq!(model.priorities(), [2, 1]);
    assert_eq!(model.cost(), [1, 1]);
}

#[test]
fn priorities_are_the_levels_not_the_costs() {
    // cost [3, 7], priority [5, 1]
    let model = only_model("a. b. :~ a. [3@5] :~ b. [7@1]");
    assert_eq!(model.cost(), [3, 7]);
    assert_eq!(model.priorities(), [5, 1]);
}

#[test]
fn priorities_are_empty_without_optimisation_statements() {
    let model = only_model("a.");
    assert!(model.priorities().is_empty());
    assert!(model.cost().is_empty());
}

#[test]
fn the_owned_and_the_borrowed_model_agree() {
    let program = "{a;b;c}. :~ a. [2@4] :~ b. [1@2] :~ c. [5@9]";
    let mut ctl = Control::new().unwrap();
    ctl.add_base(program).unwrap();
    ctl.ground(&[Part::base()]).unwrap();
    let mut handle = ctl.solve_yield(&[]).unwrap();
    let model = handle.next_model().unwrap().expect("a model");
    let borrowed = model.priorities().unwrap();
    assert_eq!(borrowed, [9, 4, 2]);
    drop(handle);

    let owned = {
        let mut ctl = Control::new().unwrap();
        ctl.add_base(program).unwrap();
        ctl.ground(&[Part::base()]).unwrap();
        let (_, models) = ctl.solve_all().unwrap();
        models.into_iter().next().unwrap()
    };
    assert_eq!(owned.priorities(), [9, 4, 2]);
}

#[test]
fn a_cloned_model_keeps_its_priorities() {
    let model = only_model("a. :~ a. [1@3]");
    let copy = model.clone();
    assert_eq!(copy.priorities(), [3]);
    assert_eq!(copy, model);
}
