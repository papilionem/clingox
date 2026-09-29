// A script's `main` may use the control it receives for anything, inside
// the call: add, ground and solve.

use clingox::ast::Span;
use clingox::script::Script;
use clingox::{Part, Result, ScopedControl};

struct Solver;

impl Script for Solver {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        Ok(())
    }

    fn callable(&self, name: &str) -> Result<bool> {
        Ok(name == "main")
    }

    fn main(&self, control: &mut ScopedControl<'_>) -> Result<()> {
        control.add_base("a.")?;
        control.ground(&[Part::base()])?;
        let (_result, models) = control.solve_all()?;
        let _ = models.len();
        Ok(())
    }
}

fn main() {
    let _ = clingox::script::register("solver", "1", Solver);
}
