// A stronger compile-fail case for the same claim
// `propagate_init_escapes_init.rs` makes (an object borrowed from
// `Propagator::init`'s own callback cannot outlive that call): that test
// only ever produces `E0277` ("`RefCell<...>` cannot be shared between
// threads safely"), never the lifetime mismatch (`E0521`/`E0495`) it
// expects, because any interior-mutable escape attempt
// trips `Propagator: Send + Sync` before the lifetime is even checked
// (`assignment_escapes_check.rs` finds the identical shape *for
// `Assignment`* fails on both `!Send` *and* two genuine "lifetime may not
// live long enough" errors).
//
// This file isolates the lifetime claim alone, with no `Propagator`,
// `Send`/`Sync`, or interior mutability involved at all, by exploiting
// `PropagateInit::assignment(&self) -> Assignment<'_>` directly (the one
// `PropagateInit` accessor whose return type borrows from `&self`, exactly
// like `PropagateControl::assignment` does): a free function that asks for
// `Assignment<'a>` as long as the *whole* `PropagateInit<'a>`'s own type
// parameter, rather than only as long as this call's own `&self` reborrow,
// cannot compile, because `assignment`'s own signature only promises the
// shorter, elided bound. Reproduced directly against a throwaway
// scratch build: `E0621` ("explicit lifetime required in the type of
// `init`"), no other error, confirming this is a clean, `Send`/`Sync`-free
// pin of the lifetime this crate's safety design relies on.
//
// This complements, and does not replace, `propagate_init_escapes_init.rs`:
// that file is still the more realistic misuse shape (a real
// `Propagator` trying to stash the value in a field), this one is the more
// precise one (isolates exactly which bound is load-bearing). Both stay.
//
// Expected: E0621.
//
//
//
//
//
//
//

use clingox::propagate::{Assignment, PropagateInit};

fn steal<'a>(init: &PropagateInit<'a>) -> Assignment<'a> {
    init.assignment()
}

fn main() {}
