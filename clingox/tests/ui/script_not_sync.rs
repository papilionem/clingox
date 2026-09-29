// A script is shared by every control in the process and called from
// whichever thread grounds, so `Script: Send + Sync`. A script holding an `Rc`
// is neither. Expected: E0277 `Rc<()>` cannot be sent/shared between threads.
//

use std::rc::Rc;

use clingox::ast::Span;
use clingox::script::Script;
use clingox::Result;

struct Shared(Rc<()>);

impl Script for Shared {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        let _ = &self.0;
        Ok(())
    }
}

fn main() {
    let _ = clingox::script::register("shared", "1", Shared(Rc::new(())));
}
