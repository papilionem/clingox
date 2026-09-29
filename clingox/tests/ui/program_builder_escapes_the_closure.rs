// `ProgramBuilder<'c>` borrows `&'c mut Control` and only exists for the
// duration of the closure `with_program_builder` runs it in: `clingo_program_builder_end`
// runs when the closure returns, whether it succeeds, fails or panics, so the
// reference cannot be kept past that point. This is what guarantees begin and
// end always pair, unlike a caller managing the two calls by hand.
// Expected: E0521, borrowed data escapes outside of closure.
use clingox::Control;

fn main() {
    let mut ctl = Control::new().unwrap();
    let mut kept = None;
    ctl.with_program_builder(|builder| {
        kept = Some(builder);
        Ok(())
    })
    .unwrap();
    drop(kept);
}
