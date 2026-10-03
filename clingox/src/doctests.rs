//! The Rust code blocks of the README and the guide, run as doctests
//! (TESTING 8). This module is compiled only by `cargo test --doc`, so the
//! published crate does not need the Markdown files. Every guide chapter
//! with a Rust block has an entry here; `tests/user_docs.rs` checks that.

#[doc = include_str!("../../README.md")]
mod readme {}

#[doc = include_str!("../README.md")]
mod crate_readme {}

#[doc = include_str!("../../guide/src/introduction.md")]
mod introduction {}

#[doc = include_str!("../../guide/src/getting-started/installation.md")]
mod installation {}

#[doc = include_str!("../../guide/src/getting-started/first-program.md")]
mod first_program {}

#[doc = include_str!("../../guide/src/tutorial/facts-and-rules.md")]
mod facts_and_rules {}

#[doc = include_str!("../../guide/src/tutorial/typed-results.md")]
mod typed_results {}

#[doc = include_str!("../../guide/src/tutorial/solving-step-by-step.md")]
mod solving_step_by_step {}

#[doc = include_str!("../../guide/src/tutorial/optimisation.md")]
mod optimisation {}

#[doc = include_str!("../../guide/src/concepts/two-layers.md")]
mod two_layers {}

#[doc = include_str!("../../guide/src/concepts/errors-and-panics.md")]
mod errors_and_panics {}

#[doc = include_str!("../../guide/src/concepts/theory-atoms.md")]
mod theory_atoms {}

#[doc = include_str!("../../guide/src/concepts/backend.md")]
mod backend_chapter {}

#[doc = include_str!("../../guide/src/concepts/observer.md")]
mod observer_chapter {}

#[doc = include_str!("../../guide/src/concepts/syntax-trees.md")]
mod syntax_trees {}

#[doc = include_str!("../../guide/src/concepts/solve-events.md")]
mod solve_events_chapter {}

#[doc = include_str!("../../guide/src/how-to/write-a-propagator.md")]
mod write_a_propagator {}

#[doc = include_str!("../../guide/src/how-to/clingo-applications.md")]
mod clingo_applications {}

#[doc = include_str!("../../guide/src/how-to/custom-scripting-languages.md")]
mod custom_scripting_languages {}

#[doc = include_str!("../../guide/src/how-to/configuration.md")]
mod configuration {}

#[doc = include_str!("../../guide/src/reference/coming-from-pyclingo.md")]
mod coming_from_pyclingo {}

#[doc = include_str!("../../guide/src/reference/coming-from-clingo-crate.md")]
mod coming_from_clingo_crate {}

#[doc = include_str!("../../guide/src/reference/feature-flags.md")]
mod feature_flags {}

#[doc = include_str!("../../guide/src/reference/error-kinds.md")]
mod error_kinds {}

#[doc = include_str!("../../guide/src/how-to/time-budget.md")]
mod time_budget {}

#[doc = include_str!("../../guide/src/how-to/test-your-rules.md")]
mod test_your_rules {}

#[doc = include_str!("../../guide/src/how-to/browser.md")]
mod browser {}

#[doc = include_str!("../../guide/src/how-to/android.md")]
mod android {}

#[doc = include_str!("../../guide/src/how-to/server.md")]
mod server {}

#[doc = include_str!("../../guide/src/how-to/rewrite-programs.md")]
mod rewrite_programs {}

#[doc = include_str!("../../guide/src/concepts/safety-and-threads.md")]
mod safety_and_threads {}

#[doc = include_str!("../../guide/src/reference/api-reference.md")]
mod api_reference {}
