// `with_backend`'s `Backend<'_>` only exists for the duration of the
// closure: `clingo_backend_end` runs when the closure returns, whether it
// succeeds, fails or panics, so the reference
// cannot be kept past that point. This is what guarantees begin/end always
// pair, unlike a caller managing `begin`/`end` calls by hand.
// Expected: E0521, borrowed data escapes outside of closure.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    ctl.add_base("a.").unwrap();
    let mut kept = None;
    ctl.with_backend(|backend| {
        kept = Some(backend);
        Ok(())
    })
    .unwrap();
    drop(kept);
}
