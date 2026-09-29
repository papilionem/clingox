//! `ModelKind` is in the prelude (smoke documentation gap): a caller who
//! writes a model printer or reads `Model::kind` needs no second import.

#![forbid(unsafe_code)]

use clingox::prelude::*;

#[test]
fn model_kind_comes_with_the_prelude() {
    let kind: ModelKind = ModelKind::StableModel;
    assert_eq!(kind, ModelKind::StableModel);
    assert_ne!(kind, ModelKind::BraveConsequences);
    assert_ne!(kind, ModelKind::CautiousConsequences);
}
